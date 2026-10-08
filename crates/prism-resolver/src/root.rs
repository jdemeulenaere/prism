//! Pure-path workspace root discovery algorithm and path normalization.
//!
//! Follows Section 2.1 of `PACKAGE_RESOLUTION_SPEC.md` and Invariants 2 and 3.

use crate::error::ResolveError;
use prism_ast::PackageDecl;
use std::path::{Path, PathBuf};

/// Representation of a parsed path with normalized separators and resolved relative components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPath {
    pub drive: String,
    pub has_root: bool,
    pub segments: Vec<String>,
    pub uses_backslash: bool,
}

impl NormalizedPath {
    /// Parses and normalizes a path across POSIX and Windows.
    pub fn parse(path: &Path) -> Self {
        let path_str = path.to_string_lossy();
        let uses_backslash = path_str.contains('\\');
        let standardized = path_str.replace('\\', "/");

        let mut drive = String::new();
        let mut rest = standardized.as_str();

        // Check Windows drive prefix (e.g. "C:")
        if rest.len() >= 2 {
            let b0 = rest.as_bytes()[0];
            let b1 = rest.as_bytes()[1];
            if b0.is_ascii_alphabetic() && b1 == b':' {
                drive = rest[..2].to_string();
                rest = &rest[2..];
            }
        }

        let has_root = rest.starts_with('/');
        let raw_segments = rest.split('/').filter(|s| !s.is_empty() && *s != ".");
        let mut segments = Vec::new();

        for seg in raw_segments {
            if seg == ".." {
                if !segments.is_empty() {
                    segments.pop();
                }
            } else {
                segments.push(seg.to_string());
            }
        }

        Self {
            drive,
            has_root,
            segments,
            uses_backslash,
        }
    }

    /// Converts back to a PathBuf preserving original drive prefix and root separators.
    pub fn to_path_buf(&self) -> PathBuf {
        let sep = if self.uses_backslash { "\\" } else { "/" };
        let mut s = String::new();
        if !self.drive.is_empty() {
            s.push_str(&self.drive);
        }
        if self.has_root {
            s.push_str(sep);
        }
        s.push_str(&self.segments.join(sep));
        PathBuf::from(s)
    }
}

/// Normalizes a path by standardizing separators and resolving `.` and `..` components lexically.
pub fn normalize_path(path: &Path) -> PathBuf {
    NormalizedPath::parse(path).to_path_buf()
}

/// Verifies whether an entry with exact case-matching `segment` exists inside `parent` on disk.
///
/// On case-insensitive filesystems (such as macOS APFS and Windows NTFS), standard existence checks
/// ignore case. This function reads directory entries directly to enforce strict case sensitivity.
pub fn verify_disk_casing_exists(parent: &Path, segment: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.file_name() == segment {
                return true;
            }
        }
    }
    false
}

/// Discovers the workspace root from a source file path and its parsed package declaration.
pub fn discover_root(
    source_path: &Path,
    package_decl: Option<&PackageDecl>,
) -> Result<PathBuf, ResolveError> {
    let segments: Vec<String> = match package_decl {
        Some(pkg) => pkg.path.clone(),
        None => Vec::new(),
    };
    discover_root_from_segments(source_path, &segments)
}

/// Discovers the workspace root from a source file path and package path segments.
///
/// Algorithm per Section 2.1 of `PACKAGE_RESOLUTION_SPEC.md`:
/// 1. Path normalization and absolute directory extraction (`dir = parent(P)`).
/// 2. If package is empty (`n == 0`), workspace root is `dir`.
/// 3. If package is non-empty (`n > 0`):
///    - If `n > m` (more package segments than directory segments), emit `E1001: PackagePathMismatch`.
///    - Compare trailing `n` segments against declared package segments (case-sensitive).
///    - Strip trailing `n` segments from `dir` to obtain `candidate_root`.
/// 4. Strict case-sensitivity verification on disk:
///    - If `candidate_root` exists on disk, verify every trailing segment matches the actual disk entry case.
pub fn discover_root_from_segments(
    source_path: &Path,
    package_segments: &[impl AsRef<str>],
) -> Result<PathBuf, ResolveError> {
    let norm = NormalizedPath::parse(source_path);
    let normalized = norm.to_path_buf();

    // Extract directory segments (all segments excluding the filename)
    let dir_segments = if norm.segments.is_empty() {
        Vec::new()
    } else {
        norm.segments[..norm.segments.len() - 1].to_vec()
    };

    let dir_norm = NormalizedPath {
        drive: norm.drive.clone(),
        has_root: norm.has_root,
        segments: dir_segments.clone(),
        uses_backslash: norm.uses_backslash,
    };
    let dir = dir_norm.to_path_buf();

    let n = package_segments.len();
    let m = dir_segments.len();
    let declared_pkg_str = package_segments
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(".");
    let expected_suffix = package_segments
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join("/");

    if n > m {
        return Err(ResolveError::PackagePathMismatch {
            file: normalized,
            declared_package: declared_pkg_str,
            expected_suffix,
            actual_dir: dir,
        });
    }

    // Compare trailing n segments against package_segments
    let start_idx = m - n;
    for (i, pkg_seg) in package_segments.iter().enumerate() {
        if dir_segments[start_idx + i] != pkg_seg.as_ref() {
            return Err(ResolveError::PackagePathMismatch {
                file: normalized,
                declared_package: declared_pkg_str,
                expected_suffix,
                actual_dir: dir,
            });
        }
    }

    // Compute root by stripping n segments from dir_segments
    let root_segments = dir_segments[..start_idx].to_vec();
    let root_norm = NormalizedPath {
        drive: norm.drive,
        has_root: norm.has_root,
        segments: root_segments,
        uses_backslash: norm.uses_backslash,
    };
    let candidate_root = root_norm.to_path_buf();

    // Strict Case-Sensitivity Enforcement on disk:
    // If candidate_root exists on disk, verify every segment matches on-disk casing exactly.
    if candidate_root.exists() {
        let mut curr = candidate_root.clone();
        for pkg_seg in package_segments {
            let seg_str = pkg_seg.as_ref();
            if !verify_disk_casing_exists(&curr, seg_str) {
                return Err(ResolveError::PackagePathMismatch {
                    file: normalized,
                    declared_package: declared_pkg_str,
                    expected_suffix,
                    actual_dir: dir,
                });
            }
            curr = curr.join(seg_str);
        }
    }

    Ok(candidate_root)
}
