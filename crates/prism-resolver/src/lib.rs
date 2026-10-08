//! `prism-resolver`: Zero-config package resolution, zero-marker workspace root discovery,
//! import resolution, and dependency graph construction for Prism.
//!
//! Conforms strictly to:
//! - `docs/specs/package-resolution-spec.md`
//! - `docs/specs/compilation-and-abi-spec.md`
//! - `docs/INVARIANTS.md` (Invariants 1, 2, and 3)

pub mod error;
pub mod graph;
pub mod import;
pub mod root;
pub mod scanner;

pub use error::ResolveError;
pub use graph::{DependencyGraph, SccCluster, SourceNode};
pub use import::{candidate_target_path, resolve_import};
pub use root::{
    discover_root, discover_root_from_segments, normalize_path, verify_disk_casing_exists,
};
pub use scanner::scan_directory;
