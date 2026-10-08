//! `prism-ast`: Abstract Syntax Tree definitions for the Prism language.
//!
//! Conforms strictly to the architectural invariants documented in `docs/INVARIANTS.md`
//! and syntax specifications in `docs/specs/syntax.md`.

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
