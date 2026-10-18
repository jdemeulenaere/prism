//! The build cache (the `build-cache` spec; D3 to D8 of `abi-cache`, D5, D8 and D11 of
//! `instance-cache`).
//!
//! A per-user, content-addressed store of entries of four kinds. A **check** entry records that
//! a file passes Phase 2 against its direct imports' ABI hashes, with the file's summary
//! ([`FileSummary`]). A **record** entry holds a generic record, addressed by its
//! Monomorphization Hash. A **unit** entry holds the compiled code of a file's unit and an
//! **instance** entry that of an instance, both as relocatable artifacts keyed by everything
//! their code depends on and the target. Entries live under
//! `<directory>/v<format>/<first two hex digits of the key>/<key>`, are written atomically,
//! carry a text header and a checksum around a binary body, and any problem reading or writing
//! one is a miss: the cache never fails a check or a run.

use prism_abi::{ContentHash, DeterministicHasher};
use prism_mir::{FileSummary, GenericRecord};
use std::cell::Cell;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The version of the entry format and key derivation. A new version uses a new `v<N>`
/// directory, so compilers of different versions never read each other's entries.
pub const FORMAT_VERSION: u64 = 2;

/// The environment variable naming the cache directory, or `off` to disable the cache.
pub const CACHE_VARIABLE: &str = "PRISM_CACHE";

const MAGIC: &str = "prism-cache";

/// The tag file marking the directory as a cache, for backup tools
/// (<https://bford.info/cachedir/>).
const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
# This file is a cache directory tag created by the Prism compiler (prism).\n\
# For information about cache directory tags see https://bford.info/cachedir/\n";

/// The kinds of entries the cache holds (D11 of `instance-cache`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A file's verdict and summary.
    Check,
    /// A generic record, by its Monomorphization Hash.
    Record,
    /// The compiled code of a file's unit.
    Unit,
    /// The compiled code of an instance's unit.
    Instance,
    /// The hash of the compiler executable, keyed by its path and file metadata, so that a
    /// process need not read and hash its whole executable to know its identity.
    Executable,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            EntryKind::Check => "check",
            EntryKind::Record => "record",
            EntryKind::Unit => "unit",
            EntryKind::Instance => "instance",
            EntryKind::Executable => "executable",
        }
    }
}

/// An open build cache: a directory and the identity of the compiler using it.
#[derive(Debug)]
pub struct BuildCache {
    directory: PathBuf,
    compiler: ContentHash,
    tagged: Cell<bool>,
    /// The number of entries this cache wrote, for `--explain-cache` tests and benchmarks.
    writes: Cell<u64>,
}

impl BuildCache {
    /// The cache in `directory`, used by the compiler whose identity is `compiler`. Nothing is
    /// created until the first write.
    pub fn open(directory: impl Into<PathBuf>, compiler: ContentHash) -> Self {
        Self {
            directory: directory.into(),
            compiler,
            tagged: Cell::new(false),
            writes: Cell::new(0),
        }
    }

    /// The cache the environment selects for the running compiler: `PRISM_CACHE`, or the
    /// platform's per-user cache directory. `None` when the cache is `off`, when no cache
    /// directory is known, or when the compiler executable cannot be identified.
    pub fn from_environment() -> Option<Self> {
        let variable = |name: &str| std::env::var(name).ok();
        let directory = cache_directory(std::env::consts::OS, &variable)?;
        let compiler = compiler_identity_in(&directory)?;
        Some(Self::open(directory, compiler))
    }

    /// The cache's directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The number of entries written through this handle.
    pub fn writes(&self) -> u64 {
        self.writes.get()
    }

