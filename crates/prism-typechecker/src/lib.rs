//! `prism-typechecker`: Core semantic analysis and static type checking for Prism.
//!
//! Enforces:
//! - Single-file compilation unit invariant: default private visibility, explicit parameter and
//!   return types on exported symbols, package boundary visibility rules (`E4007`), and checking
//!   each file against Phase 1 `FileAbi` imports only.
//! - Zero-cost memory invariant: stack references (`&T`, `&mut T`) and `Heap<T>` typing.
//! - First-class compose invariant: composable function context constraints (`E4008`).

mod builder;
mod captures;
pub mod checker;
pub mod error;
pub mod exhaustiveness;
pub mod inference;
pub mod intrinsics;
pub mod members;
mod recursion;
pub mod scope;
pub mod types;

pub use checker::TypeChecker;
pub use error::{
    closure_parameter_count_help, CaptureRefusal, MissingImplicitParameter, NoTextFormReason,
    TypeError,
};
pub use members::MemberScope;
#[cfg(any(test, feature = "test-support"))]
pub use members::{test_library_imports, test_members};
pub use prism_typed_tree::TypedFile;
pub use scope::{Mutability, ScopeStack, Symbol, SymbolKind};
pub use types::{
    type_from_abi, type_parameters_from_abi, NominalId, Type, TypeDef, TypeParameter,
    TypeRelations, BUILTIN_MODULE,
};
