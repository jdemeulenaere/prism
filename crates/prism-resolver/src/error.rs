//! Error catalog and diagnostics for package resolution.
//!
//! Follows the error catalog specified in Section 4 of `PACKAGE_RESOLUTION_SPEC.md`.

use std::fmt;
use std::path::PathBuf;

/// Structured diagnostics for package and import resolution.
#[derive(Debug)]
pub enum ResolveError {
    /// E1001: Declared package path does not match the filesystem directory suffix.
    PackagePathMismatch {
        file: PathBuf,
        declared_package: String,
        expected_suffix: String,
        actual_dir: PathBuf,
    },
    /// E1002: Imported file cannot be found or does not match exact disk casing.
    FileNotFound {
        import_path: String,
        target_path: PathBuf,
    },
    /// E1003: Multiple files in the same directory declare different packages.
    PackageHeterogeneity {
        file: PathBuf,
        package_dir: PathBuf,
        declared_package: String,
        expected_package: String,
    },
    /// E1004: A dependency resolves to a different workspace root than the entry file.
    InconsistentWorkspaceRoot {
        file: PathBuf,
        file_root: PathBuf,
        workspace_root: PathBuf,
    },
    /// E1005: An import resolves to a path escaping the workspace boundary.
    WorkspaceBoundaryViolation {
        import_path: String,
        target_path: PathBuf,
        workspace_root: PathBuf,
    },
    /// E1006: An import path syntax is invalid (e.g. contains `..` or relative parent traversal).
    InvalidImportPath { import_path: String, reason: String },
    /// IO error during resolution.
    Io { path: PathBuf, message: String },
    /// Parse error while parsing a file.
    Parse { path: PathBuf, message: String },
}

impl ResolveError {
    /// Returns the standardized error code (e.g. `E1001`).
    pub fn code(&self) -> &'static str {
        match self {
            ResolveError::PackagePathMismatch { .. } => "E1001",
            ResolveError::FileNotFound { .. } => "E1002",
            ResolveError::PackageHeterogeneity { .. } => "E1003",
            ResolveError::InconsistentWorkspaceRoot { .. } => "E1004",
            ResolveError::WorkspaceBoundaryViolation { .. } => "E1005",
            ResolveError::InvalidImportPath { .. } => "E1006",
            ResolveError::Io { .. } => "E1000",
            ResolveError::Parse { .. } => "E1000",
        }
    }

    /// Returns a resolution suggestion according to Section 4 of the specification.
    pub fn suggestion(&self) -> Option<&'static str> {
        match self {
            ResolveError::PackagePathMismatch { .. } => {
                Some("Rename directory to match package or update package statement.")
            }
            ResolveError::FileNotFound { .. } => {
                Some("Check spelling or verify target file exists at root.")
            }
            ResolveError::PackageHeterogeneity { .. } => {
                Some("Ensure all files in a folder share the same package.")
            }
            ResolveError::InconsistentWorkspaceRoot { .. } => {
                Some("Ensure all project dependencies reside within the same workspace root.")
            }
            ResolveError::WorkspaceBoundaryViolation { .. } => {
                Some("Remove symlink or relative path escaping the workspace.")
            }
            ResolveError::InvalidImportPath { .. } => {
                Some("Use sibling import 'name' or root-anchored import '/path'.")
            }
            _ => None,
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::PackagePathMismatch {
                file,
                declared_package,
                expected_suffix,
                actual_dir,
            } => {
                write!(
                    f,
                    "{}: Package path '{}' does not match directory '{}' (expected directory suffix: '{}') in '{}'",
                    self.code(),
                    declared_package,
                    actual_dir.display(),
                    expected_suffix,
                    file.display()
                )
            }
            ResolveError::FileNotFound {
                import_path,
                target_path,
            } => {
                write!(
                    f,
                    "{}: Cannot resolve import '{}': file '{}' not found",
                    self.code(),
                    import_path,
                    target_path.display()
                )
            }
            ResolveError::PackageHeterogeneity {
                file,
                package_dir,
                declared_package,
                expected_package,
            } => {
                write!(
                    f,
                    "{}: File '{}' in package directory '{}' declares conflicting package '{}' (expected '{}')",
                    self.code(),
                    file.display(),
                    package_dir.display(),
                    declared_package,
                    expected_package
                )
            }
            ResolveError::InconsistentWorkspaceRoot {
                file,
                file_root,
                workspace_root,
            } => {
                write!(
                    f,
                    "{}: File '{}' resolves to root '{}', conflicting with workspace root '{}'",
                    self.code(),
                    file.display(),
                    file_root.display(),
                    workspace_root.display()
                )
            }
            ResolveError::WorkspaceBoundaryViolation {
                import_path,
                target_path,
                workspace_root,
            } => {
                write!(
                    f,
                    "{}: Import '{}' resolves to path outside workspace root '{}': '{}'",
                    self.code(),
                    import_path,
                    workspace_root.display(),
                    target_path.display()
                )
            }
            ResolveError::InvalidImportPath {
                import_path,
                reason,
            } => {
                write!(
                    f,
                    "{}: Invalid import path '{}': {}",
                    self.code(),
                    import_path,
                    reason
                )
            }
            ResolveError::Io { path, message } => {
                write!(f, "IO error at '{}': {}", path.display(), message)
            }
            ResolveError::Parse { path, message } => {
                write!(f, "Parse error in '{}': {}", path.display(), message)
            }
        }
    }
}

impl std::error::Error for ResolveError {}
