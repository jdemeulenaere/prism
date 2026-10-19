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

use crate::cache::{BuildCache, CacheKeys, EntryKind};
use crate::plan::{
    instance_code_key, planned_display, program_types, BuildPlan, DefinitionsHasher,
    PlannedInstanceKey, PlannedUnit, UnitId,
};
use crate::workspace::{
    assert_workspace_boundary, elaborate_and_summarize, elaborate_file, explain_cache,
    extract_abis, file_check_key, instance_errors, module_path, source_of, validate_entry_file,
    CacheOutcome, ElaboratedFile, InstanceOutcome, WorkspaceError,
};
use prism_abi::ContentHash;
use prism_codegen::{
    build_instance_unit, check_file, check_instances, collect_instances, dump_file, dump_instances,
    dump_layouts, find_main, plan_instances, DumpError, InstanceDumpError, InstanceUnit, Linker,
    PlannedInstance, Program, UnitArtifact, ValueKeys,
};
use prism_mir::{FileSummary, GenericRecord, MirFile};
use prism_resolver::{DependencyGraph, SourceNode};
use prism_typed_tree::instantiation::generic_name;
use prism_typed_tree::{DeclarationPath, Item, NominalId, TypeTable};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
    /// The bundled library, which tests replace to add modules to it (D13 of
    /// `standard-library`). The command line never does.
    pub library: std::sync::Arc<prism_library::Library>,
    /// `--headless[=<w>x<h>] [--events <script>]`: `runApp` runs headless, with the parsed
    /// script (D16 of `ui-toolkit`); `None` leaves the host as configured.
    pub headless: Option<HeadlessOptions>,
    /// `--screenshot <file.png>`: each app's final frame is written to this file as a PNG
    /// (D15 of `desktop-window`). It needs the `window` feature, which holds the rasterizer;
    /// without it the command line rejects the option and the library ignores it.
    pub screenshot: Option<PathBuf>,
}

/// How a headless run's apps run: the window size that overrides `runApp`'s, and the event
/// script's commands, parsed before the program is compiled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadlessOptions {
    pub size: Option<(i64, i64)>,
    pub commands: Vec<prism_ui::script::ScriptLine>,
}

impl HeadlessOptions {
    /// Installs these options as the host every later `runApp` of the process uses.
    pub fn configure(&self) {
        prism_ui::configure(prism_ui::HostConfiguration::Headless {
            size: self.size,
            commands: self.commands.clone(),
        });
    }
}

/// The commands of the event script at `path`, or the error `prism run` reports for it:
/// `<path>:<line>: <message>` for a line it cannot parse, `<path>: <message>` for a file it
/// cannot read (D16 of `ui-toolkit`).
pub fn read_event_script(
    path: &Path,
    watching: bool,
) -> Result<Vec<prism_ui::script::ScriptLine>, String> {
    let text = fs::read_to_string(path).map_err(|error| {
        format!(
            "{}: cannot read the event script: {}",
            path.display(),
            error
        )
    })?;
    // `edit` paths are relative to the script's directory, and only a watching run may edit
    // or wait for a reload (D12 of `compose-hot-reload`).
    let context = prism_ui::script::ScriptContext {
        watching,
        directory: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        read: &|replacement: &Path| {
            fs::read_to_string(replacement).map_err(|error| error.to_string())
        },
    };
    prism_ui::script::parse_script_with(&text, &context)
        .map_err(|error| format!("{}:{}: {}", path.display(), error.line, error.message))
}

/// The size of `--headless=<width>x<height>`, or `None` when it is not two whole numbers.
pub fn parse_headless_size(text: &str) -> Option<(i64, i64)> {
    let (width, height) = text.split_once('x')?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    (width >= 0 && height >= 0).then_some((width, height))
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
            library: prism_library::Library::shared(),
            headless: None,
            screenshot: None,
        }
    }
}

