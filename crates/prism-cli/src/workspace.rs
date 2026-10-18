//! Workspace and multi-file project driver for Prism CLI.

#![allow(clippy::result_large_err)]

use crate::cache::{BuildCache, InstanceRequest};
use crate::diagnostics;
use prism_abi::{
    declared_types, type_definitions, AbiError, AbiTypeDefinition, ContentHash,
    DeterministicHasher, FileAbi, NominalHeader,
};
use prism_mir::MirFile;
use prism_parser::ParseError;
use prism_resolver::{scan_directory, DependencyGraph, ResolveError, SourceNode};
use prism_typechecker::TypedFile;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Information about a verified source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFile {
    pub path: PathBuf,
    pub package: String,
    pub exported_symbols: usize,
    pub abi_hash: String,
    /// Whether Phase 2 ran for the file or its cached verdict was reused.
    pub cache: CacheOutcome,
    /// The instances of imported generics the file requests, in order (D9 of `abi-cache`).
    pub instances: Vec<InstanceOutcome>,
}

/// Whether a file's Phase 2 ran or was skipped thanks to the build cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    /// Phase 2 ran: there was no cache or no entry for the file's inputs.
    Checked,
    /// A cached verdict for exactly these inputs was reused.
    Reused,
}

/// An instance a file requests, and whether its record already existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceOutcome {
    /// The instance as shown to the user (`lib/box::keep<Int>`).
    pub instance: String,
    /// Whether the instance was already recorded; otherwise it must be instantiated.
    pub reused: bool,
}

/// How `prism check` runs.
#[derive(Debug, Default)]
pub struct CheckOptions {
    /// The build cache to read and write, or `None` to check every file (D10 of `abi-cache`).
    pub cache: Option<BuildCache>,
}

