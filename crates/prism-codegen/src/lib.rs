//! `prism-codegen`: the Tier 1 backend, which compiles the elaborated MIR of each file into native
//! code in memory with Cranelift and links the files of a program through a function table (the
//! `code-generation` spec; D1 of `codegen-core`).

pub mod dump;
pub mod entry;
pub mod error;
pub mod isa;
pub mod link;
mod lower;
pub mod program;
pub mod representation;
pub mod supported;
pub mod symbols;
pub mod unit;

#[cfg(test)]
mod lowering_tests;
#[cfg(test)]
mod test_support;

pub use dump::{dump_file, DumpError};
pub use entry::find_main;
pub use error::{BackendError, CodegenError, CodegenErrorKind};
pub use link::{FunctionTable, Linker};
pub use program::Program;
pub use supported::check_file;
pub use unit::CodeUnit;
