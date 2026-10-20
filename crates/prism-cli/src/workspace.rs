//! Workspace and multi-file project driver for Prism CLI.

#![allow(clippy::result_large_err)]

use crate::cache::{BuildCache, EntryKind};
use crate::diagnostics;
use crate::plan::{instance_code_key, planned_display, DefinitionsHasher, ProgramDefinitions};
use prism_abi::{declared_types, type_definitions, AbiError, ContentHash, FileAbi, NominalHeader};
use prism_codegen::{InstanceError, PlannedInstance};
use prism_library::Library;
use prism_mir::{FileSummary, GenericRecord, MirFile};
use prism_parser::ParseError;
use prism_resolver::{scan_directory, DependencyGraph, ResolveError, SourceNode};
use prism_typechecker::{MemberScope, TypedFile};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Phase 1 of a program: the ABI of every file, by path, and the member scope built once from
/// the member modules' ABIs, which every workspace file's checker shares (D2 of
/// `explicit-library`).
pub(crate) struct ProgramAbis {
    pub files: HashMap<PathBuf, FileAbi>,
    /// `None` when the graph has no workspace file, as the library's own graph.
    pub members: Option<Arc<MemberScope>>,
}

impl std::ops::Deref for ProgramAbis {
    type Target = HashMap<PathBuf, FileAbi>;

    fn deref(&self) -> &Self::Target {
        &self.files
    }
}

/// Information about a verified source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFile {
    pub path: PathBuf,
    pub package: String,
    pub exported_symbols: usize,
    pub abi_hash: String,
    /// Whether Phase 2 ran for the file or its cached verdict was reused.
    pub cache: CacheOutcome,
}

/// Whether a file's Phase 2 ran or was skipped thanks to the build cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    /// Phase 2 ran: there was no cache or no entry for the file's inputs.
    Checked,
    /// A cached verdict for exactly these inputs was reused.
    Reused,
    /// Under `prism run`: the cached verdict was reused, but the file's code was generated
    /// (D9 of `instance-cache`).
    Compiled,
}

/// An instance of the program, and whether the cache holds its code (D10 of
/// `instance-cache`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceOutcome {
    /// The instance as shown to the user (`lib/box::keep<Int>`).
    pub instance: String,
    /// Whether the cache holds the instance's code: under `prism run` it was loaded, under
    /// `prism check` the next run will load it. Otherwise it is (or will be) instantiated.
    pub reused: bool,
}

/// How `prism check` runs.
#[derive(Debug)]
pub struct CheckOptions {
    /// The build cache to read and write, or `None` to check every file (D10 of `abi-cache`).
    pub cache: Option<BuildCache>,
    /// Whether to look up which instances the cache holds code for (`--explain-cache`), which
    /// costs one lookup per instance.
    pub explain_cache: bool,
    /// The bundled library, which tests replace to add modules to it (D13 of
    /// `standard-library`). The command line never does.
    pub library: Arc<Library>,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            cache: None,
            explain_cache: false,
            library: Library::shared(),
        }
    }
}

/// The result of running `prism check` on a workspace or entry file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub workspace_root: PathBuf,
    /// The number of workspace files checked; library modules are not counted (the
    /// `build-cache` spec, "Reports List Workspace Files").
    pub total_files: usize,
    /// The workspace files, in check order.
    pub checked_files: Vec<CheckedFile>,
    /// The library modules the program reached, in check order, which only `--explain-cache`
    /// shows.
    pub library_files: Vec<CheckedFile>,
    /// The program's instances, in symbol order (D7 and D10 of `instance-cache`).
    pub instances: Vec<InstanceOutcome>,
}

/// Errors encountered while driving workspace analysis.
#[derive(Debug)]
pub enum WorkspaceError {
    Resolve(ResolveError),
    Parse {
        path: PathBuf,
        source: String,
        error: ParseError,
    },
    Abi {
        path: PathBuf,
        source: String,
        error: AbiError,
    },
    Type {
        path: PathBuf,
        source: String,
        errors: Vec<prism_typechecker::TypeError>,
    },
    /// The borrow checker's errors in one file, which typechecks.
    Borrow {
        path: PathBuf,
        source: String,
        errors: Vec<prism_borrowck::BorrowError>,
    },
    /// The code generator's errors (`E6xxx`) in one file, which passed Phase 2.
    Codegen {
        path: PathBuf,
        source: String,
        errors: Vec<prism_codegen::CodegenError>,
    },
    /// A failure inside the backend, which a checked program never causes.
    Backend(prism_codegen::BackendError),
    /// Errors of several files, rendered in order: the instances of a program may be rejected
    /// in the sources of several generics (D10 of `monomorphization`).
    Several(Vec<WorkspaceError>),
    Io {
        path: PathBuf,
        message: String,
    },
    InvalidTarget(String),
}

impl WorkspaceError {
    /// Renders this error as a formatted compiler diagnostic.
    pub fn render(&self) -> String {
        match self {
            WorkspaceError::Resolve(err) => diagnostics::format_resolve_error(err),
            WorkspaceError::Parse {
                path,
                source,
                error,
            } => diagnostics::format_parse_error(path, Some(source), error),
            WorkspaceError::Abi {
                path,
                source,
                error,
            } => diagnostics::format_abi_error(path, Some(source), error),
            WorkspaceError::Type {
                path,
                source,
                errors,
            } => diagnostics::format_type_errors(path, Some(source), errors),
            WorkspaceError::Borrow {
                path,
                source,
                errors,
            } => diagnostics::format_borrow_errors(path, Some(source), errors),
            WorkspaceError::Codegen {
                path,
                source,
                errors,
            } => diagnostics::format_codegen_errors(path, Some(source), errors),
            WorkspaceError::Backend(error) => {
                format!("error: internal compiler error in the backend: {}\n", error)
            }
            WorkspaceError::Several(errors) => errors
                .iter()
                .map(WorkspaceError::render)
                .collect::<String>(),
            WorkspaceError::Io { path, message } => {
                format!(
                    "error[E1000]: IO error at '{}': {}\n",
                    diagnostics::display_path(path),
                    message
                )
            }
            WorkspaceError::InvalidTarget(msg) => format!("error: {}\n", msg),
        }
    }
}

impl From<ResolveError> for WorkspaceError {
    fn from(err: ResolveError) -> Self {
        WorkspaceError::Resolve(err)
    }
}

