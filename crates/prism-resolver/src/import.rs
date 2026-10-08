//! Sibling and root-anchored import resolution.
//!
//! Follows Section 2.2 of `PACKAGE_RESOLUTION_SPEC.md` and Invariant 2.

use crate::error::ResolveError;
use crate::root::{normalize_path, verify_disk_casing_exists};
use prism_ast::ImportDecl;
use std::path::{Path, PathBuf};

/// Resolves an import statement to a concrete `.prism` file target on disk.
///
/// Rules:
/// - Root-anchored import (`is_root == true`): Target is `<workspace_root>/<path>.prism`.
/// - Sibling import (`is_root == false`): Target is `<parent(source_file)>/<name>.prism`.
/// - Rejects parent navigation (`..` or `.`) with `E1006: InvalidImportPath`.
/// - Rejects targets outside the workspace with `E1005: WorkspaceBoundaryViolation`.
/// - Verifies case-exact existence with `E1002: FileNotFound`.
pub fn resolve_import(
    source_file: &Path,
    import: &ImportDecl,
    workspace_root: &Path,
) -> Result<PathBuf, ResolveError> {
    let import_str = import.path_string();

    // 1. Validation of import path components
    if import.path.is_empty() {
        return Err(ResolveError::InvalidImportPath {
            import_path: import_str,
            reason: "import path cannot be empty".to_string(),
        });
    }

    for component in &import.path {
        if component.is_empty()
            || component == ".."
            || component == "."
            || component.contains('/')
            || component.contains('\\')
        {
            return Err(ResolveError::InvalidImportPath {
                import_path: import_str,
                reason: if component.is_empty() {
                    "empty import path components are forbidden".to_string()
                } else {
                    "relative parent traversal is forbidden".to_string()
                },
            });
        }
    }

    // Sibling import validation: exactly one identifier
    if !import.is_root && import.path.len() != 1 {
        return Err(ResolveError::InvalidImportPath {
            import_path: import_str,
            reason: "sibling imports must specify a single identifier".to_string(),
        });
    }

    // 2. Base directory and candidate path construction
    let (base_dir, relative_dirs, file_name) = if import.is_root {
        let (dirs, last) = import.path.split_at(import.path.len() - 1);
        (
            workspace_root.to_path_buf(),
            dirs,
            format!("{}.prism", last[0]),
        )
    } else {
        let parent = source_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        (parent, &[][..], format!("{}.prism", import.path[0]))
    };

    let mut candidate = base_dir.clone();
    for dir in relative_dirs {
        candidate.push(dir);
    }
    candidate.push(&file_name);
    let normalized_candidate = normalize_path(&candidate);

    // Disallow self-imports (ADV-04)
    if normalized_candidate == normalize_path(source_file) {
        return Err(ResolveError::InvalidImportPath {
            import_path: import_str,
            reason: "a file cannot import itself".to_string(),
        });
    }

    // 3. Workspace Jail Enforcement:
    // If the candidate file exists or base_dir exists, verify against workspace_root
    if normalized_candidate.exists() && workspace_root.exists() {
        if let (Ok(canon_target), Ok(canon_root)) = (
            normalized_candidate.canonicalize(),
            workspace_root.canonicalize(),
        ) {
            if !canon_target.starts_with(&canon_root) {
                return Err(ResolveError::WorkspaceBoundaryViolation {
                    import_path: import_str,
                    target_path: canon_target,
                    workspace_root: workspace_root.to_path_buf(),
                });
            }
        }
    }

    // 4. Candidate existence and regular file check
    if !normalized_candidate.exists() || !normalized_candidate.is_file() {
        return Err(ResolveError::FileNotFound {
            import_path: import_str,
            target_path: normalized_candidate,
        });
    }

    // 5. Strict Case-Sensitivity Enforcement on disk
    if base_dir.exists() {
        let mut curr = base_dir;
        for dir in relative_dirs {
            if !verify_disk_casing_exists(&curr, dir) {
                return Err(ResolveError::FileNotFound {
                    import_path: import_str,
                    target_path: normalized_candidate,
                });
            }
            curr = curr.join(dir);
        }
        if !verify_disk_casing_exists(&curr, &file_name) {
            return Err(ResolveError::FileNotFound {
                import_path: import_str,
                target_path: normalized_candidate,
            });
        }
    }

    Ok(normalized_candidate)
}

/// Computes the unverified target candidate path for an import declaration.
pub fn candidate_target_path(
    source_file: &Path,
    import: &ImportDecl,
    workspace_root: &Path,
) -> Result<PathBuf, ResolveError> {
    let import_str = import.path_string();

    if import.path.is_empty() {
        return Err(ResolveError::InvalidImportPath {
            import_path: import_str,
            reason: "import path cannot be empty".to_string(),
        });
    }

    for component in &import.path {
        if component.is_empty()
            || component == ".."
            || component == "."
            || component.contains('/')
            || component.contains('\\')
        {
            return Err(ResolveError::InvalidImportPath {
                import_path: import_str,
                reason: if component.is_empty() {
                    "empty import path components are forbidden".to_string()
                } else {
                    "relative parent traversal is forbidden".to_string()
                },
            });
        }
    }

    if !import.is_root && import.path.len() != 1 {
        return Err(ResolveError::InvalidImportPath {
            import_path: import_str,
            reason: "sibling imports must specify a single identifier".to_string(),
        });
    }

    let (base_dir, relative_dirs, file_name) = if import.is_root {
        let (dirs, last) = import.path.split_at(import.path.len() - 1);
        (
            workspace_root.to_path_buf(),
            dirs,
            format!("{}.prism", last[0]),
        )
    } else {
        let parent = source_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        (parent, &[][..], format!("{}.prism", import.path[0]))
    };

    let mut candidate = base_dir;
    for dir in relative_dirs {
        candidate.push(dir);
    }
    candidate.push(&file_name);
    Ok(normalize_path(&candidate))
}