impl RunOptions {
    /// Installs what these options ask of the UI runtime before a program runs: the headless
    /// host of `--headless`, and the screenshot writer of `--screenshot` as the final-frame
    /// sink. A run without `--headless` keeps the host configured before, which the CLI sets to
    /// its window system (D3 of `desktop-window`) and the library leaves empty.
    pub fn configure_ui(&self) {
        if let Some(headless) = &self.headless {
            headless.configure();
        }
        prism_ui::host::set_reload_timings(self.timings);
        #[cfg(feature = "window")]
        if let Some(path) = &self.screenshot {
            prism_ui::host::set_final_frame_sink(Some(prism_desktop::screenshot::writer(
                path.clone(),
            )));
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

/// The `E6xxx` errors `errors` of `path`, a file of `graph`.
pub(crate) fn codegen_error(
    graph: &DependencyGraph,
    path: &Path,
    errors: Vec<prism_codegen::CodegenError>,
) -> WorkspaceError {
    WorkspaceError::Codegen {
        path: path.to_path_buf(),
        source: source_of(graph, path),
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

/// What a watch session keeps of one file between builds (D9 and D14 of `hot-reload`): the
/// check key its summary was computed for, the summary, and its MIR and records when Phase 2
/// ran for it in the session.
#[derive(Clone)]
pub(crate) struct KeptFile {
    pub check_key: ContentHash,
    pub summary: FileSummary,
    pub elaborated: Option<Rc<ElaboratedFile>>,
}

/// The front end's results a watch session keeps between builds, by file.
#[derive(Clone, Default)]
pub(crate) struct KeptFiles {
    pub files: HashMap<PathBuf, KeptFile>,
}

/// What [`build_program`] builds.
pub(crate) struct BuildRequest<'a> {
    pub graph: &'a DependencyGraph,
    /// The entry file's path as `graph` knows it.
    pub entry: &'a Path,
    pub cache: Option<&'a BuildCache>,
    pub keys: &'a CacheKeys,
    /// What earlier builds of the session kept: a file whose check key is unchanged is not
    /// checked again.
    pub kept: &'a KeptFiles,
    /// The plan of the code that is running, or `None` to build every unit: only the units
    /// whose code key is not in it are built (D9 of `hot-reload`).
    pub running: Option<&'a BuildPlan>,
}

/// One unit a build defines, with its artifact.
pub(crate) struct BuiltUnit {
    pub id: UnitId,
    pub artifact: UnitArtifact,
}

/// A program checked, planned, and with the artifacts of the units it needs, ready to define.
pub(crate) struct Build {
    pub main: DeclarationPath,
    pub plan: BuildPlan,
    /// What to keep of every file if the build is applied.
    pub kept: KeptFiles,
    /// The units to define, file units in check order, then instance units in symbol order.
    pub units: Vec<BuiltUnit>,
    /// Each file in check order, with whether it was checked, compiled or loaded.
    pub files: Vec<(PathBuf, CacheOutcome)>,
    /// Each instance built, in symbol order, with whether its code was loaded.
    pub instances: Vec<InstanceOutcome>,
    pub front_end: Duration,
    pub code_generation: Duration,
    /// The initializer hash of each top-level value of the file units built (D8 of
    /// `hot-reload`).
    pub value_keys: ValueKeys,
    /// The generic of each instance of the plan, by the instance's symbol, whose composable
    /// bodies a reload of the instance affects (D4 of `compose-hot-reload`).
    pub instance_generics: HashMap<String, DeclarationPath>,
}

/// One file of a program being built.
struct FileState {
    path: PathBuf,
    module: String,
    outcome: CacheOutcome,
    summary: FileSummary,
    /// Its MIR and records, when Phase 2 ran for it in this build or the session kept them.
    elaborated: Option<Rc<ElaboratedFile>>,
    /// Whether Phase 2 ran for it in this build, so that its backend checks are still to run.
    checked_now: bool,
    check_key: ContentHash,
    code_key: ContentHash,
}

/// One instance of a program being built: its plan and its code key.
struct InstanceState {
    planned: PlannedInstance,
    key: ContentHash,
}

/// Builds the program of `request` into `linker`: Phase 2 of every file whose check key the
/// session did not keep and the cache holds no verdict for, the entry's `main`, the backend's
/// support check of every file checked, the plan of every unit with its code key, and the
/// artifact of every unit the running plan lacks (all of them without one), taken from the
/// cache or compiled and written back (D9 of `instance-cache`, D9 and D14 of `hot-reload`).
/// The artifacts are not defined.
pub(crate) fn build_program(
    request: &BuildRequest,
    linker: &mut Linker,
) -> Result<Build, WorkspaceError> {
    let start = Instant::now();
    let BuildRequest {
        graph,
        entry,
        cache,
        keys,
        kept,
        running,
    } = request;
    let (graph, cache, running) = (*graph, *cache, *running);
    let (abis, definitions) = extract_abis(graph)?;
    let target = prism_codegen::isa::target_identity().map_err(WorkspaceError::Backend)?;

    let mut files: Vec<FileState> = Vec::new();
    for cluster in graph.topological_order() {
        for path in &cluster.files {
            let node = graph.get_node(path).expect("node must exist in graph");
            let check_key = file_check_key(keys, graph, &abis, path);
            let code_key = keys.unit_key(&check_key, &target);
            let module = module_path(node);
            let kept = kept
                .files
                .get(path)
                .filter(|kept| kept.check_key == check_key);
            let state = match kept {
                Some(kept) => FileState {
                    path: path.clone(),
                    module,
                    outcome: CacheOutcome::Reused,
                    summary: kept.summary.clone(),
                    elaborated: kept.elaborated.clone(),
                    checked_now: false,
                    check_key,
                    code_key,
                },
                None => match cache.and_then(|cache| cache.read_check(&check_key)) {
                    Some(summary) => FileState {
                        path: path.clone(),
                        module,
                        outcome: CacheOutcome::Reused,
                        summary,
                        elaborated: None,
                        checked_now: false,
                        check_key,
                        code_key,
                    },
                    None => {
                        let file = elaborate_and_summarize(graph, &abis, path)?;
                        if let Some(cache) = cache {
                            file.record(cache, &check_key);
                        }
                        FileState {
                            path: path.clone(),
                            module,
                            outcome: CacheOutcome::Checked,
                            summary: file.summary.clone(),
                            elaborated: Some(Rc::new(file)),
                            checked_now: true,
                            check_key,
                            code_key,
                        }
                    }
                },
            };
            files.push(state);
        }
    }
    let front_end = start.elapsed();

    let start = Instant::now();
    let main = entry_main(graph, &abis, entry, &mut files)?;
    for file in files.iter().filter(|file| file.checked_now) {
        let elaborated = file
            .elaborated
            .as_ref()
            .expect("a checked file was elaborated");
        check_file(&elaborated.mir).map_err(|errors| codegen_error(graph, &file.path, errors))?;
    }
    let summaries: Vec<&FileSummary> = files.iter().map(|file| &file.summary).collect();
    let planned = plan_instances(&summaries).map_err(|errors| instance_errors(graph, errors))?;
    let mut hasher = DefinitionsHasher::new(&definitions);
    let instances: Vec<InstanceState> = planned
        .into_iter()
        .map(|planned| {
            let key = instance_code_key(keys, &planned, &mut hasher, &target);
            InstanceState { planned, key }
        })
        .collect();
    let plan = BuildPlan {
        units: files
            .iter()
            .map(|file| PlannedUnit {
                path: file.path.clone(),
                module: file.module.clone(),
                check_key: file.check_key,
                code_key: file.code_key,
            })
            .collect(),
        instances: instances
            .iter()
            .map(|instance| PlannedInstanceKey {
                symbol: instance.planned.symbol.clone(),
                display: planned_display(&instance.planned),
                code_key: instance.key,
            })
            .collect(),
    };
    let needed: Option<HashSet<UnitId>> =
        running.map(|running| plan.changed_since(running).into_iter().collect());
    let is_needed = |id: UnitId| needed.as_ref().is_none_or(|needed| needed.contains(&id));

    let mut units: Vec<BuiltUnit> = Vec::new();
    let mut value_keys = ValueKeys::new();
    let cached_units: Vec<Option<UnitArtifact>> = files
        .iter()
        .map(|file| match (cache, file.checked_now) {
            (Some(cache), false) if is_needed(UnitId::File(file.module.clone())) => cache
                .read_code(EntryKind::Unit, &file.code_key)
                .and_then(|bytes| UnitArtifact::decode(&bytes)),
            _ => None,
        })
        .collect();
    // Every file whose code is to be generated passes the backend's support check before any
    // unit is compiled, so that a program the backend rejects writes no code to the cache,
    // whatever the order of its units (the library's come first).
    for (file, cached) in files.iter_mut().zip(&cached_units) {
        let needed = is_needed(UnitId::File(file.module.clone()));
        if needed && cached.is_none() && file.elaborated.is_none() {
            let elaborated = elaborate_and_summarize(graph, &abis, &file.path)?;
            check_file(&elaborated.mir)
                .map_err(|errors| codegen_error(graph, &file.path, errors))?;
            file.elaborated = Some(Rc::new(elaborated));
        }
    }
    for (file, cached) in files.iter_mut().zip(cached_units) {
        let id = UnitId::File(file.module.clone());
        if !is_needed(id.clone()) {
            continue;
        }
        let artifact = match cached {
            Some(artifact) => artifact,
            None => {
                if file.outcome == CacheOutcome::Reused {
                    file.outcome = CacheOutcome::Compiled;
                }
                let mir = &file.elaborated.as_ref().expect("elaborated above").mir;
                let artifact = linker
                    .compile_unit(mir, &mir.module)
                    .map_err(WorkspaceError::Backend)?;
                write_code(cache, EntryKind::Unit, &file.code_key, &artifact);
                artifact
            }
        };
        let node = graph
            .get_node(&file.path)
            .expect("node must exist in graph");
        value_keys.extend(initializer_hashes(&file.module, node));
        units.push(BuiltUnit { id, artifact });
    }

    let needed_instances: Vec<&InstanceState> = instances
        .iter()
        .filter(|instance| is_needed(UnitId::Instance(instance.planned.symbol.clone())))
        .collect();
    let cached: Vec<Option<UnitArtifact>> = needed_instances
        .iter()
        .map(|instance| {
            cache.and_then(|cache| {
                UnitArtifact::decode(&cache.read_code(EntryKind::Instance, &instance.key)?)
            })
        })
        .collect();
    let missed_plans: Vec<&PlannedInstance> = needed_instances
        .iter()
        .zip(&cached)
        .filter(|(_, artifact)| artifact.is_none())
        .map(|(instance, _)| &instance.planned)
        .collect();
    let missed =
        build_missed_instances(graph, &abis, cache, &definitions, &mut files, &missed_plans)?;
    check_instances(&missed).map_err(|errors| instance_errors(graph, errors))?;
    let mut missed = missed.into_iter();
    let mut instance_outcomes = Vec::new();
    for (instance, cached) in needed_instances.into_iter().zip(cached) {
        let reused = cached.is_some();
        let artifact = match cached {
            Some(artifact) => artifact,
            None => {
                let unit = missed.next().expect("every missed instance was built");
                let artifact = linker
                    .compile_instance(&unit)
                    .map_err(WorkspaceError::Backend)?;
                write_code(cache, EntryKind::Instance, &instance.key, &artifact);
                artifact
            }
        };
        instance_outcomes.push(InstanceOutcome {
            instance: planned_display(&instance.planned),
            reused,
        });
        units.push(BuiltUnit {
            id: UnitId::Instance(instance.planned.symbol.clone()),
            artifact,
        });
    }
    let code_generation = start.elapsed();

    let kept = KeptFiles {
        files: files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    KeptFile {
                        check_key: file.check_key,
                        summary: file.summary.clone(),
                        elaborated: file.elaborated.clone(),
                    },
                )
            })
            .collect(),
    };
    Ok(Build {
        main,
        plan,
        kept,
        units,
        files: files
            .iter()
            .map(|file| (file.path.clone(), file.outcome))
            .collect(),
        instances: instance_outcomes,
        front_end,
        code_generation,
        value_keys,
        instance_generics: instances
            .iter()
            .map(|instance| {
                (
                    instance.planned.symbol.clone(),
                    instance.planned.path.clone(),
                )
            })
            .collect(),
    })
}

/// The initializer hash of each top-level value of the file `node` of module `module`, by the
/// value's symbol (D8 of `hot-reload`).
fn initializer_hashes<'a>(
    module: &'a str,
    node: &'a SourceNode,
) -> impl Iterator<Item = (String, ContentHash)> + 'a {
    node.source_file
        .declarations
        .iter()
        .filter_map(move |declaration| match declaration {
            prism_ast::Declaration::Var(value) => Some((
                format!("{}::{}", module, value.name),
                prism_abi::initializer_hash(value),
            )),
            _ => None,
        })
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
    let graph = DependencyGraph::from_entries_with_library(&[entry], &options.library)?;
    assert_workspace_boundary(&graph)?;
    let entry_path = entry_in_graph(&graph, entry)?;
    let graph_time = start.elapsed();
    let keys = options
        .cache
        .as_ref()
        .map_or_else(CacheKeys::in_process, |cache| cache.keys().clone());
    let mut linker = Linker::new().map_err(WorkspaceError::Backend)?;
    let build = build_program(
        &BuildRequest {
            graph: &graph,
            entry: &entry_path,
            cache: options.cache.as_ref(),
            keys: &keys,
            kept: &KeptFiles::default(),
            running: None,
        },
        &mut linker,
    )?;
    let (program, linking, compiled_functions) = link_build(linker, &build)?;
    Ok(CompiledProgram {
        program,
        main: build.main,
        timings: Timings {
            front_end: graph_time + build.front_end,
            code_generation: build.code_generation,
            linking,
        },
        workspace_root: graph.workspace_root.clone(),
        files: build.files,
        instances: build.instances,
        compiled_functions,
        plan: Some(build.plan),
    })
}