/// Checks an individual entry file and its transitively imported dependencies.
///
/// Follows import-directed compilation isolation: unimported sibling files
/// in the same directory are completely excluded from parsing and checking.
pub fn check_file(entry_file: &Path) -> Result<CheckReport, WorkspaceError> {
    check_file_with(entry_file, &CheckOptions::default())
}

/// [`check_file`] with `options`.
pub fn check_file_with(
    entry_file: &Path,
    options: &CheckOptions,
) -> Result<CheckReport, WorkspaceError> {
    validate_entry_file(entry_file)?;
    let graph = DependencyGraph::from_entries_with_library(&[entry_file], &options.library)?;

    // Assert that all reachable files reside inside the discovered workspace root
    assert_workspace_boundary(&graph)?;

    validate_and_extract_abi(&graph, options)
}

/// Rejects an entry that is not an existing `.prism` file.
pub(crate) fn validate_entry_file(entry_file: &Path) -> Result<(), WorkspaceError> {
    if !entry_file.exists() {
        return Err(WorkspaceError::Io {
            path: entry_file.to_path_buf(),
            message: "File does not exist".to_string(),
        });
    }

    if !entry_file.is_file() {
        return Err(WorkspaceError::InvalidTarget(format!(
            "Path '{}' is not a file",
            entry_file.display()
        )));
    }

    let file_name = entry_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if !file_name.ends_with(".prism") {
        return Err(WorkspaceError::InvalidTarget(format!(
            "File '{}' does not have canonical extension '.prism'",
            entry_file.display()
        )));
    }
    Ok(())
}

/// Checks a directory (either a package directory or workspace root), recursively
/// scanning all subdirectories and asserting package homogeneity.
pub fn check_directory(dir: &Path) -> Result<CheckReport, WorkspaceError> {
    check_directory_with(dir, &CheckOptions::default())
}

/// [`check_directory`] with `options`.
pub fn check_directory_with(
    dir: &Path,
    options: &CheckOptions,
) -> Result<CheckReport, WorkspaceError> {
    if !dir.exists() {
        return Err(WorkspaceError::Io {
            path: dir.to_path_buf(),
            message: "Directory does not exist".to_string(),
        });
    }

    if !dir.is_dir() {
        return Err(WorkspaceError::InvalidTarget(format!(
            "Path '{}' is not a directory",
            dir.display()
        )));
    }

    // Recursively collect all directories containing .prism files
    let mut dirs_to_scan = Vec::new();
    collect_dirs_with_prism_files(dir, &mut dirs_to_scan)?;

    if dirs_to_scan.is_empty() {
        return Err(WorkspaceError::InvalidTarget(format!(
            "No '.prism' files found in directory '{}'",
            dir.display()
        )));
    }

    // Enforce package homogeneity on each directory containing .prism files
    let mut all_files = Vec::new();
    for pkg_dir in &dirs_to_scan {
        let files = scan_directory(pkg_dir)?;
        all_files.extend(files);
    }

    all_files.sort();
    all_files.dedup();

    let graph = DependencyGraph::from_entries_with_library(&all_files, &options.library)?;

    // Assert that all files reside inside the discovered workspace root
    assert_workspace_boundary(&graph)?;

    validate_and_extract_abi(&graph, options)
}

/// Universal check dispatcher accepting either a file or a directory. It uses no build cache.
pub fn check_path(path: &Path) -> Result<CheckReport, WorkspaceError> {
    check_path_with(path, &CheckOptions::default())
}

/// [`check_path`] with `options`, such as a build cache.
pub fn check_path_with(path: &Path, options: &CheckOptions) -> Result<CheckReport, WorkspaceError> {
    if path.is_file() {
        check_file_with(path, options)
    } else if path.is_dir() {
        check_directory_with(path, options)
    } else {
        Err(WorkspaceError::Io {
            path: path.to_path_buf(),
            message: "Path does not exist or is not accessible".to_string(),
        })
    }
}

/// Helper to recursively discover directories that contain direct `.prism` files.
fn collect_dirs_with_prism_files(
    dir: &Path,
    dirs: &mut Vec<PathBuf>,
) -> Result<(), WorkspaceError> {
    let entries = fs::read_dir(dir).map_err(|e| WorkspaceError::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;

    let mut has_prism_files = false;
    let mut subdirs = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| WorkspaceError::Io {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;
        let ft = entry.file_type().map_err(|e| WorkspaceError::Io {
            path: entry.path(),
            message: e.to_string(),
        })?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') {
            continue;
        }

        if ft.is_dir() {
            subdirs.push(entry.path());
        } else if name_str.ends_with(".prism") {
            has_prism_files = true;
        }
    }

    if has_prism_files {
        dirs.push(dir.to_path_buf());
    }

    subdirs.sort();
    for sub in subdirs {
        collect_dirs_with_prism_files(&sub, dirs)?;
    }

    Ok(())
}

/// Asserts that all files in the dependency graph reside within the discovered workspace root.
pub(crate) fn assert_workspace_boundary(graph: &DependencyGraph) -> Result<(), WorkspaceError> {
    let effective_root = if graph.workspace_root.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        graph.workspace_root.clone()
    };
    let canon_root = effective_root
        .canonicalize()
        .map_err(|e| WorkspaceError::Io {
            path: effective_root.clone(),
            message: e.to_string(),
        })?;

    for (path, node) in &graph.nodes {
        // A library module is not a file of the workspace (D2 of `standard-library`). A file
        // that exists only in a watch session's overlay was resolved from root-relative
        // segments without `..`, so it is inside the root (D16 of `hot-reload`).
        if node.is_library() || !path.exists() {
            continue;
        }
        let canon_path = path.canonicalize().map_err(|e| WorkspaceError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if !canon_path.starts_with(&canon_root) {
            return Err(WorkspaceError::Resolve(
                ResolveError::WorkspaceBoundaryViolation {
                    import_path: diagnostics::display_path(path),
                    target_path: canon_path,
                    workspace_root: effective_root,
                },
            ));
        }
    }
    Ok(())
}

