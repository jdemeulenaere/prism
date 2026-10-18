//! `prism run`, `prism dump-clif` and `prism dump-layout`: compiling a program with the Tier 1 backend and running it
//! (the `execution` spec; D14, D20 and D23 of `codegen-core`; D9 of `instance-cache`).
//!
//! `prism run` checks the entry file and everything it transitively imports as `prism check`
//! does, in topological order and stopping at the first file with errors. With a build cache, a
//! file whose verdict and code are both cached is neither checked nor compiled: its artifact is
//! loaded. It then finds the entry file's `main`, runs the backend's support check over every
//! file it elaborated, plans the instances of generics the program needs from every file's
//! summary, builds and checks the instances whose code is not cached, compiles every unit it
//! does not load, writes what it compiled to the cache, and links every unit in memory through
//! one path before calling `main`.

#![allow(clippy::result_large_err)]

use crate::cache::{BuildCache, EntryKind};
use crate::plan::{
    instance_code_key, planned_display, program_types, BuildPlan, DefinitionsHasher,
    PlannedInstanceKey, PlannedUnit,
};
use crate::workspace::{
    assert_workspace_boundary, elaborate_and_summarize, elaborate_file, explain_cache,
    extract_abis, file_check_key, instance_errors, module_path, validate_entry_file, CacheOutcome,
    ElaboratedFile, InstanceOutcome, WorkspaceError,
};
use prism_abi::ContentHash;
use prism_codegen::{
    build_instance_unit, check_file, check_instances, collect_instances, dump_file, dump_instances,
    dump_layouts, find_main, plan_instances, DumpError, InstanceDumpError, InstanceUnit, Linker,
    PlannedInstance, Program, UnitArtifact,
};
use prism_mir::{FileSummary, GenericRecord, MirFile};
use prism_resolver::DependencyGraph;
use prism_typed_tree::instantiation::generic_name;
use prism_typed_tree::{DeclarationPath, Item, NominalId, TypeTable};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How `prism run` runs a program.
#[derive(Debug)]
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
    /// The build cache to read and write, or `None` to check and compile every file and
    /// instance (the library default, so that tests stay hermetic; D9 of `instance-cache`).
    pub cache: Option<BuildCache>,
    /// Whether to print, before `main` runs, what was checked, compiled or loaded
    /// (`--explain-cache`).
    pub explain_cache: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            timings: false,
            check_allocations: false,
            arguments: Vec::new(),
            started: Instant::now(),
            cache: None,
            explain_cache: false,
        }
    }
}

/// The time each stage of `prism run` took.
#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    /// Resolution, parsing, Phase 1 and Phase 2 of every file, or reading its cached verdict
    /// and code.
    pub front_end: Duration,
    /// The entry and support checks, planning, instantiation, lowering and Cranelift's
    /// compilation of every unit not loaded from the cache.
    pub code_generation: Duration,
    /// Defining every unit, finalizing the load and filling the function table.
    pub linking: Duration,
}

/// A program compiled and linked in memory, ready to run.
pub struct CompiledProgram {
    pub program: Program,
    /// The entry file's `main`.
    pub main: DeclarationPath,
    pub timings: Timings,
    /// The workspace root, which explained paths are relative to.
    pub workspace_root: PathBuf,
    /// Each file in check order, with whether it was checked, compiled or loaded.
    pub files: Vec<(PathBuf, CacheOutcome)>,
    /// Each instance in symbol order, with whether its code was loaded.
    pub instances: Vec<InstanceOutcome>,
    /// The number of functions Cranelift compiled: zero for a warm run of an unchanged program.
    pub compiled_functions: usize,
    /// The program's build plan, when a cache keyed its units (D12 of `instance-cache`).
    pub plan: Option<BuildPlan>,
}

/// The import graph of the program whose entry file is `entry` and the elaborated MIR of every
/// file, in topological order.
fn elaborate_program_graph(
    entry: &Path,
) -> Result<(DependencyGraph, Vec<(PathBuf, MirFile)>), WorkspaceError> {
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entry(entry)?;
    assert_workspace_boundary(&graph)?;
    let (abis, _) = extract_abis(&graph)?;
    let mut files: Vec<(PathBuf, MirFile)> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let mir = elaborate_file(&graph, &abis, path)?;
            files.push((path.clone(), mir));
        }
    }
    Ok((graph, files))
}