/// Defines every unit of `build` into `linker` and links the program, with the time it took and
/// the number of functions Cranelift compiled for it.
pub(crate) fn link_build(
    mut linker: Linker,
    build: &Build,
) -> Result<(Program, Duration, usize), WorkspaceError> {
    let start = Instant::now();
    linker.set_value_keys(build.value_keys.clone());
    for unit in &build.units {
        linker
            .define_artifact(unit.id.name(), &unit.artifact)
            .map_err(WorkspaceError::Backend)?;
    }
    let compiled = linker.compiled_functions();
    let program = linker.link().map_err(WorkspaceError::Backend)?;
    Ok((program, start.elapsed(), compiled))
}

/// Writes `artifact` as the code entry `key` of `kind`, unless there is no cache or the
/// artifact names something its encoding cannot (D8 of `instance-cache`).
fn write_code(
    cache: Option<&BuildCache>,
    kind: EntryKind,
    key: &ContentHash,
    artifact: &UnitArtifact,
) {
    if let (Some(cache), Some(bytes)) = (cache, artifact.encode()) {
        cache.write_code(kind, key, &bytes);
    }
}

/// The entry file's `main`: its summary's when it records one, otherwise `find_main`'s error
/// on the entry's elaborated MIR, which errors are never cached for (D9 of `instance-cache`).
fn entry_main(
    graph: &DependencyGraph,
    abis: &crate::workspace::ProgramAbis,
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
    find_main(mir).map_err(|error| codegen_error(graph, entry_path, vec![error]))
}

