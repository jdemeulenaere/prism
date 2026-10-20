//! `prism-typed-tree`: the type checker's output for one file.
//!
//! The typed tree records every decision the type checker makes about a body (what each name
//! refers to, the type of every expression, which argument fills which parameter, and every
//! implicit move, copy, borrow and conversion), so that later stages never re-derive one. See
//! the `typed-tree` spec, and [`tree`] for the invariants the tree carries.

pub mod arena;
pub mod dump;
pub mod instantiation;
pub mod tree;
pub mod types;
pub mod validate;

pub use arena::{Arena, ArenaId, BodyId, ExpressionId, LocalId, StatementId};
pub use tree::*;
pub use types::{
    IntrinsicStruct, NominalId, NominalMarker, ParameterKind, Type, TypeDef, TypeParameter,
    TypeTable, BUILTIN_MODULE, COMPOSE_RUNTIME_MODULE,
};
pub use validate::foreign_parameters;
