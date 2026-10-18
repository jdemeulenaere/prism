//! `prism-resolver`: Zero-config package resolution, zero-marker workspace root discovery,
//! import resolution, and dependency graph construction for Prism.
//!
//! Conforms strictly to:
//! - `openspec/specs/package-resolution/spec.md`
//! - `openspec/specs/compilation-and-abi/spec.md`
//! - `openspec/specs/system-invariants/spec.md`

pub mod error;
pub mod graph;
pub mod import;
pub mod root;
pub mod scanner;

pub use error::ResolveError;
pub use graph::{DependencyGraph, SccCluster, SourceNode};
pub use import::{candidate_target_path, resolve_import, resolve_import_with};
pub use root::{
    absolute_path, discover_root, discover_root_from_segments, discover_root_from_segments_with,
    discover_root_with, normalize_path, verify_disk_casing_exists, DirectoryListings,
};
pub use scanner::scan_directory;