/// The elaborated MIR of every file of the program whose entry file is `entry`, checked and
/// lowered in topological order, with the entry's path as the graph knows it.
fn elaborate_program(entry: &Path) -> Result<(PathBuf, Vec<(PathBuf, MirFile)>), WorkspaceError> {
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entry(entry)?;
    assert_workspace_boundary(&graph)?;
    let (abis, _) = extract_abis(&graph)?;
    let entry_path = entry_in_graph(&graph, entry)?;
    let mut files: Vec<(PathBuf, MirFile)> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let mir = elaborate_file(&graph, &abis, path)?;
            files.push((path.clone(), mir));
        }
    }
    Ok((entry_path, files))
}

/// The elaborated MIR of every file of the program whose entry file is `entry`, in topological
/// order, or the program's errors of any stage before code generation. For tools and tests,
/// such as the record codec's round trip over the fixture corpus (`instance-cache`).
pub fn program_mir(entry: &Path) -> Result<Vec<MirFile>, WorkspaceError> {
    Ok(elaborate_program(entry)?
        .1
        .into_iter()
        .map(|(_, mir)| mir)
        .collect())
}

/// The `E6xxx` errors `errors` of `path`.
fn codegen_error(path: &Path, errors: Vec<prism_codegen::CodegenError>) -> WorkspaceError {
    WorkspaceError::Codegen {
        path: path.to_path_buf(),
        source: fs::read_to_string(path).unwrap_or_default(),
        errors,
    }
}

/// The instance units the program of `files` needs (D5 of `monomorphization`), with `check`
/// the backend's support check of each one, or their errors.
fn program_instances(
    graph: &DependencyGraph,
    files: &[(PathBuf, MirFile)],
    check: bool,
) -> Result<Vec<InstanceUnit>, WorkspaceError> {
    let mirs: Vec<&MirFile> = files.iter().map(|(_, mir)| mir).collect();
    let instances = collect_instances(&mirs).map_err(|errors| instance_errors(graph, errors))?;
    if check {
        check_instances(&instances).map_err(|errors| instance_errors(graph, errors))?;
    }
    Ok(instances)
}

/// One file of a program being compiled.
struct FileState {
    path: PathBuf,
    module: String,
    outcome: CacheOutcome,
    summary: FileSummary,
    /// Its MIR and records, when Phase 2 ran for it in this run.
    elaborated: Option<ElaboratedFile>,
    /// Its cached artifact, when it was loaded.
    artifact: Option<UnitArtifact>,
    /// The key of its check entry and of its code entry, with a cache.
    keys: Option<(ContentHash, ContentHash)>,
}

/// One instance of a program being compiled: its plan, its code key, and its cached artifact
/// when it was loaded.
struct InstanceState {
    planned: PlannedInstance,
    key: Option<ContentHash>,
    artifact: Option<UnitArtifact>,
    /// Whether its code was loaded from the cache.
    reused: bool,
}

/// Compiles the program whose entry file is `entry`: every file it transitively imports,
/// checked and lowered in topological order, and the instances of generics they need, each in
/// a unit of its own (D3 of `monomorphization`), without a cache.
pub fn compile_program(entry: &Path) -> Result<CompiledProgram, WorkspaceError> {
    compile_program_with(entry, &RunOptions::default())
}

