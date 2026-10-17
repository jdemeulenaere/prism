//! `prism-mir`: the mid-level intermediate representation of a file that typechecks.
//!
//! `lower_file` lowers the typed tree of a file without type errors to MIR: one control-flow
//! graph per function, top-level `val`, parameter default and closure, over typed locals and
//! places, with explicit storage markers, drops, branches and calls. The borrow checker and the
//! code generator work from it without re-deriving any decision of the type checker. See the
//! `mir` spec, [`mir`] for the data model and its invariants, and [`dump`] for the text form
//! `prism dump-mir` prints.

pub mod dump;
pub mod mir;
pub mod types;
pub mod validate;

pub use mir::*;
pub use types::{mir_range_definition, mir_type_table, needs_drop, projection_type};