    /// The key of a file's check entry: the format version, the compiler, the file's module
    /// path and exact source text, and its direct imports' ABI hashes in import order (D5 of
    /// `abi-cache`, D3 of `instance-cache`).
    pub fn check_key(
        &self,
        module: &str,
        source: &str,
        import_hashes: &[ContentHash],
    ) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Check);
        hasher.write_str(module);
        hasher.write_hash(&ContentHash::of(source.as_bytes()));
        hasher.write_u64(import_hashes.len() as u64);
        for hash in import_hashes {
            hasher.write_hash(hash);
        }
        hasher.finish()
    }

    /// The key of the record entry of the generic record whose Monomorphization Hash is
    /// `record_hash` (D5 of `instance-cache`).
    pub fn record_key(&self, record_hash: &ContentHash) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Record);
        hasher.write_hash(record_hash);
        hasher.finish()
    }

    /// The key of the code of a file's unit: its check key, which covers everything a file's
    /// code depends on, and the target identity (D8 of `instance-cache`).
    pub fn unit_key(&self, check_key: &ContentHash, target: &str) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Unit);
        hasher.write_str(target);
        hasher.write_hash(check_key);
        hasher.finish()
    }

    /// The key of the code of the instance `symbol`: its generic's Monomorphization Hash, the
    /// hash of the definitions its type arguments reach, and the target identity (D6 and D8 of
    /// `instance-cache`).
    pub fn instance_key(
        &self,
        symbol: &str,
        record_hash: &ContentHash,
        definitions_hash: &ContentHash,
        target: &str,
    ) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Instance);
        hasher.write_str(target);
        hasher.write_str(symbol);
        hasher.write_hash(record_hash);
        hasher.write_hash(definitions_hash);
        hasher.finish()
    }

    fn key_hasher(&self, kind: EntryKind) -> DeterministicHasher {
        let mut hasher = DeterministicHasher::new();
        hasher.write_str(MAGIC);
        hasher.write_str(kind.as_str());
        hasher.write_u64(FORMAT_VERSION);
        hasher.write_hash(&self.compiler);
        hasher
    }

    /// The summary in the check entry `key`, or `None` on a miss.
    pub fn read_check(&self, key: &ContentHash) -> Option<FileSummary> {
        FileSummary::decode(&self.read_entry(EntryKind::Check, key)?)
    }

    /// Records the check entry `key` with the file's summary.
    pub fn write_check(&self, key: &ContentHash, summary: &FileSummary) {
        self.write_entry(EntryKind::Check, key, &summary.encode());
    }

    /// The generic record whose Monomorphization Hash is `record_hash`, or `None` on a miss or
    /// when the entry holds another record.
    pub fn read_record(&self, record_hash: &ContentHash) -> Option<GenericRecord> {
        let bytes = self.read_entry(EntryKind::Record, &self.record_key(record_hash))?;
        if ContentHash::of(&bytes) != *record_hash {
            return None;
        }
        GenericRecord::decode(&bytes)
    }

    /// Records `encoding`, the encoding of the record whose Monomorphization Hash is
    /// `record_hash`.
    pub fn write_record(&self, record_hash: &ContentHash, encoding: &[u8]) {
        self.write_entry(EntryKind::Record, &self.record_key(record_hash), encoding);
    }

    /// The bytes of the code entry `key` of `kind` ([`EntryKind::Unit`] or
    /// [`EntryKind::Instance`]), or `None` on a miss.
    pub fn read_code(&self, kind: EntryKind, key: &ContentHash) -> Option<Vec<u8>> {
        self.read_entry(kind, key)
    }

    /// Records the code entry `key` of `kind` holding `artifact`'s bytes.
    pub fn write_code(&self, kind: EntryKind, key: &ContentHash, artifact: &[u8]) {
        self.write_entry(kind, key, artifact);
    }

    /// Whether the code entry `key` of `kind` exists with a valid header and the length its
    /// header announces, read without its body.
    pub fn has_code(&self, kind: EntryKind, key: &ContentHash) -> bool {
        self.has_entry(kind, key)
    }

    /// The path of the entry `key`.
    pub fn entry_path(&self, key: &ContentHash) -> PathBuf {
        let hex = key.to_hex();
        self.directory
            .join(format!("v{}", FORMAT_VERSION))
            .join(&hex[..2])
            .join(hex)
    }

    /// The body of the entry `key` of `kind`, or `None` when it is missing, unreadable or
    /// invalid (D7 of `abi-cache`).
    fn read_entry(&self, kind: EntryKind, key: &ContentHash) -> Option<Vec<u8>> {
        let bytes = fs::read(self.entry_path(key)).ok()?;
        parse_entry(&bytes, kind, key).map(<[u8]>::to_vec)
    }

    /// Whether the entry `key` of `kind` has a valid header and the size it announces.
    fn has_entry(&self, kind: EntryKind, key: &ContentHash) -> bool {
        let Ok(mut file) = fs::File::open(self.entry_path(key)) else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        let mut start = vec![0u8; 512];
        let mut read = 0;
        while read < start.len() {
            match file.read(&mut start[read..]) {
                Ok(0) => break,
                Ok(count) => read += count,
                Err(_) => return false,
            }
        }
        start.truncate(read);
        let Some((header_length, body_length)) = parse_header(&start, kind, key) else {
            return false;
        };
        metadata.len() == (header_length + body_length + CHECKSUM_LINE_LENGTH) as u64
    }

    /// Writes the entry `key` of `kind` to a temporary file in its directory and renames it into
    /// place (D8 of `abi-cache`). Every failure is ignored: the entry is then simply not
    /// recorded (D7).
    fn write_entry(&self, kind: EntryKind, key: &ContentHash, body: &[u8]) {
        let path = self.entry_path(key);
        let Some(parent) = path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        if !self.tagged.get() {
            let tag = self.directory.join("CACHEDIR.TAG");
            if !tag.exists() {
                write_atomically(&tag, CACHEDIR_TAG.as_bytes());
            }
            self.tagged.set(true);
        }
        if write_atomically(&path, &render_entry(kind, key, body)) {
            self.writes.set(self.writes.get() + 1);
        }
    }
}