/// The result of running `prism check` on a workspace or entry file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub workspace_root: PathBuf,
    pub total_files: usize,
    pub checked_files: Vec<CheckedFile>,
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
    let graph = DependencyGraph::from_entry(entry_file)?;

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

    let graph = DependencyGraph::from_entries(&all_files)?;

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

    for path in graph.nodes.keys() {
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
/// whose inputs have a cached verdict skips Phase 2 (`abi-cache`).
fn validate_and_extract_abi(
    graph: &DependencyGraph,
    options: &CheckOptions,
) -> Result<CheckReport, WorkspaceError> {
    let abis = extract_abis(graph)?;
    let cache = options.cache.as_ref();

    // Phase 2: Typecheck function bodies and expressions in topological SCC order
    let mut checked_files = Vec::new();

    for cluster in graph.topological_order() {
        for file_path in &cluster.files {
            let node = graph.get_node(file_path).expect("node must exist in graph");
            let imports: Vec<&FileAbi> = node
                .direct_imports
                .iter()
                .filter_map(|dep| abis.get(dep))
                .collect();
            // The verdict depends on the imports' interfaces only, never on their generic bodies
            // (D2 of `abi-cache`).
            let key = cache.map(|cache| {
                let interfaces: Vec<ContentHash> =
                    imports.iter().map(|abi| abi.interface_hash).collect();
                cache.check_key(&module_path(node), &node.source, &interfaces)
            });
            let cached = cache.zip(key.as_ref()).and_then(|(c, k)| c.read_check(k));

            let (outcome, requests) = match cached {
                Some(requests) => (CacheOutcome::Reused, requests),
                None => {
                    let requests = check_phase_two(graph, &abis, file_path)?;
                    if let Some((cache, key)) = cache.zip(key.as_ref()) {
                        cache.write_check(key, &requests);
                    }
                    (CacheOutcome::Checked, requests)
                }
            };
            let instances = instance_outcomes(cache, &imports, &requests);

            let abi = abis.get(file_path).expect("abi must exist");
            let package_str = if node.package.is_empty() {
                "<root>".to_string()
            } else {
                node.package.join(".")
            };

            checked_files.push(CheckedFile {
                path: file_path.clone(),
                package: package_str,
                exported_symbols: abi.symbols.len(),
                abi_hash: abi.abi_hash_hex(),
                cache: outcome,
                instances,
            });
        }
    }

    Ok(CheckReport {
        workspace_root: graph.workspace_root.clone(),
        total_files: checked_files.len(),
        checked_files,
    })
}

/// Phase 2 of the file at `file_path`: type checking, MIR lowering and borrow checking. Returns
/// the instances its elaborated MIR requests, or its type or borrow errors.
fn check_phase_two(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, FileAbi>,
    file_path: &Path,
) -> Result<Vec<InstanceRequest>, WorkspaceError> {
    let elaborated = elaborate_file(graph, abis, file_path)?;
    let node = graph.get_node(file_path).expect("node must exist in graph");
    let import_modules: Vec<String> = node
        .direct_imports
        .iter()
        .filter_map(|dep| graph.get_node(dep))
        .map(module_path)
        .collect();
    Ok(instance_requests(&elaborated, &import_modules))
}

/// The elaborated MIR of the file at `file_path` of `graph`: Phase 2 type-checks it against the
/// ABIs of its direct imports, lowers it to MIR from its typed tree alone (D1 of
/// `mir-lowering`) and borrow-checks that MIR alone, never another file's (D1 of
/// `borrowck-moves`). Returns its type or borrow errors instead when it has any. The instance
/// requests of `prism check` and the backend of `prism run` consume it.
pub(crate) fn elaborate_file(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, FileAbi>,
    file_path: &Path,
) -> Result<MirFile, WorkspaceError> {
    let (tree, errors) = typecheck_file(graph, abis, file_path);
    if !errors.is_empty() {
        let source = fs::read_to_string(file_path).unwrap_or_default();
        return Err(WorkspaceError::Type {
            path: file_path.to_path_buf(),
            source,
            errors,
        });
    }
    let mir = prism_mir::lower_file(&tree);
    prism_borrowck::check_file(&mir).map_err(|errors| {
        let source = fs::read_to_string(file_path).unwrap_or_default();
        WorkspaceError::Borrow {
            path: file_path.to_path_buf(),
            source,
            errors,
        }
    })
}

/// The instances `mir` requests: its calls to generics of the modules `import_modules` (its
/// direct imports) with type arguments that mention no type parameter, deduplicated and sorted
/// (D9 of `abi-cache`). Polymorphic calls are instantiated with their caller, and calls to the
/// file's own generics are re-checked with the file.
fn instance_requests(mir: &MirFile, import_modules: &[String]) -> Vec<InstanceRequest> {
    let mut requests = Vec::new();
    for (_, body) in mir.bodies.iter() {
        for (_, block) in body.blocks.iter() {
            let prism_mir::TerminatorKind::Call {
                callee: prism_mir::Callee::Function(path),
                type_arguments,
                ..
            } = &block.terminator.kind
            else {
                continue;
            };
            if type_arguments.is_empty()
                || path.module == mir.module
                || !import_modules.contains(&path.module)
                || type_arguments
                    .iter()
                    .any(prism_typed_tree::Type::mentions_parameter)
            {
                continue;
            }
            let mut hasher = DeterministicHasher::new();
            hasher.write_u64(type_arguments.len() as u64);
            for argument in type_arguments {
                hasher.write_str(&argument.qualified());
            }
            let shown: Vec<String> = type_arguments.iter().map(ToString::to_string).collect();
            requests.push(InstanceRequest {
                module: path.module.clone(),
                receiver: path
                    .receiver
                    .as_ref()
                    .map(prism_typed_tree::Type::receiver_constructor),
                name: path.name.clone(),
                type_arguments: hasher.finish(),
                display: instance_display(path, &shown),
            });
        }
    }
    requests.sort();
    requests.dedup();
    requests
}

/// How the instance of the generic `path` with the type arguments `shown` is shown to the user:
/// `lib/box::keep<Int>`, or `lib/box::Box.get<Int>` for an extension. The receiver is shown by
/// its type's name alone: its type arguments name the extension's own type parameters, which no
/// interface hash covers, so showing them would let a reused entry show a stale name.
fn instance_display(path: &prism_typed_tree::DeclarationPath, shown: &[String]) -> String {
    use prism_typed_tree::Type;
    let receiver = match &path.receiver {
        Some(Type::Struct { id, .. } | Type::Enum { id, .. }) => format!("{}.", id.name),
        Some(other) => format!("{}.", other),
        None => String::new(),
    };
    format!(
        "{}::{}{}<{}>",
        path.module,
        receiver,
        path.name,
        shown.join(", ")
    )
}

/// Looks up, or records, the instance of each of `requests` in `cache`, keyed by the generic's
/// current signature and Monomorphization Hash in the importing file's direct imports
/// `imports`. Without a cache every instance is one to instantiate.
fn instance_outcomes(
    cache: Option<&BuildCache>,
    imports: &[&FileAbi],
    requests: &[InstanceRequest],
) -> Vec<InstanceOutcome> {
    requests
        .iter()
        .filter_map(|request| {
            let generic = imports
                .iter()
                .filter(|abi| abi.module == request.module)
                .find_map(|abi| {
                    abi.generic_function_of(&request.name, request.receiver.as_deref())
                })?;
            let body_hash = generic.generic_body_hash()?;
            let reused = cache.is_some_and(|cache| {
                let key = cache.instance_key(request, &generic.interface_hash(), &body_hash);
                let reused = cache.has_instance(&key);
                if !reused {
                    cache.record_instance(&key);
                }
                reused
            });
            Some(InstanceOutcome {
                instance: request.display.clone(),
                reused,
            })
        })
        .collect()
}

/// Phase 1: extracts the ABI of every file of `graph` from its AST alone. No body is read.
pub(crate) fn extract_abis(
    graph: &DependencyGraph,
) -> Result<HashMap<PathBuf, FileAbi>, WorkspaceError> {
    // Phase 1a: the structs and enums each file exports, with their kinds and markers, read from
    // its AST alone.
    let exported_types: HashMap<&Path, HashMap<&str, NominalHeader>> = graph
        .nodes
        .iter()
        .map(|(path, node)| (path.as_path(), declared_types(&node.source_file, true)))
        .collect();

    // The binder of each file: its type names bound to the one direct import that exports them.
    let bind_import = |node: &SourceNode, name: &str| {
        let mut found = node.direct_imports.iter().filter_map(|dep| {
            let header = exported_types.get(dep.as_path())?.get(name)?;
            Some((header.clone(), module_path(graph.get_node(dep)?)))
        });
        match (found.next(), found.next()) {
            // A name exported by several imports stays unbound; Phase 2 reports `E4012`.
            (Some(binding), None) => Some(binding),
            _ => None,
        }
    };

    // Phase 1a, continued: the definition of every struct and enum of every file, private ones
    // included, bound as Phase 1b binds signature types (D3 of `codegen-aggregates`).
    let mut definitions: HashMap<(String, String), AbiTypeDefinition> = HashMap::new();
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
        let abi = FileAbi::extract_with_definitions(
            &node.source_file,
            &module_path(node),
            &bind,
            &definition,
        )
        .map_err(|err| {
            let source = fs::read_to_string(file_path).unwrap_or_default();
            WorkspaceError::Abi {
                path: file_path.clone(),
                source,
                error: err,
            }
        })?;
        abis.insert(file_path.clone(), abi);
    }
    Ok(abis)
}