/// [`compile_program`] with `options`: with a build cache, a file whose verdict is cached
/// skips Phase 2 unless its code or one of its generics' records is needed, a unit or instance
/// whose code is cached is loaded rather than compiled, and everything compiled is written back
/// (D9 and D14 of `instance-cache`). Errors are reported exactly as without a cache, since a
/// cached unit is one that passed every check.
pub fn compile_program_with(
    entry: &Path,
    options: &RunOptions,
) -> Result<CompiledProgram, WorkspaceError> {
    let start = Instant::now();
    validate_entry_file(entry)?;
    let graph = DependencyGraph::from_entry(entry)?;
    assert_workspace_boundary(&graph)?;
    let (abis, definitions) = extract_abis(&graph)?;
    let entry_path = entry_in_graph(&graph, entry)?;
    let cache = options.cache.as_ref();
    let target = match cache {
        Some(_) => Some(prism_codegen::isa::target_identity().map_err(WorkspaceError::Backend)?),
        None => None,
    };

    let mut files: Vec<FileState> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let node = graph.get_node(path).expect("node must exist in graph");
            let keys = cache.zip(target.as_deref()).map(|(cache, target)| {
                let check_key = file_check_key(cache, &graph, &abis, path);
                (check_key, cache.unit_key(&check_key, target))
            });
            let summary = cache
                .zip(keys.as_ref())
                .and_then(|(cache, (check_key, _))| cache.read_check(check_key));
            let artifact = match (&summary, cache, &keys) {
                (Some(_), Some(cache), Some((_, code_key))) => cache
                    .read_code(EntryKind::Unit, code_key)
                    .and_then(|bytes| UnitArtifact::decode(&bytes)),
                _ => None,
            };
            let state = match (summary, artifact) {
                (Some(summary), Some(artifact)) => FileState {
                    path: path.clone(),
                    module: module_path(node),
                    outcome: CacheOutcome::Reused,
                    summary,
                    elaborated: None,
                    artifact: Some(artifact),
                    keys,
                },
                (cached, _) => {
                    let file = elaborate_and_summarize(&graph, &abis, path)?;
                    let outcome = match cached {
                        Some(_) => CacheOutcome::Compiled,
                        None => {
                            if let Some((cache, (check_key, _))) = cache.zip(keys.as_ref()) {
                                file.record(cache, check_key);
                            }
                            CacheOutcome::Checked
                        }
                    };
                    FileState {
                        path: path.clone(),
                        module: module_path(node),
                        outcome,
                        summary: file.summary.clone(),
                        elaborated: Some(file),
                        artifact: None,
                        keys,
                    }
                }
            };
            files.push(state);
        }
    }
    let front_end = start.elapsed();

    let start = Instant::now();
    let main = entry_main(&graph, &abis, &entry_path, &mut files)?;
    for file in &files {
        if let Some(elaborated) = &file.elaborated {
            check_file(&elaborated.mir).map_err(|errors| codegen_error(&file.path, errors))?;
        }
    }
    let summaries: Vec<&FileSummary> = files.iter().map(|file| &file.summary).collect();
    let planned = plan_instances(&summaries).map_err(|errors| instance_errors(&graph, errors))?;
    let mut hasher = DefinitionsHasher::new(&definitions);
    let mut instances: Vec<InstanceState> = planned
        .into_iter()
        .map(|planned| {
            let key = cache
                .zip(target.as_deref())
                .map(|(cache, target)| instance_code_key(cache, &planned, &mut hasher, target));
            let artifact = cache.zip(key.as_ref()).and_then(|(cache, key)| {
                UnitArtifact::decode(&cache.read_code(EntryKind::Instance, key)?)
            });
            InstanceState {
                planned,
                key,
                artifact,
                reused: false,
            }
        })
        .collect();
    let missed =
        build_missed_instances(&graph, &abis, cache, &definitions, &mut files, &instances)?;
    check_instances(&missed).map_err(|errors| instance_errors(&graph, errors))?;

    let mut linker = Linker::new().map_err(WorkspaceError::Backend)?;
    let mut artifacts: Vec<(String, UnitArtifact)> = Vec::new();
    for file in &mut files {
        let artifact = match file.artifact.take() {
            Some(artifact) => artifact,
            None => {
                let mir = &file
                    .elaborated
                    .as_ref()
                    .expect("an uncached file was elaborated")
                    .mir;
                let artifact = linker
                    .compile_unit(mir, &mir.module)
                    .map_err(WorkspaceError::Backend)?;
                write_code(
                    cache,
                    EntryKind::Unit,
                    file.keys.as_ref().map(|(_, code)| code),
                    &artifact,
                );
                artifact
            }
        };
        artifacts.push((file.module.clone(), artifact));
    }
    let mut missed = missed.into_iter();
    for instance in &mut instances {
        match instance.artifact.take() {
            Some(artifact) => {
                instance.reused = true;
                artifacts.push((instance.planned.symbol.clone(), artifact));
            }
            None => {
                let unit = missed.next().expect("every missed instance was built");
                let artifact = linker
                    .compile_instance(&unit)
                    .map_err(WorkspaceError::Backend)?;
                write_code(cache, EntryKind::Instance, instance.key.as_ref(), &artifact);
                artifacts.push((instance.planned.symbol.clone(), artifact));
            }
        }
    }
    let code_generation = start.elapsed();

    let start = Instant::now();
    for (unit, artifact) in &artifacts {
        linker
            .define_artifact(unit, artifact)
            .map_err(WorkspaceError::Backend)?;
    }
    let compiled_functions = linker.compiled_functions();
    let program = linker.link().map_err(WorkspaceError::Backend)?;
    let linking = start.elapsed();

    let plan = cache.map(|_| BuildPlan {
        units: files
            .iter()
            .filter_map(|file| {
                let (check_key, code_key) = file.keys?;
                Some(PlannedUnit {
                    path: file.path.clone(),
                    module: file.module.clone(),
                    check_key,
                    code_key,
                })
            })
            .collect(),
        instances: instances
            .iter()
            .filter_map(|instance| {
                Some(PlannedInstanceKey {
                    symbol: instance.planned.symbol.clone(),
                    display: planned_display(&instance.planned),
                    code_key: instance.key?,
                })
            })
            .collect(),
    });
    Ok(CompiledProgram {
        program,
        main,
        timings: Timings {
            front_end,
            code_generation,
            linking,
        },
        workspace_root: graph.workspace_root.clone(),
        files: files
            .iter()
            .map(|file| (file.path.clone(), file.outcome))
            .collect(),
        instances: instances
            .iter()
            .map(|instance| InstanceOutcome {
                instance: planned_display(&instance.planned),
                reused: instance.reused,
            })
            .collect(),
        compiled_functions,
        plan,
    })
}