/// The length of the line `\nchecksum <64 hex digits>\n` that ends every entry.
const CHECKSUM_LINE_LENGTH: usize = "\nchecksum \n".len() + 64;

/// The bytes of an entry: a text header naming the format version, kind, key and the body's
/// length, the body, and a line with a checksum of everything before it (D11 of
/// `instance-cache`).
fn render_entry(kind: EntryKind, key: &ContentHash, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "{} {}\nkind {}\nkey {}\nlength {}\n",
        MAGIC,
        FORMAT_VERSION,
        kind.as_str(),
        key.to_hex(),
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    let checksum = ContentHash::of(&bytes);
    bytes.extend_from_slice(format!("\nchecksum {}\n", checksum.to_hex()).as_bytes());
    bytes
}

/// The length of the header at the start of `bytes` and the body length it announces, if it is
/// the header of an entry of `kind` for `key`.
fn parse_header(bytes: &[u8], kind: EntryKind, key: &ContentHash) -> Option<(usize, usize)> {
    let expected = format!(
        "{} {}\nkind {}\nkey {}\nlength ",
        MAGIC,
        FORMAT_VERSION,
        kind.as_str(),
        key.to_hex()
    );
    let rest = bytes.strip_prefix(expected.as_bytes())?;
    let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 || digits > 19 || rest.get(digits) != Some(&b'\n') {
        return None;
    }
    let length: usize = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    Some((expected.len() + digits + 1, length))
}

/// The body of `bytes` if they are a valid entry of `kind` for `key`.
fn parse_entry<'a>(bytes: &'a [u8], kind: EntryKind, key: &ContentHash) -> Option<&'a [u8]> {
    let (header_length, body_length) = parse_header(bytes, kind, key)?;
    let checked_length = header_length.checked_add(body_length)?;
    if bytes.len() != checked_length.checked_add(CHECKSUM_LINE_LENGTH)? {
        return None;
    }
    let (checked, line) = bytes.split_at(checked_length);
    let hex = line.strip_prefix(b"\nchecksum ")?.strip_suffix(b"\n")?;
    let checksum = ContentHash::from_hex(std::str::from_utf8(hex).ok()?)?;
    if ContentHash::of(checked) != checksum {
        return None;
    }
    Some(&checked[header_length..])
}