/// Phase 2 of one file: checks the file at `file_path` of `graph` against the ABIs of its direct
/// imports only, never another file's body or typed tree (the deterministic hermetic builds
/// invariant), returning its typed tree and its type errors.
fn typecheck_file(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, FileAbi>,
    file_path: &Path,
) -> (TypedFile, Vec<prism_typechecker::TypeError>) {
    let node = graph.get_node(file_path).expect("node must exist in graph");
    let package_name = if node.package.is_empty() {
        None
    } else {
        Some(node.package.join("."))
    };

    let mut checker = prism_typechecker::TypeChecker::new(package_name, module_path(node));

    // Ingest FileAbi of all direct dependencies
    for dep_path in &node.direct_imports {
        if let Some(dep_abi) = abis.get(dep_path) {
            checker.ingest_file_abi(dep_abi);
        }
    }

    // Pass 1 (file-level forward declarations) and Pass 2 (function body checks)
    let tree = checker.check_module(&node.source_file);
    (tree, checker.errors)
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
    let abis = extract_abis(&graph)?;
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
        source: fs::read_to_string(&entry_path).unwrap_or_default(),
        errors,
    });
    Ok((tree, type_errors))
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
        };

        let uncached = check_path(&temp).unwrap();
        let cold = check_path_with(&temp, &options).unwrap();
        let warm = check_path_with(&temp, &options).unwrap();
        assert_eq!(uncached, cold);
        for file in &uncached.checked_files {
            assert_eq!(file.cache, CacheOutcome::Checked);
        }
        assert_eq!(uncached.checked_files[1].instances.len(), 1);
        assert!(!uncached.checked_files[1].instances[0].reused);

        let mut expected = uncached.clone();
        for file in &mut expected.checked_files {
            file.cache = CacheOutcome::Reused;
            for instance in &mut file.instances {
                instance.reused = true;
            }
        }
        assert_eq!(warm, expected);

        fs::remove_dir_all(&temp).ok();
    }
}