/// The module path of a source file: its root-relative path without extension, which is also its
/// root-anchored import path (`a/lib` for `<root>/a/lib.prism`). The package segments are the
/// file's directory relative to the workspace root.
pub(crate) fn module_path(node: &SourceNode) -> String {
    // A library module's key is its module path and `.prism` (D2 of `standard-library`).
    if node.is_library() {
        let key = node.path.to_string_lossy();
        return key.strip_suffix(".prism").unwrap_or(&key).to_string();
    }
    let stem = node
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    node.package
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(stem))
        .collect::<Vec<_>>()
        .join("/")
}

/// Validates AST, extracts ABI across all files in Phase 1, and runs Phase 2 in topological SCC
/// order: typechecking each file, lowering each file that typechecks to MIR and borrow-checking
/// that MIR, stopping at the first file with type or borrow errors. With a build cache, a file
/// whose inputs have a cached verdict skips Phase 2 (`abi-cache`), and its summary is read from
/// its check entry. The program's instances are then planned from every file's summary, which
/// reports cross-file polymorphic recursion (`E6004`, D7 of `instance-cache`).
fn validate_and_extract_abi(
    graph: &DependencyGraph,
    options: &CheckOptions,
) -> Result<CheckReport, WorkspaceError> {
    let (abis, definitions) = extract_abis(graph)?;
    let cache = options.cache.as_ref();

    // Phase 2: Typecheck function bodies and expressions in topological SCC order
    let mut checked_files = Vec::new();
    let mut library_files = Vec::new();
    let mut summaries: Vec<FileSummary> = Vec::new();

    for cluster in graph.topological_order() {
        for file_path in &cluster.files {
            let node = graph.get_node(file_path).expect("node must exist in graph");
            let key = cache.map(|cache| file_check_key(cache.keys(), graph, &abis, file_path));
            let cached = cache.zip(key.as_ref()).and_then(|(c, k)| c.read_check(k));

            let (outcome, summary) = match cached {
                Some(summary) => (CacheOutcome::Reused, summary),
                None => {
                    let file = elaborate_and_summarize(graph, &abis, file_path)?;
                    if let Some((cache, key)) = cache.zip(key.as_ref()) {
                        file.record(cache, key);
                    }
                    (CacheOutcome::Checked, file.summary)
                }
            };
            summaries.push(summary);

            let abi = abis.get(file_path).expect("abi must exist");
            let package_str = if node.package.is_empty() {
                "<root>".to_string()
            } else {
                node.package.join(".")
            };

            let checked = CheckedFile {
                path: file_path.clone(),
                package: package_str,
                exported_symbols: abi.symbols.len(),
                abi_hash: abi.abi_hash_hex(),
                cache: outcome,
            };
            if node.is_library() {
                library_files.push(checked);
            } else {
                checked_files.push(checked);
            }
        }
    }

    let planned = prism_codegen::plan_instances(&summaries.iter().collect::<Vec<_>>())
        .map_err(|errors| instance_errors(graph, errors))?;
    let instances = instance_outcomes(
        cache.filter(|_| options.explain_cache),
        &definitions,
        &planned,
    )?;

    Ok(CheckReport {
        workspace_root: graph.workspace_root.clone(),
        total_files: checked_files.len(),
        checked_files,
        library_files,
        instances,
    })
}

/// The key of the check entry of the file at `file_path`: its module path, its source and its
/// direct imports' ABI hashes, which cover no body (D3 of `instance-cache`), then for a
/// workspace file the member modules' (D14 of `explicit-library`). A library module is keyed
/// by the same inputs, so its entries are shared by every workspace.
pub(crate) fn file_check_key(
    keys: &crate::cache::CacheKeys,
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, FileAbi>,
    file_path: &Path,
) -> ContentHash {
    let node = graph.get_node(file_path).expect("node must exist in graph");
    let import_hashes: Vec<ContentHash> = node
        .imports_with_member_modules()
        .filter_map(|dep| abis.get(dep))
        .map(|abi| abi.abi_hash)
        .collect();
    keys.check_key(&module_path(node), &node.source, &import_hashes)
}

/// The source of the file at `path` as `graph` read it, which diagnostics show: under a watch
/// session's overlay it is not what the disk holds (D16 of `hot-reload`).
pub(crate) fn source_of(graph: &DependencyGraph, path: &Path) -> String {
    match graph.get_node(path) {
        Some(node) => node.source.clone(),
        None => fs::read_to_string(path).unwrap_or_default(),
    }
}

/// A file that passed Phase 2: its elaborated MIR, its generic records with their encodings,
/// and its summary.
pub(crate) struct ElaboratedFile {
    pub mir: MirFile,
    pub records: Vec<GenericRecord>,
    /// The encoding of each record, whose hash is its Monomorphization Hash.
    pub encodings: Vec<Vec<u8>>,
    pub summary: FileSummary,
}

impl ElaboratedFile {
    /// Writes the file's record entries, then its check entry `key` with its summary (D14 of
    /// `instance-cache`: each entry is true whatever another file of the run does).
    pub fn record(&self, cache: &BuildCache, key: &ContentHash) {
        for (generic, encoding) in self.summary.generics.iter().zip(&self.encodings) {
            cache.write_record(&ContentHash(generic.record_hash), encoding);
        }
        cache.write_check(key, &self.summary);
    }
}

/// Phase 2 of the file at `file_path` ([`elaborate_file`]), then its generic records, their
/// Monomorphization Hashes and its summary, with the `main` the backend accepts. Returns its
/// type or borrow errors instead when it has any.
pub(crate) fn elaborate_and_summarize(
    graph: &DependencyGraph,
    abis: &ProgramAbis,
    file_path: &Path,
) -> Result<ElaboratedFile, WorkspaceError> {
    let mir = elaborate_file(graph, abis, file_path)?;
    let records = GenericRecord::of_file(&mir);
    let encodings: Vec<Vec<u8>> = records.iter().map(GenericRecord::encode).collect();
    let hashes: Vec<[u8; 32]> = encodings
        .iter()
        .map(|encoding| ContentHash::of(encoding).0)
        .collect();
    let entry_point = prism_codegen::find_main(&mir).ok();
    let summary = FileSummary::of(&mir, &records, &hashes, entry_point);
    Ok(ElaboratedFile {
        mir,
        records,
        encodings,
        summary,
    })
}