/// Writes `artifact` as the code entry `key` of `kind`, unless there is no cache or the
/// artifact names something its encoding cannot (D8 of `instance-cache`).
fn write_code(
    cache: Option<&BuildCache>,
    kind: EntryKind,
    key: Option<&ContentHash>,
    artifact: &UnitArtifact,
) {
    if let (Some(cache), Some(key), Some(bytes)) = (cache, key, artifact.encode()) {
        cache.write_code(kind, key, &bytes);
    }
}

/// The entry file's `main`: its summary's when it records one, otherwise `find_main`'s error
/// on the entry's elaborated MIR, which errors are never cached for (D9 of `instance-cache`).
fn entry_main(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, prism_abi::FileAbi>,
    entry_path: &Path,
    files: &mut [FileState],
) -> Result<DeclarationPath, WorkspaceError> {
    let entry = files
        .iter_mut()
        .find(|file| file.path == entry_path)
        .ok_or_else(|| WorkspaceError::Io {
            path: entry_path.to_path_buf(),
            message: "the entry file is not in its own dependency graph".to_string(),
        })?;
    if let Some(main) = &entry.summary.entry_point {
        return Ok(main.clone());
    }
    let mir = match &entry.elaborated {
        Some(file) => &file.mir,
        None => &elaborate_file(graph, abis, entry_path)?,
    };
    find_main(mir).map_err(|error| codegen_error(entry_path, vec![error]))
}

/// The instance units of `instances` whose code is not cached, in order, each built from its
/// generic's record: one elaborated in this run, else its record entry, else its generic's file
/// elaborated again, which is then listed as checked (D5 and D9 of `instance-cache`). The
/// program-wide type table is built only when an instance misses (D13).
fn build_missed_instances(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, prism_abi::FileAbi>,
    cache: Option<&BuildCache>,
    definitions: &crate::plan::ProgramDefinitions,
    files: &mut [FileState],
    instances: &[InstanceState],
) -> Result<Vec<InstanceUnit>, WorkspaceError> {
    let mut missed = Vec::new();
    let mut types: Option<TypeTable> = None;
    let mut stored: HashMap<String, GenericRecord> = HashMap::new();
    for instance in instances
        .iter()
        .filter(|instance| instance.artifact.is_none())
    {
        let name = generic_name(&instance.planned.path);
        let record = record_of(graph, abis, cache, files, &mut stored, &instance.planned)?;
        let types = types.get_or_insert_with(|| program_types(definitions));
        let unit = build_instance_unit(record, &instance.planned, types).ok_or_else(|| {
            WorkspaceError::Backend(prism_codegen::BackendError::new(format!(
                "the record of {} has no body for {}",
                name, instance.planned.symbol
            )))
        })?;
        missed.push(unit);
    }
    Ok(missed)
}