/// Writes `bytes` to `path` through a temporary file in the same directory and a rename, so
/// that readers see no file or a complete one. Returns whether the entry was written; failures
/// are otherwise ignored.
fn write_atomically(path: &Path, bytes: &[u8]) -> bool {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let temporary = path.with_file_name(format!(
        "{}.tmp-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::write(&temporary, bytes).is_err() {
        fs::remove_file(&temporary).ok();
        return false;
    }
    if fs::rename(&temporary, path).is_err() {
        fs::remove_file(&temporary).ok();
        return false;
    }
    true
}

/// The cache directory selected by the environment `variable` on the operating system `os`
/// (`std::env::consts::OS`): `PRISM_CACHE` unless it is empty, the platform's per-user cache
/// directory followed by `prism` otherwise, and `None` when `PRISM_CACHE` is `off` or no
/// per-user cache directory is known (D3).
pub fn cache_directory(os: &str, variable: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |name: &str| variable(name).filter(|value| !value.is_empty());
    match non_empty(CACHE_VARIABLE) {
        Some(value) if value == "off" => return None,
        Some(value) => return Some(PathBuf::from(value)),
        None => {}
    }
    let base = match os {
        "macos" | "ios" => PathBuf::from(non_empty("HOME")?).join("Library/Caches"),
        "windows" => PathBuf::from(non_empty("LOCALAPPDATA")?),
        _ => match non_empty("XDG_CACHE_HOME").filter(|dir| Path::new(dir).is_absolute()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(non_empty("HOME")?).join(".cache"),
        },
    };
    Some(base.join("prism"))
}

/// The identity of the running compiler: its version and a hash of its executable, computed
/// once per process (D5). `None` when the executable cannot be read.
pub fn compiler_identity() -> Option<ContentHash> {
    static IDENTITY: OnceLock<Option<ContentHash>> = OnceLock::new();
    *IDENTITY.get_or_init(|| {
        let executable = std::env::current_exe().ok()?;
        Some(identity_of(
            env!("CARGO_PKG_VERSION"),
            &ContentHash::of(&fs::read(executable).ok()?),
        ))
    })
}

/// [`compiler_identity`], with the hash of the executable taken from the cache in `directory`
/// when an `executable` entry records it for the executable's current path and file metadata,
/// and recorded there otherwise (D16 of `instance-cache`). Reading and hashing the whole
/// executable would otherwise be most of a warm run of a small program.
pub fn compiler_identity_in(directory: &Path) -> Option<ContentHash> {
    static IDENTITY: OnceLock<Option<ContentHash>> = OnceLock::new();
    *IDENTITY.get_or_init(|| {
        let executable = std::env::current_exe().ok()?;
        let hash = executable_hash(directory, &executable, std::time::SystemTime::now())?;
        Some(identity_of(env!("CARGO_PKG_VERSION"), &hash))
    })
}

/// How long after its last change an executable's hash is first recorded: a file changed this
/// recently might change again within its file system's timestamp granularity without its
/// metadata changing, as Git's index treats "racily clean" files.
const EXECUTABLE_SETTLE_TIME: std::time::Duration = std::time::Duration::from_secs(2);

/// The hash of the file `executable`, read from its `executable` entry in the cache in
/// `directory` or computed and recorded there. The entry's key covers the file's path and every
/// piece of metadata that changes when its content does: its size, its modification and status
/// change times and, on Unix, its device and inode numbers. A file whose metadata changes
/// while it is hashed, or that changed less than [`EXECUTABLE_SETTLE_TIME`] before `now`, is
/// hashed without being recorded.
fn executable_hash(
    directory: &Path,
    executable: &Path,
    now: std::time::SystemTime,
) -> Option<ContentHash> {
    let before = fs::metadata(executable).ok()?;
    let memo = executable_memo(directory);
    let key = executable_key(executable, &before);
    if let Some(key) = &key {
        if let Some(body) = memo.read_entry(EntryKind::Executable, key) {
            if let Ok(hash) = <[u8; 32]>::try_from(body.as_slice()) {
                return Some(ContentHash(hash));
            }
        }
    }
    let hash = ContentHash::of(&fs::read(executable).ok()?);
    let after = fs::metadata(executable).ok()?;
    let settled = before
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= EXECUTABLE_SETTLE_TIME);
    if let Some(key) = key.filter(|key| settled && executable_key(executable, &after) == Some(*key))
    {
        memo.write_entry(EntryKind::Executable, &key, &hash.0);
    }
    Some(hash)
}