/// The elaborated MIR of the file at `file_path` of `graph`: Phase 2 type-checks it against the
/// ABIs of its direct imports, lowers it to MIR from its typed tree alone (D1 of
/// `mir-lowering`) and borrow-checks that MIR alone, never another file's (D1 of
/// `borrowck-moves`). Returns its type or borrow errors instead when it has any. The summaries
/// of `prism check` and the backend of `prism run` consume it.
pub(crate) fn elaborate_file(
    graph: &DependencyGraph,
    abis: &ProgramAbis,
    file_path: &Path,
) -> Result<MirFile, WorkspaceError> {
    let (tree, errors) = typecheck_file(graph, abis, file_path);
    if !errors.is_empty() {
        let source = source_of(graph, file_path);
        return Err(WorkspaceError::Type {
            path: file_path.to_path_buf(),
            source,
            errors,
        });
    }
    let mir = prism_mir::lower_file(&tree);
    let elaborated = prism_borrowck::check_file(&mir).map_err(|errors| {
        let source = source_of(graph, file_path);
        WorkspaceError::Borrow {
            path: file_path.to_path_buf(),
            source,
            errors,
        }
    })?;
    // Records, instances and the backend read composed MIR, so `prism check` composes too (D2
    // of `compose-transform`).
    Ok(prism_compose_transform::compose_file(&elaborated))
}

/// The errors of a program's instances, each in the file of its generic's module, grouped by
/// file in the order the program's files are checked.
pub(crate) fn instance_errors(
    graph: &DependencyGraph,
    errors: Vec<InstanceError>,
) -> WorkspaceError {
    let mut grouped: Vec<WorkspaceError> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let node = graph.get_node(path).expect("node must exist in graph");
            let module = module_path(node);
            let in_file: Vec<prism_codegen::CodegenError> = errors
                .iter()
                .filter(|error| error.module == module)
                .map(|error| error.error.clone())
                .collect();
            if !in_file.is_empty() {
                grouped.push(WorkspaceError::Codegen {
                    path: path.clone(),
                    source: source_of(graph, path),
                    errors: in_file,
                });
            }
        }
    }
    if grouped.len() == 1 {
        grouped.remove(0)
    } else {
        WorkspaceError::Several(grouped)
    }
}

/// The instances `planned`, each with whether `cache` holds its code: one lookup of a code
/// entry's header per instance, done only when `cache` is given (`--explain-cache`).
fn instance_outcomes(
    cache: Option<&BuildCache>,
    definitions: &ProgramDefinitions,
    planned: &[PlannedInstance],
) -> Result<Vec<InstanceOutcome>, WorkspaceError> {
    let target = match cache {
        Some(_) => Some(prism_codegen::isa::target_identity().map_err(WorkspaceError::Backend)?),
        None => None,
    };
    let mut hasher = DefinitionsHasher::new(definitions);
    Ok(planned
        .iter()
        .map(|instance| InstanceOutcome {
            instance: planned_display(instance),
            reused: cache.zip(target.as_deref()).is_some_and(|(cache, target)| {
                let key = instance_code_key(cache.keys(), instance, &mut hasher, target);
                cache.has_code(EntryKind::Instance, &key)
            }),
        })
        .collect())
}

/// The text `--explain-cache` prints (the `build-cache` spec): `Cache: <directory>` or
/// `Cache: off`, then each file in check order, library modules first, as `checked`, `reused`
/// or `compiled` under its path relative to `workspace_root` with `/` separators, a library
/// module under its key (`prism/core.prism`), then the program's instances once, in symbol
/// order, as `instantiate <instance>` or `reused instance <instance>` (D10 of
/// `instance-cache`). A library module's key is relative; a workspace file's path never is.
pub fn explain_cache(
    directory: Option<&Path>,
    workspace_root: &Path,
    files: &[(&Path, CacheOutcome)],
    instances: &[InstanceOutcome],
) -> String {
    let (library, workspace): (Vec<_>, Vec<_>) =
        files.iter().partition(|(path, _)| path.is_relative());
    let files: Vec<&(&Path, CacheOutcome)> = library.into_iter().chain(workspace).collect();
    let mut text = match directory {
        Some(directory) => format!("Cache: {}\n", directory.display()),
        None => "Cache: off\n".to_string(),
    };
    let root = match workspace_root.as_os_str().is_empty() {
        true => Path::new("."),
        false => workspace_root,
    };
    let root = root.canonicalize().ok();
    for (path, outcome) in files {
        let canonical = path
            .is_absolute()
            .then(|| path.canonicalize().ok())
            .flatten();
        let relative = root
            .as_ref()
            .zip(canonical.as_ref())
            .and_then(|(root, path)| path.strip_prefix(root).ok())
            .unwrap_or(path);
        let shown: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let outcome = match outcome {
            CacheOutcome::Checked => "checked",
            CacheOutcome::Reused => "reused",
            CacheOutcome::Compiled => "compiled",
        };
        text.push_str(&format!("  {} {}\n", outcome, shown.join("/")));
    }
    for instance in instances {
        let outcome = if instance.reused {
            "reused instance"
        } else {
            "instantiate"
        };
        text.push_str(&format!("  {} {}\n", outcome, instance.instance));
    }
    text
}

