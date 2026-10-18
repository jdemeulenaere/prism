//! `prism-codegen`: the Tier 1 backend, which compiles the elaborated MIR of each file into native
//! code in memory with Cranelift and links the files of a program through a function table (the
//! `code-generation` spec; D1 of `codegen-core`).

pub mod artifact;
pub mod contract;
pub mod dump;
pub mod entry;
mod equality;
pub mod error;
pub mod externals;
pub mod glue;
pub mod instances;
pub mod isa;
pub mod layout;
pub mod link;
mod lower;
pub mod program;
pub mod reload;
pub mod representation;
pub mod supported;
pub mod symbols;
pub mod unit;
mod variables;

#[cfg(test)]
mod lowering_tests;
#[cfg(test)]
mod reload_tests;
#[cfg(test)]
mod test_support;

pub use artifact::UnitArtifact;
pub use dump::{dump_file, dump_instances, DumpError, InstanceDumpError};
pub use entry::find_main;
pub use error::{BackendError, CodegenError, CodegenErrorKind};
pub use instances::{
    build_instance_unit, check_instances, collect_instances, plan_instances, InstanceError,
    InstanceUnit, PlannedInstance,
};
pub use layout::{dump_layouts, Layout, LayoutTable};
pub use link::{FunctionTable, Linker, SlotChange, ValueKeys, ValueTable};
pub use program::{Program, RunningProgram, PROGRAM_STACK_SIZE};
pub use reload::{LiveProgram, PreparedReload, ReloadWrites, SlotKind};
pub use supported::check_file;
pub use unit::CodeUnit;
