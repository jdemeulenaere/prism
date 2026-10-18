//! `prism-codegen`: the Tier 1 backend, which compiles the elaborated MIR of each file into native
//! code in memory with Cranelift and links the files of a program through a function table (the
//! `code-generation` spec; D1 of `codegen-core`).

pub mod entry;
pub mod error;
pub mod isa;
pub mod representation;
pub mod supported;
pub mod symbols;

#[cfg(test)]
mod test_support;

pub use entry::find_main;
pub use error::{BackendError, CodegenError, CodegenErrorKind};
pub use supported::check_file;
