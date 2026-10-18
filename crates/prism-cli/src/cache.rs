//! The build cache (the `build-cache` spec; D3 to D8 of `abi-cache`).
//!
//! A per-user, content-addressed store of small text entries. A check entry records that a file
//! passes Phase 2 against its direct imports' interfaces, with the instances it requests; an
//! instance record stands in for an instantiated generic until code generation (D9). Entries
//! live under `<directory>/v<format>/<first two hex digits of the key>/<key>`, are written
//! atomically, end with a checksum, and any problem reading or writing one is a miss: the cache
//! never fails a check.

use prism_abi::{ContentHash, DeterministicHasher};
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The version of the entry format and key derivation. A new version uses a new `v<N>`
/// directory, so compilers of different versions never read each other's entries.
pub const FORMAT_VERSION: u64 = 1;

/// The environment variable naming the cache directory, or `off` to disable the cache.
pub const CACHE_VARIABLE: &str = "PRISM_CACHE";

const MAGIC: &str = "prism-cache";

/// The tag file marking the directory as a cache, for backup tools
/// (<https://bford.info/cachedir/>).
const CACHEDIR_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
# This file is a cache directory tag created by the Prism compiler (prism).\n\
# For information about cache directory tags see https://bford.info/cachedir/\n";

/// A request, found in a file's MIR, to instantiate a generic function or extension of a direct
/// import with concrete type arguments (D9).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstanceRequest {
    /// The generic's module path.
    pub module: String,
    /// The constructor of an extension's receiver type (`lib/box.Box`), `None` for a function.
    pub receiver: Option<String>,
    /// The generic's name.
    pub name: String,
    /// A hash of the type arguments, each with its nominal types qualified by module.
    pub type_arguments: ContentHash,
    /// How the instance is shown to the user (`lib/box::keep<Int>`).
    pub display: String,
}

impl InstanceRequest {
    fn fields_are_writable(&self) -> bool {
        [
            self.module.as_str(),
            self.receiver.as_deref().unwrap_or_default(),
            self.name.as_str(),
            self.display.as_str(),
        ]
        .iter()
        .all(|field| !field.contains(['\t', '\n', '\r']))
    }

    fn to_line(&self) -> String {
        format!(
            "instance\t{}\t{}\t{}\t{}\t{}",
            self.module,
            self.receiver.as_deref().unwrap_or_default(),
            self.name,
            self.type_arguments.to_hex(),
            self.display
        )
    }

    fn from_line(line: &str) -> Option<Self> {
        let mut fields = line.strip_prefix("instance\t")?.split('\t');
        let module = fields.next()?.to_string();
        let receiver = Some(fields.next()?)
            .filter(|r| !r.is_empty())
            .map(str::to_string);
        let name = fields.next()?.to_string();
        let type_arguments = ContentHash::from_hex(fields.next()?)?;
        let display = fields.next()?.to_string();
        if fields.next().is_some() || module.is_empty() || name.is_empty() {
            return None;
        }
        Some(Self {
            module,
            receiver,
            name,
            type_arguments,
            display,
        })
    }
}

/// The kinds of entries the cache holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Check,
    Instance,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            EntryKind::Check => "check",
            EntryKind::Instance => "instance",
        }
    }
}

/// An open build cache: a directory and the identity of the compiler using it.
#[derive(Debug)]
pub struct BuildCache {
    directory: PathBuf,
    compiler: ContentHash,
    tagged: Cell<bool>,
}

impl BuildCache {
    /// The cache in `directory`, used by the compiler whose identity is `compiler`. Nothing is
    /// created until the first write.
    pub fn open(directory: impl Into<PathBuf>, compiler: ContentHash) -> Self {
        Self {
            directory: directory.into(),
            compiler,
            tagged: Cell::new(false),
        }
    }

    /// The cache the environment selects for the running compiler: `PRISM_CACHE`, or the
    /// platform's per-user cache directory. `None` when the cache is `off`, when no cache
    /// directory is known, or when the compiler executable cannot be identified.
    pub fn from_environment() -> Option<Self> {
        let variable = |name: &str| std::env::var(name).ok();
        let directory = cache_directory(std::env::consts::OS, &variable)?;
        Some(Self::open(directory, compiler_identity()?))
    }

