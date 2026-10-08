//! Optional CLI directory scanner for batch package validation (`prism check <dir>`).
//!
//! Follows Section 2.3 of `package-resolution.md`.

use crate::error::ResolveError;
use crate::root::discover_root;
use std::path::{Path, PathBuf};

/// Scans all direct `.prism` files in a directory and verifies package homogeneity.
///
/// Filter criteria:
/// - Excludes subdirectories (subdirectories are distinct packages).
/// - Excludes hidden files (e.g. `.DS_Store`, `.git`).
/// - Includes only files ending with canonical `.prism` extension.
///
/// Package Homogeneity Validation:
/// - Every `.prism` file in `dir` must declare the exact same package statement.
/// - If any file declares a conflicting package, emits `E1003: PackageHeterogeneity`.
pub fn scan_directory(dir: &Path) -> Result<Vec<PathBuf>, ResolveError> {
    if !dir.is_dir() {
        return Err(ResolveError::Io {
            path: dir.to_path_buf(),
            message: format!("Path '{}' is not a directory", dir.display()),
        });
    }

    let entries = std::fs::read_dir(dir).map_err(|e| ResolveError::Io {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;

    let mut prism_files = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| ResolveError::Io {
            path: dir.to_path_buf(),
            message: e.to_string(),
        })?;
        let file_type = entry.file_type().map_err(|e| ResolveError::Io {
            path: entry.path(),
            message: e.to_string(),
        })?;

        // Exclude subdirectories
        if file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        // Exclude hidden files
        if name_str.starts_with('.') {
            continue;
        }

        // Include .prism files
        if name_str.ends_with(".prism") {
            prism_files.push(entry.path());
        }
    }

    prism_files.sort();

    if prism_files.is_empty() {
        return Ok(prism_files);
    }

    // Package Homogeneity Validation
    let mut file_data = Vec::new();
    for file in &prism_files {
        let content = std::fs::read_to_string(file).map_err(|e| ResolveError::Io {
            path: file.clone(),
            message: e.to_string(),
        })?;
        let ast = prism_parser::parse_source(&content).map_err(|e| ResolveError::Parse {
            path: file.clone(),
            message: e.to_string(),
        })?;

        let declared_pkg = ast
            .package
            .as_ref()
            .map(|p| p.package_string())
            .unwrap_or_default();

        file_data.push((file.clone(), ast, declared_pkg));
    }

    // 1. Homogeneity check: If any two files declare different packages, emit E1003.
    // The expected package is the one that matches dir suffix, or defaults to the first file's.
    let expected_pkg = file_data
        .iter()
        .find(|(f, ast, _)| discover_root(f, ast.package.as_ref()).is_ok())
        .map(|(_, _, pkg)| pkg.clone())
        .unwrap_or_else(|| file_data[0].2.clone());

    for (file, _, declared_pkg) in &file_data {
        if declared_pkg != &expected_pkg {
            return Err(ResolveError::PackageHeterogeneity {
                file: file.clone(),
                package_dir: dir.to_path_buf(),
                declared_package: declared_pkg.clone(),
                expected_package: expected_pkg.clone(),
            });
        }
    }

    // 2. Directory match check: verify that the homogeneous package matches dir
    for (file, ast, _) in &file_data {
        discover_root(file, ast.package.as_ref())?;
    }

    Ok(prism_files)
}