/// The instance units of `missed`, in order, each built from its generic's record: one
/// elaborated in this build or kept by the session, else its record entry, else its generic's
/// file elaborated again, which is then listed as checked (D5 and D9 of `instance-cache`). The
/// program-wide type table is built only when an instance misses (D13).
fn build_missed_instances(
    graph: &DependencyGraph,
    abis: &crate::workspace::ProgramAbis,
    cache: Option<&BuildCache>,
    definitions: &crate::plan::ProgramDefinitions,
    files: &mut [FileState],
    missed: &[&PlannedInstance],
) -> Result<Vec<InstanceUnit>, WorkspaceError> {
    let mut built = Vec::new();
    let mut types: Option<TypeTable> = None;
    let mut stored: HashMap<String, GenericRecord> = HashMap::new();
    for planned in missed {
        let name = generic_name(&planned.path);
        let record = record_of(graph, abis, cache, files, &mut stored, planned)?;
        let types = types.get_or_insert_with(|| program_types(definitions));
        let unit = build_instance_unit(record, planned, types).ok_or_else(|| {
            WorkspaceError::Backend(prism_codegen::BackendError::new(format!(
                "the record of {} has no body for {}",
                name, planned.symbol
            )))
        })?;
        built.push(unit);
    }
    Ok(built)
}