/// The record of the generic of `planned`, found as [`build_missed_instances`] says.
fn record_of<'a>(
    graph: &DependencyGraph,
    abis: &HashMap<PathBuf, prism_abi::FileAbi>,
    cache: Option<&BuildCache>,
    files: &'a mut [FileState],
    stored: &'a mut HashMap<String, GenericRecord>,
    planned: &PlannedInstance,
) -> Result<&'a GenericRecord, WorkspaceError> {
    let name = generic_name(&planned.path);
    let position = files
        .iter()
        .position(|file| file.module == planned.path.module)
        .ok_or_else(|| {
            WorkspaceError::Backend(prism_codegen::BackendError::new(format!(
                "no file of the program declares {}",
                name
            )))
        })?;
    if files[position].elaborated.is_none() && !stored.contains_key(&name) {
        let from_entry =
            cache.and_then(|cache| cache.read_record(&ContentHash(planned.record_hash)));
        match from_entry {
            Some(record) => {
                stored.insert(name.clone(), record);
            }
            None => {
                // The record entry is missing or corrupt: check the generic's file again and
                // rewrite its records.
                let file = &mut files[position];
                let elaborated = elaborate_and_summarize(graph, abis, &file.path)?;
                if let (Some(cache), Some((check_key, _))) = (cache, file.keys.as_ref()) {
                    elaborated.record(cache, check_key);
                }
                file.outcome = CacheOutcome::Checked;
                file.elaborated = Some(elaborated);
            }
        }
    }
    if let Some(record) = stored.get(&name) {
        return Ok(record);
    }
    files[position]
        .elaborated
        .as_ref()
        .and_then(|file| {
            file.records
                .iter()
                .find(|record| generic_name(&record.path) == name)
        })
        .ok_or_else(|| {
            WorkspaceError::Backend(prism_codegen::BackendError::new(format!(
                "no record of {}",
                name
            )))
        })
}

/// The elaborated MIR of every instance the program whose entry file is `entry` needs, in order
/// of their symbols, as `prism dump-mir --instances` prints it (D11 of `monomorphization`).
/// The whole program is compiled, as for `prism run`, since an instance needs other files'
/// records; a program with errors returns them instead.
pub fn dump_instance_mir(entry: &Path) -> Result<String, WorkspaceError> {
    let (graph, files) = elaborate_program_graph(entry)?;
    let instances = program_instances(&graph, &files, false)?;
    Ok(instances
        .iter()
        .map(|instance| instance.file.dump_bodies())
        .collect())
}

/// The Cranelift IR of every instance unit the program whose entry file is `entry` needs, as
/// `prism dump-clif --instances` prints it (D11 of `monomorphization`), or the program's errors
/// of any stage.
pub fn dump_instance_clif(entry: &Path) -> Result<String, WorkspaceError> {
    let (graph, files) = elaborate_program_graph(entry)?;
    for (path, mir) in &files {
        check_file(mir).map_err(|errors| codegen_error(path, errors))?;
    }
    let mirs: Vec<&MirFile> = files.iter().map(|(_, mir)| mir).collect();
    dump_instances(&mirs).map_err(|error| match error {
        InstanceDumpError::Rejected(errors) => instance_errors(&graph, errors),
        InstanceDumpError::Backend(error) => WorkspaceError::Backend(error),
    })
}