/// Phase 1: extracts the ABI of every file of `graph` from its AST alone. No body is read.
pub(crate) fn extract_abis(
    graph: &DependencyGraph,
) -> Result<(ProgramAbis, ProgramDefinitions), WorkspaceError> {
    // Phase 1a: the structs and enums each file exports, with their kinds and markers, read from
    // its AST alone.
    let exported_types: HashMap<&Path, HashMap<&str, NominalHeader>> = graph
        .nodes
        .iter()
        .map(|(path, node)| (path.as_path(), declared_types(&node.source_file, true)))
        .collect();

    // The binder of each file: its type names bound to the one direct import that exports them
    // (no library module is an implicit import, D1 of `explicit-library`).
    let bind_from = |imports: &[PathBuf], name: &str| {
        let mut found = imports.iter().filter_map(|dep| {
            let header = exported_types.get(dep.as_path())?.get(name)?;
            Some((header.clone(), module_path(graph.get_node(dep)?)))
        });
        match (found.next(), found.next()) {
            // A name exported by several imports stays unbound; Phase 2 reports `E4012`.
            (Some(binding), None) => Some(Ok(binding)),
            (Some(_), Some(_)) => Some(Err(())),
            _ => None,
        }
    };
    let bind_import =
        |node: &SourceNode, name: &str| bind_from(&node.direct_imports, name).and_then(Result::ok);

    // Phase 1a, continued: the definition of every struct and enum of every file, private ones
    // included, bound as Phase 1b binds signature types (D3 of `codegen-aggregates`).
    let mut definitions: ProgramDefinitions = HashMap::new();
    for node in graph.nodes.values() {
        let bind = |name: &str| bind_import(node, name);
        for definition in type_definitions(&node.source_file, &module_path(node), &bind) {
            definitions
                .entry((definition.module.clone(), definition.name.clone()))
                .or_insert(definition);
        }
    }
    let definition = |module: &str, name: &str| {
        definitions
            .get(&(module.to_string(), name.to_string()))
            .cloned()
    };

    // Phase 1b: extract each file's ABI, binding the type names in its exported signatures to
    // its own declarations or to the one direct import that exports them, and recording the
    // definitions of the types its exports reach.
    let mut abis: HashMap<PathBuf, FileAbi> = HashMap::new();
    for (file_path, node) in &graph.nodes {
        let bind = |name: &str| bind_import(node, name);
        let abi = FileAbi::extract_with_origin(
            &node.source_file,
            &module_path(node),
            &bind,
            &definition,
            node.is_library(),
        )
        .map_err(|err| {
            let source = node.source.clone();
            WorkspaceError::Abi {
                path: file_path.clone(),
                source,
                error: err,
            }
        })?;
        abis.insert(file_path.clone(), abi);
    }
    // The member scope, built once from the member modules' ABIs for every workspace file (D2
    // of `explicit-library`).
    let members = graph
        .nodes
        .values()
        .find(|node| !node.is_library())
        .map(|node| {
            let modules: Vec<&FileAbi> = node
                .member_imports
                .iter()
                .filter_map(|module| abis.get(module))
                .collect();
            Arc::new(MemberScope::from_abis(&modules))
        });
    Ok((
        ProgramAbis {
            files: abis,
            members,
        },
        definitions,
    ))
}

/// Phase 2 of one file: checks the file at `file_path` of `graph` against the ABIs of its direct
/// imports only, never another file's body or typed tree (the deterministic hermetic builds
/// invariant), returning its typed tree and its type errors.
fn typecheck_file(
    graph: &DependencyGraph,
    abis: &ProgramAbis,
    file_path: &Path,
) -> (TypedFile, Vec<prism_typechecker::TypeError>) {
    let node = graph.get_node(file_path).expect("node must exist in graph");
    let package_name = if node.package.is_empty() {
        None
    } else {
        Some(node.package.join("."))
    };

    let mut checker = prism_typechecker::TypeChecker::new(package_name, module_path(node));
    // A workspace file sees the members of the built-in types without an import; a library
    // file imports what it uses (D2 and D7 of `explicit-library`).
    if !node.is_library() {
        if let Some(members) = &abis.members {
            checker = checker.with_members(Arc::clone(members));
        }
    }

    // Ingest FileAbi of all direct dependencies
    for dep_path in &node.direct_imports {
        if let Some(dep_abi) = abis.get(dep_path) {
            checker.ingest_file_abi(dep_abi);
        }
    }

    // Pass 1 (file-level forward declarations) and Pass 2 (function body checks)
    let tree = checker.check_module(&node.source_file);
    let mut errors = checker.errors;
    // A workspace file's unresolved library names get help naming their import; a library
    // file imports its siblings by another form (D8 of `explicit-library`).
    if !node.is_library() {
        crate::library_index::suggest_library_imports(&graph.library, &mut errors);
    }
    (tree, errors)
}

/// The typed tree of one file, as `prism dump-typed-tree` prints it.
#[derive(Debug)]
pub struct TypedTreeDump {
    /// The tree's text (`TypedFile::dump`).
    pub text: String,
    /// The file's type errors, if it has any.
    pub type_errors: Option<WorkspaceError>,
    /// The tree itself.
    pub tree: TypedFile,
}

/// Builds the typed tree of the entry file `entry`: runs Phase 1 over its import graph, then
/// Phase 2 for the entry only, so the bodies of its dependencies (and their errors) cannot
/// affect it (D15 of `typed-tree`). Resolve, parse and ABI errors are returned as by
/// [`check_file`], with no tree; type errors come with the tree.
pub fn dump_typed_tree(entry: &Path) -> Result<TypedTreeDump, WorkspaceError> {
    let (tree, type_errors) = typecheck_entry(entry)?;
    Ok(TypedTreeDump {
        text: tree.dump(),
        type_errors,
        tree,
    })
}

/// Phase 1 over the import graph of the entry file `entry`, then Phase 2 for the entry only:
/// its typed tree, and its type errors if it has any. Resolve, parse and ABI errors are
/// returned as by [`check_file`].
fn typecheck_entry(entry: &Path) -> Result<(TypedFile, Option<WorkspaceError>), WorkspaceError> {
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entry(entry)?;
    assert_workspace_boundary(&graph)?;
    let (abis, _) = extract_abis(&graph)?;
    let canonical_entry = entry.canonicalize().map_err(|e| WorkspaceError::Io {
        path: entry.to_path_buf(),
        message: e.to_string(),
    })?;
    let entry_path = graph
        .nodes
        .keys()
        .find(|path| path.canonicalize().ok().as_ref() == Some(&canonical_entry))
        .cloned()
        .ok_or_else(|| WorkspaceError::Io {
            path: entry.to_path_buf(),
            message: "the entry file is not in its own dependency graph".to_string(),
        })?;
    let (tree, errors) = typecheck_file(&graph, &abis, &entry_path);
    let type_errors = (!errors.is_empty()).then(|| WorkspaceError::Type {
        path: entry_path.clone(),
        source: source_of(&graph, &entry_path),
        errors,
    });
    Ok((tree, type_errors))
}