/// The record of the generic of `planned`, found as [`build_missed_instances`] says.
fn record_of<'a>(
    graph: &DependencyGraph,
    abis: &crate::workspace::ProgramAbis,
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
                if let Some(cache) = cache {
                    elaborated.record(cache, &file.check_key);
                }
                file.outcome = CacheOutcome::Checked;
                file.elaborated = Some(Rc::new(elaborated));
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
        check_file(mir).map_err(|errors| codegen_error(&graph, path, errors))?;
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
///
/// This is the library's entry, which opens no window: without `--headless`, `runApp` uses the
/// host configured before, none unless the caller set one. The command line serves windows
/// between [`prepare_run`] and [`start_run`] instead (D3 of `desktop-window`).
pub fn run_file(entry: &Path, options: &RunOptions) -> Result<i32, WorkspaceError> {
    let compiled = prepare_run(entry, options)?;
    start_run(&compiled, options, |_| {})?
        .join()
        .map_err(WorkspaceError::Backend)
}

/// Compiles the program whose entry file is `entry` for [`start_run`], as [`run_file`] does:
/// prints what `--explain-cache` and `--timings` ask for, and configures the UI runtime
/// ([`RunOptions::configure_ui`]).
pub fn prepare_run(entry: &Path, options: &RunOptions) -> Result<CompiledProgram, WorkspaceError> {
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
        print_load_timings(compiled.timings, options);
    }
    options.configure_ui();
    Ok(compiled)
}