/// The store of `executable` entries in the cache in `directory`: `<directory>/executables`, kept
/// apart from the entries of checks, records and code, so that what a check or run writes there
/// stays a function of its inputs alone. An executable entry's key is a function of a file's
/// metadata, and whether one is written depends on the file's age.
fn executable_memo(directory: &Path) -> BuildCache {
    BuildCache::open(directory.join("executables"), ContentHash([0; 32]))
}

/// The key of the `executable` entry of the file at `path` with `metadata`, or `None` when the
/// platform reports no modification time.
fn executable_key(path: &Path, metadata: &fs::Metadata) -> Option<ContentHash> {
    let nanoseconds = |time: std::time::SystemTime| {
        time.duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64)
    };
    let mut hasher = DeterministicHasher::new();
    hasher.write_str(MAGIC);
    hasher.write_str(EntryKind::Executable.as_str());
    hasher.write_u64(FORMAT_VERSION);
    hasher.write_str(env!("CARGO_PKG_VERSION"));
    hasher.write_str(&path.to_string_lossy());
    hasher.write_u64(metadata.len());
    hasher.write_u64(nanoseconds(metadata.modified().ok()?));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.write_u64(metadata.dev());
        hasher.write_u64(metadata.ino());
        hasher.write_u64(metadata.ctime() as u64);
        hasher.write_u64(metadata.ctime_nsec() as u64);
    }
    #[cfg(not(unix))]
    if let Ok(created) = metadata.created() {
        hasher.write_u64(nanoseconds(created));
    }
    Some(hasher.finish())
}