/// The typed tree and the MIR of the library module `module` (`prism/test`) in the program of
/// the entry file `entry`, built with `library` as the bundled library: what the fixtures of
/// injected library modules snapshot (D13 of `standard-library`). The MIR is `None` when the
/// module has type errors, which come with the tree.
pub fn dump_library_module(
    entry: &Path,
    library: &Library,
    module: &str,
) -> Result<(TypedTreeDump, Option<MirDump>), WorkspaceError> {
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entries_with_library(&[entry], library)?;
    assert_workspace_boundary(&graph)?;
    let (abis, _) = extract_abis(&graph)?;
    let key = prism_resolver::library_key(module);
    if graph.get_node(&key).is_none() {
        return Err(WorkspaceError::InvalidTarget(format!(
            "the program of '{}' does not reach the library module '{}'",
            entry.display(),
            module
        )));
    }
    let (tree, errors) = typecheck_file(&graph, &abis, &key);
    let type_errors = (!errors.is_empty()).then(|| WorkspaceError::Type {
        path: key.clone(),
        source: source_of(&graph, &key),
        errors,
    });
    let mir = type_errors.is_none().then(|| {
        let mir = prism_mir::lower_file(&tree);
        MirDump {
            text: mir.dump(),
            mir,
        }
    });
    Ok((
        TypedTreeDump {
            text: tree.dump(),
            type_errors,
            tree,
        },
        mir,
    ))
}

/// The Phase 1 ABIs of the modules `modules` of the bundled library (`prism/compose/ui`), as
/// a program importing them sees them: for the compile-time benchmark, which checks generated
/// programs that import library modules against them.
pub fn library_abis(modules: &[&str]) -> Result<Vec<prism_abi::FileAbi>, WorkspaceError> {
    let graph = DependencyGraph::from_library(Library::bundled())?;
    let (abis, _) = extract_abis(&graph)?;
    Ok(modules
        .iter()
        .filter_map(|module| abis.get(&prism_resolver::library_key(module)).cloned())
        .collect())
}

/// The composed MIR of the library module `module` of `library`, reached from the workspace
/// entry `entry`, as `prism dump-mir --composed` prints a workspace file's: for the library
/// cases of the fixtures (D13 of `standard-library`).
pub fn dump_library_module_composed(
    entry: &Path,
    library: &Library,
    module: &str,
) -> Result<MirDump, WorkspaceError> {
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entries_with_library(&[entry], library)?;
    assert_workspace_boundary(&graph)?;
    let (abis, _) = extract_abis(&graph)?;
    let key = prism_resolver::library_key(module);
    // The file's MIR through the borrow checker and the compose transform.
    let composed = elaborate_file(&graph, &abis, &key)?;
    Ok(MirDump {
        text: composed.dump(),
        mir: composed,
    })
}

/// Checks every module of `library` as the compiler checks a program (D13 of
/// `standard-library`): Phase 1 and Phase 2 of each, in dependency order, the backend's support
/// check of each, which compares every external it declares with the runtime's entry point
/// (`E6005`), and the compilation of each module's unit. Returns the modules' elaborated MIR,
/// in check order, or the first module's errors.
pub fn check_library(library: &Library) -> Result<Vec<MirFile>, WorkspaceError> {
    let graph = DependencyGraph::from_library(library)?;
    let (abis, _) = extract_abis(&graph)?;
    let mut linker = prism_codegen::Linker::new().map_err(WorkspaceError::Backend)?;
    let mut files = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let mir = elaborate_file(&graph, &abis, path)?;
            prism_codegen::check_file(&mir).map_err(|errors| WorkspaceError::Codegen {
                path: path.clone(),
                source: source_of(&graph, path),
                errors,
            })?;
            linker
                .compile_unit(&mir, &mir.module)
                .map_err(WorkspaceError::Backend)?;
            files.push(mir);
        }
    }
    Ok(files)
}

/// The MIR of one file, as `prism dump-mir` prints it.
#[derive(Debug)]
pub struct MirDump {
    /// The MIR's text (`MirFile::dump`).
    pub text: String,
    /// The MIR itself.
    pub mir: MirFile,
}

/// Builds the MIR of the entry file `entry`, checked against the ABIs of its direct imports as
/// by [`dump_typed_tree`] (D15 of `mir-lowering`). A file with type errors is not lowered: its
/// errors are returned instead (D1).
pub fn dump_mir(entry: &Path) -> Result<MirDump, WorkspaceError> {
    let (tree, type_errors) = typecheck_entry(entry)?;
    if let Some(errors) = type_errors {
        return Err(errors);
    }
    let mir = prism_mir::lower_file(&tree);
    Ok(MirDump {
        text: mir.dump(),
        mir,
    })
}

/// Builds the elaborated MIR of the entry file `entry`, as `prism dump-mir --elaborated` prints
/// it: the MIR of [`dump_mir`], borrow-checked, with its drops elaborated. A file with borrow
/// errors has no elaborated MIR: its errors are returned instead (D1 of `borrowck-moves`). Only
/// the entry is borrow-checked, from its own MIR.
pub fn dump_elaborated_mir(entry: &Path) -> Result<MirDump, WorkspaceError> {
    let (_, elaborated) = elaborate_entry(entry)?;
    Ok(MirDump {
        text: elaborated.dump(),
        mir: elaborated,
    })
}

/// Builds the composed MIR of the entry file `entry`, as `prism dump-mir --composed` prints it:
/// the MIR of [`dump_elaborated_mir`] through the compose transform (D19 of
/// `compose-transform`).
pub fn dump_composed_mir(entry: &Path) -> Result<MirDump, WorkspaceError> {
    let (_, composed) = compose_entry(entry)?;
    Ok(MirDump {
        text: composed.dump(),
        mir: composed,
    })
}

/// The typed tree of the entry file `entry` and its composed MIR, or its errors of any stage.
pub(crate) fn compose_entry(entry: &Path) -> Result<(TypedFile, MirFile), WorkspaceError> {
    let (tree, elaborated) = elaborate_entry(entry)?;
    let composed = prism_compose_transform::compose_file(&elaborated);
    Ok((tree, composed))
}

