//! `prism-ast`: Abstract Syntax Tree definitions for the Prism language.
//!
//! Conforms strictly to the architectural invariants documented in `openspec/specs/system-invariants/spec.md`
//! and syntax specifications in `openspec/specs/syntax/spec.md`.

pub mod decl;
pub mod expr;
pub mod module;
pub mod span;
pub mod types;

pub use decl::*;
pub use expr::*;
pub use module::*;
pub use span::*;
pub use types::*;