/// The identity of the compiler of version `version` whose executable hashes to `executable`.
pub fn identity_of(version: &str, executable: &ContentHash) -> ContentHash {
    let mut hasher = DeterministicHasher::new();
    hasher.write_str("prism-compiler");
    hasher.write_str(version);
    hasher.write_hash(executable);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_mir::InstanceRequest;
    use prism_typed_tree::{DeclarationPath, Type};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_cache(name: &str) -> (PathBuf, BuildCache) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("prism_cache_unit_{}_{}", name, nanos));
        let cache = BuildCache::open(&directory, identity_of("0.0.0", &ContentHash([7; 32])));
        (directory, cache)
    }

    fn summary() -> FileSummary {
        FileSummary {
            entry_point: Some(DeclarationPath::new("app/main", "main")),
            roots: vec![InstanceRequest {
                callee: DeclarationPath::new("lib/box", "keep"),
                default: None,
                type_arguments: vec![Type::Int],
                span: prism_ast::Span::new(1, 2, 3, 4),
            }],
            generics: Vec::new(),
        }
    }

    #[test]
    fn a_check_entry_round_trips() {
        let (directory, cache) = temporary_cache("round_trip");
        let key = cache.check_key("app/main", "fun main() {}\n", &[ContentHash([1; 32])]);
        assert_eq!(cache.read_check(&key), None);
        cache.write_check(&key, &summary());
        assert_eq!(cache.read_check(&key), Some(summary()));
        assert_eq!(cache.writes(), 1);
        assert!(directory.join("CACHEDIR.TAG").is_file());
        assert!(cache
            .entry_path(&key)
            .starts_with(directory.join(format!("v{}", FORMAT_VERSION))));
        // The header stays readable text.
        let text = fs::read(cache.entry_path(&key)).unwrap();
        assert!(text.starts_with(b"prism-cache 2\nkind check\nkey "));
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn every_kind_round_trips() {
        let (directory, cache) = temporary_cache("kinds");
        let record_bytes = b"not a record".to_vec();
        let record_hash = ContentHash::of(&record_bytes);
        cache.write_record(&record_hash, &record_bytes);
        // The bytes are no record, so they decode to nothing, but the entry is valid.
        assert_eq!(cache.read_record(&record_hash), None);
        assert!(cache.has_entry(EntryKind::Record, &cache.record_key(&record_hash)));
        for kind in [EntryKind::Unit, EntryKind::Instance] {
            let key = ContentHash::of(kind.as_str().as_bytes());
            assert!(!cache.has_code(kind, &key));
            assert_eq!(cache.read_code(kind, &key), None);
            cache.write_code(kind, &key, &[0, 1, 2, 255, b'\n']);
            assert!(cache.has_code(kind, &key));
            assert_eq!(cache.read_code(kind, &key), Some(vec![0, 1, 2, 255, b'\n']));
            let other = if kind == EntryKind::Unit {
                EntryKind::Instance
            } else {
                EntryKind::Unit
            };
            assert!(!cache.has_code(other, &key));
            assert_eq!(cache.read_code(other, &key), None);
        }
        // An empty body is valid.
        let key = ContentHash([3; 32]);
        cache.write_code(EntryKind::Unit, &key, &[]);
        assert_eq!(cache.read_code(EntryKind::Unit, &key), Some(Vec::new()));
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_record_entry_holds_the_record_of_its_hash() {
        let (directory, cache) = temporary_cache("records");
        let source = "fun <T> keep(x: T): T = x\n";
        let file = prism_parser::parse_source(source).unwrap();
        let mut checker = prism_typechecker::TypeChecker::new(None, "main");
        let tree = checker.check_module(&file);
        let record = GenericRecord::of_file(&prism_mir::lower_file(&tree)).remove(0);
        let bytes = record.encode();
        let hash = ContentHash::of(&bytes);
        assert_eq!(cache.read_record(&hash), None);
        cache.write_record(&hash, &bytes);
        assert_eq!(cache.read_record(&hash), Some(record));
        // Bytes of another record under this hash are a miss.
        let other = ContentHash([1; 32]);
        cache.write_record(&other, &bytes);
        assert_eq!(cache.read_record(&other), None);
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn invalid_entries_are_misses() {
        let (directory, cache) = temporary_cache("invalid");
        let key = cache.check_key("app/main", "", &[]);
        cache.write_check(&key, &summary());
        let path = cache.entry_path(&key);
        let valid = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&valid).to_string();

        let truncated = valid[..valid.len() - 10].to_vec();
        let body_start = text.find("length ").unwrap()
            + text[text.find("length ").unwrap()..].find('\n').unwrap()
            + 1;
        let mut flipped = valid.clone();
        flipped[body_start + 2] ^= 0x40;
        let other_version = text
            .replacen("prism-cache 2", "prism-cache 1", 1)
            .into_bytes();
        let other_length = {
            let length = valid.len() - body_start - CHECKSUM_LINE_LENGTH;
            text.replacen(
                &format!("length {}", length),
                &format!("length {}", length + 1),
                1,
            )
            .into_bytes()
        };
        let record_kind = render_entry(EntryKind::Record, &key, &summary().encode());
        let other_key = render_entry(EntryKind::Check, &ContentHash([9; 32]), &summary().encode());
        let undecodable = render_entry(EntryKind::Check, &key, &[7, 7, 7]);
        for corrupt in [
            b"garbage".to_vec(),
            Vec::new(),
            truncated,
            flipped,
            other_version,
            other_length,
            record_kind,
            other_key,
            undecodable,
            vec![0xFF, 0xFE],
        ] {
            fs::write(&path, &corrupt).unwrap();
            assert_eq!(
                cache.read_check(&key),
                None,
                "{:?}",
                String::from_utf8_lossy(&corrupt)
            );
        }

        cache.write_check(&key, &summary());
        assert_eq!(cache.read_check(&key), Some(summary()));
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_truncated_code_entry_has_no_code() {
        let (directory, cache) = temporary_cache("truncated_code");
        let key = ContentHash([4; 32]);
        cache.write_code(EntryKind::Instance, &key, &[1; 1000]);
        let path = cache.entry_path(&key);
        let valid = fs::read(&path).unwrap();
        fs::write(&path, &valid[..valid.len() - 1]).unwrap();
        assert!(!cache.has_code(EntryKind::Instance, &key));
        assert_eq!(cache.read_code(EntryKind::Instance, &key), None);
        // A flipped body byte keeps the length, so only reading the body finds it.
        let mut flipped = valid.clone();
        flipped[200] ^= 1;
        fs::write(&path, &flipped).unwrap();
        assert!(cache.has_code(EntryKind::Instance, &key));
        assert_eq!(cache.read_code(EntryKind::Instance, &key), None);
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn an_unwritable_cache_reads_and_writes_nothing() {
        let (directory, _) = temporary_cache("unwritable");
        fs::write(&directory, "a regular file").unwrap();
        let cache = BuildCache::open(&directory, ContentHash([1; 32]));
        let key = cache.check_key("app/main", "", &[]);
        cache.write_check(&key, &summary());
        cache.write_code(EntryKind::Unit, &key, &[1]);
        cache.write_record(&key, &[1]);
        assert_eq!(cache.read_check(&key), None);
        assert!(!cache.has_code(EntryKind::Unit, &key));
        assert_eq!(cache.read_record(&key), None);
        assert_eq!(cache.writes(), 0);
        fs::remove_file(&directory).ok();
    }

    /// build-cache: "A different compiler re-checks everything".
    #[test]
    fn a_different_compiler_re_checks_everything() {
        let (directory, cache) = temporary_cache("compilers");
        let key = cache.check_key("app/main", "fun main() {}\n", &[]);
        cache.write_check(&key, &summary());
        for other_compiler in [
            identity_of("0.0.0", &ContentHash([8; 32])),
            identity_of("0.0.1", &ContentHash([7; 32])),
        ] {
            let other = BuildCache::open(&directory, other_compiler);
            let other_key = other.check_key("app/main", "fun main() {}\n", &[]);
            assert_ne!(key, other_key);
            assert_eq!(other.read_check(&other_key), None);
            assert_ne!(cache.unit_key(&key, "t"), other.unit_key(&key, "t"));
            assert_ne!(
                cache.instance_key("s", &key, &key, "t"),
                other.instance_key("s", &key, &key, "t")
            );
            assert_ne!(cache.record_key(&key), other.record_key(&key));
        }
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn keys_cover_every_input() {
        let (_, cache) = temporary_cache("keys");
        let base = cache.check_key("app/main", "a", &[ContentHash([1; 32])]);
        assert_eq!(
            base,
            cache.check_key("app/main", "a", &[ContentHash([1; 32])])
        );
        for other in [
            cache.check_key("app/other", "a", &[ContentHash([1; 32])]),
            cache.check_key("app/main", "b", &[ContentHash([1; 32])]),
            cache.check_key("app/main", "a", &[ContentHash([2; 32])]),
            cache.check_key("app/main", "a", &[]),
            cache.check_key(
                "app/main",
                "a",
                &[ContentHash([1; 32]), ContentHash([1; 32])],
            ),
        ] {
            assert_ne!(base, other);
        }

        let unit = cache.unit_key(&base, "aarch64 flags");
        assert_eq!(unit, cache.unit_key(&base, "aarch64 flags"));
        // build-cache: "Another target generates code again".
        assert_ne!(unit, cache.unit_key(&base, "aarch64 other flags"));
        assert_ne!(unit, cache.unit_key(&ContentHash([5; 32]), "aarch64 flags"));
        assert_ne!(unit, base);

        let (record, definitions) = (ContentHash([2; 32]), ContentHash([3; 32]));
        let instance = cache.instance_key("lib/box::keep<Int>", &record, &definitions, "t");
        for other in [
            cache.instance_key("lib/box::keep<Bool>", &record, &definitions, "t"),
            cache.instance_key("lib/box::keep<Int>", &definitions, &definitions, "t"),
            cache.instance_key("lib/box::keep<Int>", &record, &record, "t"),
            cache.instance_key("lib/box::keep<Int>", &record, &definitions, "u"),
        ] {
            assert_ne!(instance, other);
        }
        assert_ne!(cache.record_key(&record), cache.record_key(&definitions));
        // The kind is part of every key.
        assert_ne!(cache.record_key(&base), cache.unit_key(&base, ""));
    }

    /// build-cache: "Default cache location".
    #[test]
    fn default_cache_location() {
        let environment = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        let home = environment(&[("HOME", "/home/u")]);
        assert_eq!(
            cache_directory("macos", &home),
            Some(PathBuf::from("/home/u/Library/Caches/prism"))
        );
        assert_eq!(
            cache_directory("linux", &home),
            Some(PathBuf::from("/home/u/.cache/prism"))
        );
        let xdg = environment(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "/xdg")]);
        assert_eq!(
            cache_directory("linux", &xdg),
            Some(PathBuf::from("/xdg/prism"))
        );
        let relative_xdg = environment(&[("HOME", "/home/u"), ("XDG_CACHE_HOME", "xdg")]);
        assert_eq!(
            cache_directory("linux", &relative_xdg),
            Some(PathBuf::from("/home/u/.cache/prism"))
        );
        let windows = environment(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")]);
        assert_eq!(
            cache_directory("windows", &windows),
            Some(PathBuf::from(r"C:\Users\u\AppData\Local").join("prism"))
        );
        assert_eq!(cache_directory("linux", &environment(&[])), None);
        let overridden = environment(&[("HOME", "/home/u"), ("PRISM_CACHE", "/tmp/c")]);
        assert_eq!(
            cache_directory("macos", &overridden),
            Some(PathBuf::from("/tmp/c"))
        );
        let off = environment(&[("HOME", "/home/u"), ("PRISM_CACHE", "off")]);
        assert_eq!(cache_directory("macos", &off), None);
        let empty = environment(&[("HOME", "/home/u"), ("PRISM_CACHE", "")]);
        assert_eq!(
            cache_directory("linux", &empty),
            Some(PathBuf::from("/home/u/.cache/prism"))
        );
    }

    #[test]
    fn the_running_compiler_has_an_identity() {
        assert!(compiler_identity().is_some());
        assert_eq!(compiler_identity(), compiler_identity());
    }

    /// "A rebuilt compiler is identified by its content" (D16 of `instance-cache`): the hash of
    /// an executable is recorded once it has settled, read back while its metadata is
    /// unchanged, and computed afresh when its content changes.
    #[test]
    fn a_rebuilt_compiler_is_identified_by_its_content() {
        let (directory, _) = temporary_cache("executable");
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("prism-executable");
        fs::write(&executable, b"first").unwrap();
        let first = ContentHash::of(b"first");
        let now = SystemTime::now();
        let entries = |directory: &Path| {
            fs::read_dir(
                directory
                    .join("executables")
                    .join(format!("v{}", FORMAT_VERSION)),
            )
            .map_or(0, |entries| entries.count())
        };
        let checks_and_code = |directory: &Path| directory.join(format!("v{}", FORMAT_VERSION));
        // Just changed: hashed, not recorded.
        assert_eq!(executable_hash(&directory, &executable, now), Some(first));
        assert_eq!(entries(&directory), 0);
        // Settled: recorded, then read back (a wrong recorded hash proves the read).
        let later = now + EXECUTABLE_SETTLE_TIME * 2;
        assert_eq!(executable_hash(&directory, &executable, later), Some(first));
        assert_eq!(entries(&directory), 1);
        assert!(!checks_and_code(&directory).exists());
        let key = executable_key(&executable, &fs::metadata(&executable).unwrap()).unwrap();
        let memo = executable_memo(&directory);
        memo.write_entry(EntryKind::Executable, &key, &[9; 32]);
        assert_eq!(
            executable_hash(&directory, &executable, later),
            Some(ContentHash([9; 32]))
        );
        // Replaced with other content: its metadata changes, so it is hashed again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&executable, b"second, longer").unwrap();
        assert_eq!(
            executable_hash(&directory, &executable, later),
            Some(ContentHash::of(b"second, longer"))
        );
        // A corrupt entry is a miss.
        let key = executable_key(&executable, &fs::metadata(&executable).unwrap()).unwrap();
        fs::write(memo.entry_path(&key), b"garbage").unwrap();
        assert_eq!(
            executable_hash(&directory, &executable, later),
            Some(ContentHash::of(b"second, longer"))
        );
        fs::remove_dir_all(&directory).ok();
    }
}