    /// The cache's directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The key of a file's check entry: the format version, the compiler, the file's module
    /// path and exact source text, and its direct imports' interface hashes in import order
    /// (D5).
    pub fn check_key(
        &self,
        module: &str,
        source: &str,
        import_interfaces: &[ContentHash],
    ) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Check);
        hasher.write_str(module);
        hasher.write_hash(&ContentHash::of(source.as_bytes()));
        hasher.write_u64(import_interfaces.len() as u64);
        for interface in import_interfaces {
            hasher.write_hash(interface);
        }
        hasher.finish()
    }

    /// The key of the instance record of `request`, whose generic has the signature hash
    /// `signature_hash` ([`prism_abi::ExportedSymbol::interface_hash`]) and the Monomorphization
    /// Hash `body_hash` (D9): an instance depends on the generic's signature as much as on its
    /// body.
    pub fn instance_key(
        &self,
        request: &InstanceRequest,
        signature_hash: &ContentHash,
        body_hash: &ContentHash,
    ) -> ContentHash {
        let mut hasher = self.key_hasher(EntryKind::Instance);
        hasher.write_str(&request.module);
        match &request.receiver {
            Some(receiver) => {
                hasher.write_u8(1);
                hasher.write_str(receiver);
            }
            None => hasher.write_u8(0),
        }
        hasher.write_str(&request.name);
        hasher.write_hash(signature_hash);
        hasher.write_hash(body_hash);
        hasher.write_hash(&request.type_arguments);
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

    /// The instance requests of the check entry `key`, or `None` on a miss.
    pub fn read_check(&self, key: &ContentHash) -> Option<Vec<InstanceRequest>> {
        let body = self.read_entry(EntryKind::Check, key)?;
        body.iter()
            .map(|line| InstanceRequest::from_line(line))
            .collect()
    }

    /// Records the check entry `key` with its instance requests. Does nothing when a field
    /// cannot be written or the cache cannot be written to.
    pub fn write_check(&self, key: &ContentHash, requests: &[InstanceRequest]) {
        if !requests.iter().all(InstanceRequest::fields_are_writable) {
            return;
        }
        let lines: Vec<String> = requests.iter().map(InstanceRequest::to_line).collect();
        self.write_entry(EntryKind::Check, key, &lines);
    }

    /// Whether the instance record `key` exists and is valid.
    pub fn has_instance(&self, key: &ContentHash) -> bool {
        self.read_entry(EntryKind::Instance, key)
            .is_some_and(|body| body.is_empty())
    }

    /// Records the instance record `key`.
    pub fn record_instance(&self, key: &ContentHash) {
        self.write_entry(EntryKind::Instance, key, &[]);
    }

    /// The path of the entry `key`.
    pub fn entry_path(&self, key: &ContentHash) -> PathBuf {
        let hex = key.to_hex();
        self.directory
            .join(format!("v{}", FORMAT_VERSION))
            .join(&hex[..2])
            .join(hex)
    }

    /// The body lines of the entry `key` of `kind`, or `None` when it is missing, unreadable or
    /// invalid (D7).
    fn read_entry(&self, kind: EntryKind, key: &ContentHash) -> Option<Vec<String>> {
        let bytes = fs::read(self.entry_path(key)).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        parse_entry(&text, kind, key)
    }

    /// Writes the entry `key` of `kind` to a temporary file in its directory and renames it into
    /// place (D8). Every failure is ignored: the entry is then simply not recorded (D7).
    fn write_entry(&self, kind: EntryKind, key: &ContentHash, body: &[String]) {
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
        write_atomically(&path, render_entry(kind, key, body).as_bytes());
    }
}

/// The text of an entry: a header naming the format version, kind and key, the body lines, and
/// a checksum of everything before it (D6).
fn render_entry(kind: EntryKind, key: &ContentHash, body: &[String]) -> String {
    let mut text = format!(
        "{} {}\nkind {}\nkey {}\n",
        MAGIC,
        FORMAT_VERSION,
        kind.as_str(),
        key.to_hex()
    );
    for line in body {
        text.push_str(line);
        text.push('\n');
    }
    let checksum = ContentHash::of(text.as_bytes());
    text.push_str(&format!("checksum {}\n", checksum.to_hex()));
    text
}

/// The body lines of `text` if it is a valid entry of `kind` for `key`.
fn parse_entry(text: &str, kind: EntryKind, key: &ContentHash) -> Option<Vec<String>> {
    let content = text.strip_suffix('\n')?;
    let checksum_start = content.rfind('\n')? + 1;
    let checksum = ContentHash::from_hex(content[checksum_start..].strip_prefix("checksum ")?)?;
    let checked = &text[..checksum_start];
    if ContentHash::of(checked.as_bytes()) != checksum {
        return None;
    }
    let mut lines = checked.lines();
    if lines.next()? != format!("{} {}", MAGIC, FORMAT_VERSION)
        || lines.next()? != format!("kind {}", kind.as_str())
        || lines.next()? != format!("key {}", key.to_hex())
    {
        return None;
    }
    Some(lines.map(str::to_string).collect())
}

