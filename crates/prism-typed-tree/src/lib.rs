//! `prism-typed-tree`: the type checker's output for one file.
//!
//! The typed tree records every decision the type checker makes about a body (what each name
//! refers to, the type of every expression, which argument fills which parameter, and every
//! implicit move, copy, borrow and conversion), so that later stages never re-derive one. See
//! the `typed-tree` spec, and [`tree`] for the invariants the tree carries.

pub mod arena;
pub mod dump;
pub mod tree;
pub mod types;
pub mod validate;

pub use arena::{Arena, ArenaId, BodyId, ExpressionId, LocalId, StatementId};
pub use tree::*;
pub use types::{NominalId, Type, TypeDef, BUILTIN_MODULE};