/// Compiles and runs the program whose entry file is `entry`, returning its exit status: 0 when
/// `main` returns, or 102 when the allocation check finds allocations still live. A runtime
/// error ends the process with status 101 instead of returning, and an invalid free under the
/// allocation check with status 102. With `--explain-cache`, what was checked, compiled or
/// loaded is printed to stderr before `main` runs, since stdout is the program's.
pub fn run_file(entry: &Path, options: &RunOptions) -> Result<i32, WorkspaceError> {
    let compiled = compile_program_with(entry, options)?;
    if options.explain_cache {
        let files: Vec<(&Path, CacheOutcome)> = compiled
            .files
            .iter()
            .map(|(path, outcome)| (path.as_path(), *outcome))
            .collect();
        eprint!(
            "{}",
            explain_cache(
                options.cache.as_ref().map(BuildCache::directory),
                &compiled.workspace_root,
                &files,
                &compiled.instances
            )
        );
    }
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
    // The graph adds its entry under its absolute path, so this finds it without canonicalizing
    // every file's path.
    let absolute = prism_resolver::absolute_path(entry);
    if graph.nodes.contains_key(&absolute) {
        return Ok(absolute);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::UnitId;

    fn workspace(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("prism_run_unit_{}_{}", name, nanos));
        for (path, source) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
        root
    }

    fn options(root: &Path) -> RunOptions {
        RunOptions {
            cache: Some(BuildCache::open(
                root.join(".cache"),
                crate::cache::identity_of("test", &ContentHash([3; 32])),
            )),
            ..RunOptions::default()
        }
    }

    /// D9 of `hot-reload`: a body edit changes the edited file's unit alone, and a field added
    /// to a type argument changes that type's file and the instances over it.
    #[test]
    fn a_body_edit_or_a_type_argument_edit_changes_only_its_units() {
        const SHAPES: &str = "package lib\npublic struct P : Copy { val x: Int }\n\
                              public fun origin(): P = P(1)\n";
        const MATH: &str = "package lib\npublic fun twice(n: Int): Int = n * 2\n";
        let root = workspace(
            "plan_edits",
            &[
                ("lib/box.prism", BOX),
                ("lib/shapes.prism", SHAPES),
                ("lib/math.prism", MATH),
                (
                    "app/main.prism",
                    "package app\nimport /lib/box\nimport /lib/shapes\nimport /lib/math\n\
                     fun main() { println(keep(origin()).x); println(keep(2)); println(twice(3)) }\n",
                ),
            ],
        );
        let entry = root.join("app/main.prism");
        let options = options(&root);
        let before = compile_program_with(&entry, &options)
            .unwrap()
            .plan
            .unwrap();
        fs::write(root.join("lib/math.prism"), MATH.replace("n * 2", "n * 3")).unwrap();
        let body = compile_program_with(&entry, &options)
            .unwrap()
            .plan
            .unwrap();
        assert_eq!(
            body.changed_since(&before),
            [UnitId::File("lib/math".to_string())]
        );
        fs::write(
            root.join("lib/shapes.prism"),
            SHAPES
                .replace("val x: Int }", "val x: Int, val y: Int }")
                .replace("P(1)", "P(1, 2)"),
        )
        .unwrap();
        let field = compile_program_with(&entry, &options)
            .unwrap()
            .plan
            .unwrap();
        let changed = field.changed_since(&body);
        // `app/main` imports the edited type, whose definition is in its import's ABI hash.
        assert_eq!(
            changed,
            [
                UnitId::File("lib/shapes".to_string()),
                UnitId::File("app/main".to_string()),
                UnitId::Instance("lib/box::keep<lib/shapes::P>".to_string()),
            ]
        );
        fs::remove_dir_all(&root).ok();
    }

    const BOX: &str =
        "package lib\npublic fun <T> drop(x: T): Int = 1\npublic fun <T> keep(x: T): T = x\n";

    /// D12 of `instance-cache`: two plans of one program, before and after a generic body
    /// edit, differ in the defining file's unit and that generic's instances alone.
    #[test]
    fn a_generic_body_edit_changes_one_unit_and_its_instances() {
        let root = workspace(
            "plan",
            &[
                ("lib/box.prism", BOX),
                (
                    "app/main.prism",
                    "package app\nimport /lib/box\nfun main() { println(keep(1)); println(drop(\"s\")) }\n",
                ),
            ],
        );
        let entry = root.join("app/main.prism");
        let options = options(&root);
        let before = compile_program_with(&entry, &options).unwrap();
        assert!(before.compiled_functions > 0);
        let warm = compile_program_with(&entry, &options).unwrap();
        assert_eq!(warm.compiled_functions, 0, "a warm run compiles nothing");
        assert_eq!(warm.plan, before.plan);
        let before = before.plan.unwrap();
        assert_eq!(before.units.len(), 2);
        assert_eq!(before.instances.len(), 2);
        assert!(before.changed_since(&before).is_empty());

        fs::write(
            root.join("lib/box.prism"),
            BOX.replace("): T = x", "): T { val y = x; return y }"),
        )
        .unwrap();
        let after = compile_program_with(&entry, &options).unwrap();
        let plan = after.plan.unwrap();
        assert_eq!(
            plan.changed_since(&before),
            [
                UnitId::File("lib/box".to_string()),
                UnitId::Instance("lib/box::keep<Int>".to_string())
            ]
        );
        fs::remove_dir_all(&root).ok();
    }
}
