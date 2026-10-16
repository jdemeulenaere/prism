//! `prism-typechecker`: Core semantic analysis and static type checking for Prism.
//!
//! Enforces:
//! - Single-file compilation unit invariant: default private visibility, explicit parameter and
//!   return types on exported symbols, package boundary visibility rules (`E4007`), and checking
//!   each file against Phase 1 `FileAbi` imports only.
//! - Zero-cost memory invariant: stack references (`&T`, `&mut T`) and `Heap<T>` typing.
//! - First-class compose invariant: composable function context constraints (`E4008`).

pub mod checker;
pub mod error;
pub mod scope;
pub mod types;

pub use checker::TypeChecker;
pub use error::TypeError;
pub use scope::{Mutability, ScopeStack, Symbol, SymbolKind};
pub use types::Type;
