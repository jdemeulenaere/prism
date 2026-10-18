//! `prism-cli`: Developer command line interface and workspace driver for Prism.

pub mod cache;
pub mod diagnostics;
pub mod run;
pub mod workspace;

pub use cache::BuildCache;
pub use diagnostics::{display_path, format_abi_error, format_parse_error, format_resolve_error};
pub use run::{
    compile_program, dump_clif, dump_instance_clif, dump_instance_mir, dump_layout, run_file,
    CompiledProgram, RunOptions, Timings,
};
pub use workspace::{
    check_directory, check_directory_with, check_file, check_file_with, check_path,
    check_path_with, dump_elaborated_mir, dump_mir, dump_typed_tree, CacheOutcome, CheckOptions,
    CheckReport, CheckedFile, InstanceOutcome, MirDump, TypedTreeDump, WorkspaceError,
};
