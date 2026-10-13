//! Workspace and multi-file project driver for Prism CLI.

use crate::diagnostics;
use prism_abi::{AbiError, FileAbi};
use prism_parser::ParseError;
use prism_resolver::{
    scan_directory, DependencyGraph, ResolveError,
};
use std::fs;
use std::path::{Path, PathBuf};

/// Information about a verified source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFile {
    pub path: PathBuf,
    pub package: String,
    pub exported_symbols: usize,
    pub abi_hash: String,
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
    Cycle {
        cycle: Vec<PathBuf>,
    },
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
            WorkspaceError::Parse { path, source, error } => {
                diagnostics::format_parse_error(path, Some(source), error)
            }
            WorkspaceError::Abi { path, source, error } => {
                diagnostics::format_abi_error(path, Some(source), error)
            }
            WorkspaceError::Cycle { cycle } => diagnostics::format_cycle_error(cycle),
            WorkspaceError::Io { path, message } => {
                format!("error[E1000]: IO error at '{}': {}\n", path.display(), message)
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

    let graph = DependencyGraph::from_entry(entry_file)?;

    // Check for cycles
    if graph.is_cyclic() {
        let cycle = extract_cycle_path(&graph);
        return Err(WorkspaceError::Cycle { cycle });
    }

    validate_and_extract_abi(&graph)
}

/// Checks a directory (either a package directory or workspace root), recursively
/// scanning all subdirectories and asserting package homogeneity.
pub fn check_directory(dir: &Path) -> Result<CheckReport, WorkspaceError> {
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

    if graph.is_cyclic() {
        let cycle = extract_cycle_path(&graph);
        return Err(WorkspaceError::Cycle { cycle });
    }

    validate_and_extract_abi(&graph)
}

/// Universal check dispatcher accepting either a file or a directory.
pub fn check_path(path: &Path) -> Result<CheckReport, WorkspaceError> {
    if path.is_file() {
        check_file(path)
    } else if path.is_dir() {
        check_directory(path)
    } else {
        Err(WorkspaceError::InvalidTarget(format!(
            "Path '{}' does not exist or is not accessible",
            path.display()
        )))
    }
}

/// Helper to recursively discover directories that contain direct `.prism` files.
fn collect_dirs_with_prism_files(dir: &Path, dirs: &mut Vec<PathBuf>) -> Result<(), WorkspaceError> {
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

/// Extracts a representative cycle path from a cyclic dependency graph.
fn extract_cycle_path(graph: &DependencyGraph) -> Vec<PathBuf> {
    for cluster in graph.sccs() {
        if cluster.files.len() > 1 {
            let mut cycle = cluster.files.clone();
            if let Some(first) = cycle.first().cloned() {
                cycle.push(first);
            }
            return cycle;
        } else if let Some(single) = cluster.files.first() {
            if let Some(node) = graph.get_node(single) {
                if node.direct_imports.contains(single) {
                    return vec![single.clone(), single.clone()];
                }
            }
        }
    }
    Vec::new()
}

/// Validates AST and extracts ABI for all files in topological order.
fn validate_and_extract_abi(graph: &DependencyGraph) -> Result<CheckReport, WorkspaceError> {
    let mut checked_files = Vec::new();

    for cluster in graph.topological_order() {
        for file_path in &cluster.files {
            let node = graph.get_node(file_path).expect("node must exist in graph");
            let abi = FileAbi::extract(&node.source_file).map_err(|err| {
                let source = fs::read_to_string(file_path).unwrap_or_default();
                WorkspaceError::Abi {
                    path: file_path.clone(),
                    source,
                    error: err,
                }
            })?;

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
            });
        }
    }

    Ok(CheckReport {
        workspace_root: graph.workspace_root.clone(),
        total_files: checked_files.len(),
        checked_files,
    })
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
    fn test_check_file_cycle_detected() {
        let temp = create_temp_dir("cycle");
        let pkg_dir = temp.join("p");
        fs::create_dir_all(&pkg_dir).unwrap();

        let a = pkg_dir.join("a.prism");
        let b = pkg_dir.join("b.prism");

        fs::write(&a, "package p\nimport b\npublic fun fa(): Int = 1\n").unwrap();
        fs::write(&b, "package p\nimport a\npublic fun fb(): Int = 2\n").unwrap();

        let err = check_file(&a).unwrap_err();
        match err {
            WorkspaceError::Cycle { cycle } => {
                assert!(cycle.len() >= 2);
                let rendered = diagnostics::format_cycle_error(&cycle);
                assert!(rendered.contains("error[E1007]: cyclic dependency detected"));
            }
            other => panic!("expected Cycle error, got {:?}", other),
        }

        fs::remove_dir_all(&temp).ok();
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
}
