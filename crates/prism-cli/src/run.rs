//! `prism run`, `prism dump-clif` and `prism dump-layout`: compiling a program with the Tier 1 backend and running it
//! (the `execution` spec; D14, D20 and D23 of `codegen-core`).
//!
//! `prism run` checks the entry file and everything it transitively imports as `prism check`
//! does, in topological order and stopping at the first file with errors, without the build
//! cache: a cached verdict carries no MIR. It then finds the entry file's `main`, runs the
//! backend's support check over every file, lowers each file into its own code unit and links
//! the units in memory before calling `main`.

#![allow(clippy::result_large_err)]

use crate::workspace::{
    assert_workspace_boundary, elaborate_file, extract_abis, validate_entry_file, WorkspaceError,
};
use prism_codegen::{check_file, dump_file, dump_layouts, find_main, DumpError, Linker, Program};
use prism_mir::MirFile;
use prism_resolver::DependencyGraph;
use prism_typed_tree::{DeclarationPath, Item, NominalId};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How `prism run` runs a program.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Whether to print the time of each stage to stderr before `main` runs.
    pub timings: bool,
    /// Whether the runtime tracks every allocation and checks, when `main` returns, that none is
    /// still live and none was freed invalidly (`--check-allocations`, D13 of
    /// `runtime-strings-and-drops`).
    pub check_allocations: bool,
    /// The arguments after `--`, passed to the program.
    pub arguments: Vec<String>,
    /// When the `prism` process started, from which the total time to `main` counts.
    pub started: Instant,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timings: false,
            check_allocations: false,
            arguments: Vec::new(),
            started: Instant::now(),
        }
    }
}

/// The time each stage of `prism run` took.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    /// Resolution, parsing, Phase 1 and Phase 2 of every file.
    pub front_end: Duration,
    /// The entry and support checks, lowering and Cranelift's compilation.
    pub code_generation: Duration,
    /// Finalizing the load and filling the function table.
    pub linking: Duration,
}

/// A program compiled and linked in memory, ready to run.
pub struct CompiledProgram {
    pub program: Program,
    /// The entry file's `main`.
    pub main: DeclarationPath,
    pub timings: Timings,
}

/// Compiles the program whose entry file is `entry`: every file it transitively imports,
/// checked and lowered in topological order.
pub fn compile_program(entry: &Path) -> Result<CompiledProgram, WorkspaceError> {
    let start = Instant::now();
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entry(entry)?;
    assert_workspace_boundary(&graph)?;
    let abis = extract_abis(&graph)?;
    let entry_path = entry_in_graph(&graph, entry)?;
    let mut files: Vec<(PathBuf, MirFile)> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let mir = elaborate_file(&graph, &abis, path)?;
            files.push((path.clone(), mir));
        }
    }
    let front_end = start.elapsed();

    let start = Instant::now();
    let codegen_error = |path: &Path, errors| WorkspaceError::Codegen {
        path: path.to_path_buf(),
        source: fs::read_to_string(path).unwrap_or_default(),
        errors,
    };
    let (_, entry_mir) = files
        .iter()
        .find(|(path, _)| *path == entry_path)
        .ok_or_else(|| WorkspaceError::Io {
            path: entry.to_path_buf(),
            message: "the entry file is not in its own dependency graph".to_string(),
        })?;
    let main = find_main(entry_mir).map_err(|error| codegen_error(&entry_path, vec![error]))?;
    for (path, mir) in &files {
        check_file(mir).map_err(|errors| codegen_error(path, errors))?;
    }
    let mut linker = Linker::new().map_err(WorkspaceError::Backend)?;
    for (_, mir) in &files {
        linker.add_file(mir).map_err(WorkspaceError::Backend)?;
    }
    let code_generation = start.elapsed();

    let start = Instant::now();
    let program = linker.link().map_err(WorkspaceError::Backend)?;
    let linking = start.elapsed();
    Ok(CompiledProgram {
        program,
        main,
        timings: Timings {
            front_end,
            code_generation,
            linking,
        },
    })
}

/// Compiles and runs the program whose entry file is `entry`, returning its exit status: 0 when
/// `main` returns, or 102 when the allocation check finds allocations still live. A runtime
/// error ends the process with status 101 instead of returning, and an invalid free under the
/// allocation check with status 102.
pub fn run_file(entry: &Path, options: &RunOptions) -> Result<i32, WorkspaceError> {
    let compiled = compile_program(entry)?;
    if options.timings {
        let milliseconds = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let timings = compiled.timings;
        eprintln!(
            "timing: front end {:.3} ms",
            milliseconds(timings.front_end)
        );
        eprintln!(
            "timing: code generation {:.3} ms",
            milliseconds(timings.code_generation)
        );
        eprintln!("timing: linking {:.3} ms", milliseconds(timings.linking));
        eprintln!(
            "timing: total to main {:.3} ms",
            milliseconds(options.started.elapsed())
        );
    }
    compiled
        .program
        .run(
            &compiled.main,
            options.arguments.clone(),
            options.check_allocations,
        )
        .map_err(WorkspaceError::Backend)
}

/// The Cranelift IR of the entry file `entry`, compiled alone against the ABIs of its direct
/// imports as `prism dump-mir --elaborated` does, or its errors of any stage.
pub fn dump_clif(entry: &Path) -> Result<String, WorkspaceError> {
    let dump = crate::workspace::dump_elaborated_mir(entry)?;
    dump_file(&dump.mir).map_err(|error| match error {
        DumpError::Unsupported(errors) => WorkspaceError::Codegen {
            path: entry.to_path_buf(),
            source: fs::read_to_string(entry).unwrap_or_default(),
            errors,
        },
        DumpError::Backend(error) => WorkspaceError::Backend(error),
    })
}

/// The layouts of the entry file `entry`, as `prism dump-layout` prints them (D18 of
/// `codegen-aggregates`): its own structs and enums in declaration order, then every other
/// struct, enum and nullable type its compiled bodies hold. The file is compiled alone against
/// the ABIs of its direct imports, as for `prism dump-clif`; a file with type or borrow errors
/// returns them instead.
pub fn dump_layout(entry: &Path) -> Result<String, WorkspaceError> {
    let (tree, mir) = crate::workspace::elaborate_entry(entry)?;
    let declared: Vec<NominalId> = tree
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(item) => Some(item.id.clone()),
            Item::Enum(item) => Some(item.id.clone()),
            Item::Function(_) | Item::Value(_) => None,
        })
        .collect();
    let pointer = prism_codegen::isa::isa()
        .map_err(WorkspaceError::Backend)?
        .pointer_type();
    Ok(dump_layouts(&mir, &declared, pointer))
}

/// The path under which `graph` knows the entry file `entry`.
fn entry_in_graph(graph: &DependencyGraph, entry: &Path) -> Result<PathBuf, WorkspaceError> {
    let canonical = entry.canonicalize().map_err(|e| WorkspaceError::Io {
        path: entry.to_path_buf(),
        message: e.to_string(),
    })?;
    graph
        .nodes
        .keys()
        .find(|path| path.canonicalize().ok().as_ref() == Some(&canonical))
        .cloned()
        .ok_or_else(|| WorkspaceError::Io {
            path: entry.to_path_buf(),
            message: "the entry file is not in its own dependency graph".to_string(),
        })
}
