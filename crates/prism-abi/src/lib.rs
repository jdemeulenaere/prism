//! `prism-abi`: Surface signature extraction & deterministic ABI hashing.
//!
//! Enforces the single-file compilation unit invariant and the `compilation-and-abi` spec:
//! - Default visibility is `private`.
//! - Private symbols and non-generic function bodies are excluded from the exported ABI.
//! - Exported (`public` and `package`) declarations must declare explicit types.
//! - Computes deterministic ABI hash for incremental compilation avoidance.

pub mod error;
pub mod hasher;
pub mod signature;

pub use error::AbiError;
pub use hasher::DeterministicHasher;
pub use signature::{ExportedSymbol, FileAbi};