/// Writes `bytes` to `path` through a temporary file in the same directory and a rename, so
/// that readers see no file or a complete one. Failures are ignored.
fn write_atomically(path: &Path, bytes: &[u8]) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let temporary = path.with_file_name(format!(
        "{}.tmp-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::write(&temporary, bytes).is_err() {
        fs::remove_file(&temporary).ok();
        return;
    }
    if fs::rename(&temporary, path).is_err() {
        fs::remove_file(&temporary).ok();
    }
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
        let executable = fs::read(std::env::current_exe().ok()?).ok()?;
        Some(identity_of(
            env!("CARGO_PKG_VERSION"),
            &ContentHash::of(&executable),
        ))
    })
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

    fn request(name: &str) -> InstanceRequest {
        InstanceRequest {
            module: "lib/box".to_string(),
            receiver: None,
            name: name.to_string(),
            type_arguments: ContentHash::of(b"Int"),
            display: format!("lib/box::{}<Int>", name),
        }
    }

    #[test]
    fn a_check_entry_round_trips() {
        let (directory, cache) = temporary_cache("round_trip");
        let key = cache.check_key("app/main", "fun main() {}\n", &[ContentHash([1; 32])]);
        assert_eq!(cache.read_check(&key), None);
        let mut extension = request("get");
        extension.receiver = Some("lib/box.Box".to_string());
        let requests = vec![request("keep"), extension];
        cache.write_check(&key, &requests);
        assert_eq!(cache.read_check(&key), Some(requests));
        assert!(directory.join("CACHEDIR.TAG").is_file());
        assert!(cache
            .entry_path(&key)
            .starts_with(directory.join(format!("v{}", FORMAT_VERSION))));
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn instance_records_round_trip() {
        let (directory, cache) = temporary_cache("instances");
        let signature = ContentHash([2; 32]);
        let key = cache.instance_key(&request("keep"), &signature, &ContentHash([3; 32]));
        assert!(!cache.has_instance(&key));
        cache.record_instance(&key);
        assert!(cache.has_instance(&key));
        let other = cache.instance_key(&request("keep"), &signature, &ContentHash([4; 32]));
        assert_ne!(key, other, "the body hash is part of the key");
        assert!(!cache.has_instance(&other));
        let other = cache.instance_key(
            &request("keep"),
            &ContentHash([5; 32]),
            &ContentHash([3; 32]),
        );
        assert_ne!(key, other, "the signature hash is part of the key");
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn invalid_entries_are_misses() {
        let (directory, cache) = temporary_cache("invalid");
        let key = cache.check_key("app/main", "", &[]);
        cache.write_check(&key, &[request("keep")]);
        let path = cache.entry_path(&key);
        let valid = fs::read_to_string(&path).unwrap();

        let truncated = valid[..valid.len() - 10].to_string();
        let flipped = valid.replacen("keep", "kept", 1);
        let other_version = valid.replacen("prism-cache 1", "prism-cache 2", 1);
        let instance_kind = render_entry(EntryKind::Instance, &key, &[]);
        let other_key = render_entry(EntryKind::Check, &ContentHash([9; 32]), &[]);
        let bad_line = render_entry(EntryKind::Check, &key, &["instance\tx".to_string()]);
        for corrupt in [
            "garbage".to_string(),
            String::new(),
            truncated,
            flipped,
            other_version,
            instance_kind,
            other_key,
            bad_line,
        ] {
            fs::write(&path, &corrupt).unwrap();
            assert_eq!(cache.read_check(&key), None, "{:?}", corrupt);
        }
        fs::write(&path, [0xFF, 0xFE]).unwrap();
        assert_eq!(cache.read_check(&key), None);

        cache.write_check(&key, &[request("keep")]);
        assert_eq!(cache.read_check(&key), Some(vec![request("keep")]));
        fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn an_unwritable_cache_reads_and_writes_nothing() {
        let (directory, _) = temporary_cache("unwritable");
        fs::write(&directory, "a regular file").unwrap();
        let cache = BuildCache::open(&directory, ContentHash([1; 32]));
        let key = cache.check_key("app/main", "", &[]);
        cache.write_check(&key, &[]);
        cache.record_instance(&key);
        assert_eq!(cache.read_check(&key), None);
        assert!(!cache.has_instance(&key));
        fs::remove_file(&directory).ok();
    }

    #[test]
    fn unwritable_fields_are_not_cached() {
        let (directory, cache) = temporary_cache("fields");
        let key = cache.check_key("app/main", "", &[]);
        let mut bad = request("keep");
        bad.display = "a\tb".to_string();
        cache.write_check(&key, &[bad]);
        assert_eq!(cache.read_check(&key), None);
        fs::remove_dir_all(&directory).ok();
    }

    /// build-cache: "A different compiler re-checks everything".
    #[test]
    fn a_different_compiler_re_checks_everything() {
        let (directory, cache) = temporary_cache("compilers");
        let key = cache.check_key("app/main", "fun main() {}\n", &[]);
        cache.write_check(&key, &[]);
        for other_compiler in [
            identity_of("0.0.0", &ContentHash([8; 32])),
            identity_of("0.0.1", &ContentHash([7; 32])),
        ] {
            let other = BuildCache::open(&directory, other_compiler);
            let other_key = other.check_key("app/main", "fun main() {}\n", &[]);
            assert_ne!(key, other_key);
            assert_eq!(other.read_check(&other_key), None);
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
}
