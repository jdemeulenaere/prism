//! `prism-cli`: Developer command line interface and workspace driver for Prism.

pub mod cache;
pub mod diagnostics;
pub mod plan;
pub mod run;
pub mod watch;
pub mod workspace;

pub use cache::BuildCache;
pub use diagnostics::{display_path, format_abi_error, format_parse_error, format_resolve_error};
pub use run::{
    compile_program, compile_program_with, dump_clif, dump_instance_clif, dump_instance_mir,
    dump_layout, program_mir, run_file, CompiledProgram, RunOptions, Timings,
};
pub use watch::{run_watch, EditScript, ReloadOutcome, SessionEvent, WatchSession};
pub use workspace::{
    check_directory, check_directory_with, check_file, check_file_with, check_library, check_path,
    check_path_with, dump_composed_mir, dump_elaborated_mir, dump_library_module, dump_mir,
    dump_typed_tree, explain_cache, CacheOutcome, CheckOptions, CheckReport, CheckedFile,
    InstanceOutcome, MirDump, TypedTreeDump, WorkspaceError,
};

/// The compose transform, re-exported for the tests that check composed MIR.
pub use prism_compose_transform::compose_file;
