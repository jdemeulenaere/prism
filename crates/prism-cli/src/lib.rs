//! `prism-cli`: Developer command line interface and workspace driver for Prism.

pub mod diagnostics;
pub mod workspace;

pub use diagnostics::{format_abi_error, format_parse_error, format_resolve_error};
pub use workspace::{
    check_directory, check_file, check_path, dump_typed_tree, CheckReport, CheckedFile,
    TypedTreeDump, WorkspaceError,
};