/// The typed tree of the entry file `entry` and its elaborated MIR, as [`dump_elaborated_mir`]
/// builds it, or its errors of any stage.
pub(crate) fn elaborate_entry(entry: &Path) -> Result<(TypedFile, MirFile), WorkspaceError> {
    let (tree, type_errors) = typecheck_entry(entry)?;
    if let Some(errors) = type_errors {
        return Err(errors);
    }
    let mir = prism_mir::lower_file(&tree);
    match prism_borrowck::check_file(&mir) {
        Ok(elaborated) => Ok((tree, elaborated)),
        Err(errors) => {
            let path = entry.to_path_buf();
            let source = fs::read_to_string(entry).unwrap_or_default();
            Err(WorkspaceError::Borrow {
                path,
                source,
                errors,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(test_name: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("prism_test_{}_{}", test_name, ts));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn test_check_single_file_success() {
        let temp = create_temp_dir("single_file");
        let file = temp.join("main.prism");
        fs::write(&file, "fun main(): Int = 42\n").unwrap();

        let report = check_file(&file).unwrap();
        assert_eq!(report.total_files, 1);
        assert_eq!(report.checked_files[0].path, file);
        assert_eq!(report.checked_files[0].package, "<root>");

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_check_file_with_sibling_import() {
        let temp = create_temp_dir("sibling_import");
        let pkg_dir = temp.join("core");
        fs::create_dir_all(&pkg_dir).unwrap();

        let math = pkg_dir.join("math.prism");
        fs::write(
            &math,
            "package core\npublic fun square(x: Int): Int = x * x\n",
        )
        .unwrap();

        let main = pkg_dir.join("main.prism");
        fs::write(
            &main,
            "package core\nimport math\nfun run(): Int = square(5)\n",
        )
        .unwrap();

        let report = check_file(&main).unwrap();
        assert_eq!(report.total_files, 2);
        // Math must precede main in topological order
        assert_eq!(report.checked_files[0].path, math);
        assert_eq!(report.checked_files[1].path, main);

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_check_file_import_directed_isolation() {
        let temp = create_temp_dir("import_isolation");
        let pkg_dir = temp.join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        let main = pkg_dir.join("main.prism");
        fs::write(&main, "package pkg\nfun main(): Int = 0\n").unwrap();

        // Sibling file that has a syntax error and is NOT imported
        let draft = pkg_dir.join("draft.prism");
        fs::write(&draft, "THIS IS TOTAL INVALID SYNTAX !!!@@@\n").unwrap();

        // check_file(main) must succeed because draft is unimported
        let report = check_file(&main).unwrap();
        assert_eq!(report.total_files, 1);
        assert_eq!(report.checked_files[0].path, main);

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_check_file_missing_import_error() {
        let temp = create_temp_dir("missing_import");
        let main = temp.join("main.prism");
        fs::write(&main, "import nonexistent\nfun main(): Int = 0\n").unwrap();

        let err = check_file(&main).unwrap_err();
        match err {
            WorkspaceError::Resolve(ResolveError::FileNotFound { import_path, .. }) => {
                assert_eq!(import_path, "nonexistent");
            }
            other => panic!("expected FileNotFound error, got {:?}", other),
        }

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_check_file_mutual_recursion_succeeds() {
        let temp = create_temp_dir("mutual_recursion");
        let pkg_dir = temp.join("p");
        fs::create_dir_all(&pkg_dir).unwrap();

        let a = pkg_dir.join("a.prism");
        let b = pkg_dir.join("b.prism");

        fs::write(&a, "package p\nimport b\npublic fun fa(): Int = 1\n").unwrap();
        fs::write(&b, "package p\nimport a\npublic fun fb(): Int = 2\n").unwrap();

        let report = check_file(&a).unwrap();
        assert_eq!(report.total_files, 2);

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_symlink_workspace_boundary_rejected() {
        let temp = create_temp_dir("symlink_escape");
        let outside = create_temp_dir("outside");
        let secret = outside.join("secret.prism");
        fs::write(&secret, "package pkg\npublic fun secret(): Int = 42\n").unwrap();

        let pkg_dir = temp.join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        let link = pkg_dir.join("escaped.prism");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        #[cfg(unix)]
        {
            let err = check_file(&link).unwrap_err();
            match err {
                WorkspaceError::Resolve(ResolveError::WorkspaceBoundaryViolation { .. }) => {}
                other => panic!("expected WorkspaceBoundaryViolation, got {:?}", other),
            }
        }

        fs::remove_dir_all(&temp).ok();
        fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn test_check_directory_homogeneity() {
        let temp = create_temp_dir("dir_homogeneity");
        let pkg_dir = temp.join("ui");
        fs::create_dir_all(&pkg_dir).unwrap();

        let f1 = pkg_dir.join("view1.prism");
        let f2 = pkg_dir.join("view2.prism");

        fs::write(&f1, "package ui\nfun v1(): Int = 1\n").unwrap();
        fs::write(&f2, "package conflicting\nfun v2(): Int = 2\n").unwrap();

        let err = check_directory(&pkg_dir).unwrap_err();
        match err {
            WorkspaceError::Resolve(ResolveError::PackageHeterogeneity {
                declared_package,
                expected_package,
                ..
            }) => {
                assert_eq!(declared_package, "conflicting");
                assert_eq!(expected_package, "ui");
            }
            other => panic!("expected PackageHeterogeneity, got {:?}", other),
        }

        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_check_directory_recursive_multi_package() {
        let temp = create_temp_dir("multi_package");
        let core = temp.join("core");
        let app = temp.join("app");
        fs::create_dir_all(&core).unwrap();
        fs::create_dir_all(&app).unwrap();

        let math = core.join("math.prism");
        fs::write(
            &math,
            "package core\npublic fun add(a: Int, b: Int): Int = a + b\n",
        )
        .unwrap();

        let main = app.join("main.prism");
        fs::write(
            &main,
            "package app\nimport /core/math\nfun run(): Int = add(1, 2)\n",
        )
        .unwrap();

        let report = check_directory(&temp).unwrap();
        assert_eq!(report.total_files, 2);
        // core/math must precede app/main
        assert_eq!(report.checked_files[0].path, math);
        assert_eq!(report.checked_files[1].path, main);

        fs::remove_dir_all(&temp).ok();
    }

    /// D13 of `instance-cache`: the program-wide table built from Phase 1a definitions agrees
    /// with every elaborated file's table on every type both hold.
    #[test]
    fn the_program_table_agrees_with_every_file_table() {
        let temp = create_temp_dir("program_table");
        fs::create_dir_all(temp.join("lib")).unwrap();
        fs::create_dir_all(temp.join("app")).unwrap();
        fs::write(
            temp.join("lib/shapes.prism"),
            "package lib\nstruct Secret : Copy { val k: Int }\n\
             public struct Pair<A, B>(val a: A, val b: B)\n\
             public enum Shape<T> : Clone { Dot, Point(T, Int) }\n\
             public struct Holder : Copy { val s: Secret, val n: Int }\n\
             public fun holder(): Holder = Holder(Secret(1), 2)\n",
        )
        .unwrap();
        fs::write(
            temp.join("app/main.prism"),
            "package app\nimport /lib/shapes\nimport /prism/core\nstruct Thing(val p: Pair<Int, String>, val c: Chain)\n\
             enum Chain { End, Link(Int, Heap<Chain>) }\n\
             fun main() { val s: Shape<Int> = Shape.Dot; println(holder().n) }\n",
        )
        .unwrap();
        let graph = DependencyGraph::from_entry(&temp.join("app/main.prism")).unwrap();
        let (abis, definitions) = extract_abis(&graph).unwrap();
        let program = crate::plan::program_types(&definitions);
        let mut compared = 0;
        for path in graph.nodes.keys() {
            let mir = elaborate_file(&graph, &abis, path).unwrap();
            for id in mir.types.ids() {
                if let (Some(file), Some(whole)) =
                    (mir.types.definition(id), program.definition(id))
                {
                    assert_eq!(file, whole, "{}::{}", id.module, id.name);
                    compared += 1;
                }
                if let Some(marker) = mir.types.marker_entry(id) {
                    assert_eq!(program.marker_entry(id), Some(marker), "{}", id.name);
                }
            }
        }
        assert!(compared >= 6, "{}", compared);
        fs::remove_dir_all(&temp).ok();
    }

    /// build-cache: "Cache Keys Cover Exactly the Compilation Inputs": a workspace file's key
    /// covers the member modules' ABI hashes after its imports', and a library module's key
    /// its own inputs only (D3 of `standard-library`, D14 of `explicit-library`).
    #[test]
    fn keys_cover_the_member_modules() {
        let temp = create_temp_dir("member_keys");
        let main = temp.join("main.prism");
        fs::write(&main, "fun main() {}\n").unwrap();
        let keys = crate::cache::CacheKeys::in_process();
        let key_of = |library: &Library, path: &Path| {
            let graph = DependencyGraph::from_entries_with_library(&[&main], library).unwrap();
            let (abis, _) = extract_abis(&graph).unwrap();
            file_check_key(&keys, &graph, &abis, path)
        };
        let bundled = Library::bundled();
        let collections = &bundled.module("prism/collections").unwrap().source;
        // A new public helper changes the ABI hash of a member module.
        let grown = Library::with_modules(vec![(
            "prism/collections".to_string(),
            format!(
                "{}\n// Nothing.\npublic fun nothing(): Unit {{}}\n",
                collections
            ),
        )]);
        assert_ne!(key_of(bundled, &main), key_of(&grown, &main));
        let core = Path::new("prism/core.prism");
        assert_eq!(key_of(bundled, core), key_of(&grown, core));
        let collections_key = Path::new("prism/collections.prism");
        assert_ne!(
            key_of(bundled, collections_key),
            key_of(&grown, collections_key)
        );
        fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn test_cached_report_matches_uncached_report() {
        let temp = create_temp_dir("cached_report");
        let lib = temp.join("lib");
        let app = temp.join("app");
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(&app).unwrap();
        fs::write(
            lib.join("box.prism"),
            "package lib\npublic fun <T> keep(x: T): T = x\n",
        )
        .unwrap();
        fs::write(
            app.join("main.prism"),
            "package app\nimport /lib/box\nfun f(): Int = keep(1)\n",
        )
        .unwrap();
        let options = CheckOptions {
            cache: Some(BuildCache::open(
                temp.join(".cache"),
                crate::cache::identity_of("test", &ContentHash([5; 32])),
            )),
            explain_cache: true,
            ..CheckOptions::default()
        };

        let uncached = check_path(&temp).unwrap();
        let cold = check_path_with(&temp, &options).unwrap();
        let warm = check_path_with(&temp, &options).unwrap();
        assert_eq!(uncached, cold);
        for file in &uncached.checked_files {
            assert_eq!(file.cache, CacheOutcome::Checked);
        }
        assert_eq!(
            uncached.instances,
            [InstanceOutcome {
                instance: "lib/box::keep<Int>".to_string(),
                reused: false,
            }]
        );

        // No run generated the instance's code, so it stays one to instantiate.
        let mut expected = uncached.clone();
        for file in expected
            .checked_files
            .iter_mut()
            .chain(&mut expected.library_files)
        {
            file.cache = CacheOutcome::Reused;
        }
        assert_eq!(warm, expected);
        assert_eq!(uncached.total_files, 2, "library modules are not counted");

        // An injected library module, imported by the program, is checked and reused like the
        // others (D13 of `standard-library`).
        fs::write(
            app.join("main.prism"),
            "package app\nimport /lib/box\nimport /prism/test\nfun f(): Int = keep(seven())\n",
        )
        .unwrap();
        let options = CheckOptions {
            library: Arc::new(Library::with_modules(vec![(
                "prism/test".to_string(),
                "package prism\npublic fun seven(): Int = 7\n".to_string(),
            )])),
            ..options
        };
        let uncached = check_path_with(
            &temp,
            &CheckOptions {
                cache: None,
                explain_cache: true,
                library: Arc::clone(&options.library),
            },
        )
        .unwrap();
        let cold = check_path_with(&temp, &options).unwrap();
        let warm = check_path_with(&temp, &options).unwrap();
        // The cache already holds the entries of the files the edit left unchanged.
        let outcomes_cleared = |report: &CheckReport| {
            let mut report = report.clone();
            for file in report
                .checked_files
                .iter_mut()
                .chain(&mut report.library_files)
            {
                file.cache = CacheOutcome::Checked;
            }
            report
        };
        assert_eq!(outcomes_cleared(&uncached), outcomes_cleared(&cold));
        assert!(cold
            .library_files
            .iter()
            .any(|file| file.path == Path::new("prism/test.prism")
                && file.cache == CacheOutcome::Checked));
        assert!(uncached
            .library_files
            .iter()
            .any(|file| file.path == Path::new("prism/test.prism")));
        assert!(warm
            .library_files
            .iter()
            .chain(&warm.checked_files)
            .all(|file| file.cache == CacheOutcome::Reused));

        fs::remove_dir_all(&temp).ok();
    }
}