/// Starts `compiled`'s `main` on the program's thread without waiting for it: `on_return` is
/// called on that thread with the exit status once `main` returned and the run finished. The
/// program must outlive the returned handle's `join`.
pub fn start_run(
    compiled: &CompiledProgram,
    options: &RunOptions,
    on_return: impl FnOnce(i32) + Send + 'static,
) -> Result<prism_codegen::RunningProgram, WorkspaceError> {
    compiled
        .program
        .start(
            &compiled.main,
            options.arguments.clone(),
            options.check_allocations,
            on_return,
        )
        .map_err(WorkspaceError::Backend)
}

/// Prints the time of each stage of a load and the total time to `main` (`--timings`).
pub(crate) fn print_load_timings(timings: Timings, options: &RunOptions) {
    let milliseconds = |duration: Duration| duration.as_secs_f64() * 1000.0;
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

/// The Cranelift IR of the entry file `entry`, compiled alone against the ABIs of its direct
/// imports as `prism dump-mir --composed` does, or its errors of any stage.
pub fn dump_clif(entry: &Path) -> Result<String, WorkspaceError> {
    let dump = crate::workspace::dump_composed_mir(entry)?;
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
    let (tree, mir) = crate::workspace::compose_entry(entry)?;
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
pub(crate) fn entry_in_graph(
    graph: &DependencyGraph,
    entry: &Path,
) -> Result<PathBuf, WorkspaceError> {
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
        // The two files' units, and the prelude modules' (D3 of `standard-library`).
        let workspace_units = before
            .units
            .iter()
            .filter(|unit| !unit.module.starts_with("prism/"))
            .count();
        assert_eq!(workspace_units, 2);
        assert_eq!(before.units.len(), 5);
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
