//! `prism-typechecker`: Core semantic analysis and static type checking for Prism.
//!
//! Enforces:
//! - Invariant 1: Default private visibility, explicit parameter and return types on exported symbols.
//! - Invariant 2: Package boundary visibility rules (`E4007`).
//! - Invariant 4: Single-owner value semantics, stack references (`&T`, `&mut T`), `Heap<T>`.
//! - Invariant 5: Composable function context constraints (`E4008`).
//! - Invariant 6: File as independent compilation unit via Phase 1 `FileAbi` imports.

pub mod checker;
pub mod error;
pub mod scope;
pub mod types;

pub use checker::TypeChecker;
pub use error::TypeError;
pub use scope::{Mutability, ScopeStack, Symbol, SymbolKind};
pub use types::Type;
