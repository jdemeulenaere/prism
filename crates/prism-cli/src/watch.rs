//! `prism run --watch`: a watch session that keeps a program running while its files change
//! (the `hot-reload` spec).
//!
//! The session runs on the CLI's thread, or on a `watch-session` thread while the main thread
//! serves windows (D17 of `desktop-window`), and the program on a thread of its own, as
//! `prism run` runs it (D2). The session watches the program's files ([`FileWatcher`], D15) or follows an
//! edit script ([`EditScript`], D16). On a change it rebuilds the graph through its
//! [`SourceStore`], which parses only changed files, checks again only the files whose check key
//! changed, and compiles only the units whose code key is not in the plan of the code that is
//! running ([`crate::run::build_program`], D9 and D14). The new units are defined into one new
//! JIT module over the program's tables and finalized while the program keeps running; the
//! program thread applies the reload at its next call (D3). A reload with errors keeps the
//! previous code (D12). When `main` returns or stops with a runtime error, the next change that
//! compiles runs `main` again in a fresh program (D13).
//!
//! Status lines go to stderr and start with `reload:` or `watch:`; the program's output goes to
//! stdout.

#![allow(clippy::result_large_err)]

use crate::cache::CacheKeys;
use crate::plan::{BuildPlan, UnitId};
use crate::run::{
    build_program, entry_in_graph, link_build, print_load_timings, Build, BuildRequest, KeptFiles,
    RunOptions,
};
use crate::workspace::{
    assert_workspace_boundary, explain_cache, validate_entry_file, CacheOutcome, WorkspaceError,
};
use prism_codegen::{Program, SlotKind};
use prism_resolver::{DependencyGraph, ResolveError, SourceStore};
use prism_runtime::reload::{self, AppliedReload, PendingReload};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the session waits for.
#[derive(Debug)]
pub enum SessionEvent {
    /// Files of the program changed on disk, first noticed at the instant given.
    FilesChanged(Vec<PathBuf>, Instant),
    /// `main` returned, and the run finished with this exit status.
    MainReturned(i32),
    /// The program stopped with a runtime error; the process would have exited with this
    /// status.
    Stopped(i32),
    /// The program wrote its `n`-th newline of the run and waits for the session (D16).
    LineWritten(u64),
}

/// What a change did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// No file read differently, or no unit's code changed.
    Unchanged,
    /// A reload was handed to the running program.
    Requested,
    /// `main` was started in a fresh program.
    Started,
    /// The change did not compile; the program keeps its previous code.
    Failed,
}

/// The time each stage of a reload took, reported with `--timings`.
#[derive(Debug, Clone, Copy, Default)]
struct ReloadTimings {
    detect: Duration,
    front_end: Duration,
    code_generation: Duration,
    linking: Duration,
}

/// Runs `prism run --watch` on `entry` with `options`, following the edit script `edits` when
/// one is given, and returns the session's exit status. Without a script it returns only if the
/// watcher fails; the session normally ends with the process (Ctrl-C).
pub fn run_watch(entry: &Path, options: RunOptions, edits: Option<&Path>) -> i32 {
    run_watch_with(entry, options, edits, None)
}

/// What a watch session calls each time the program stops, by returning from `main` or with a
/// runtime error: the window system's `close_all`, since a stopped program thread never drops
/// its window host (D17 of `desktop-window`).
pub type ProgramEndHook = Arc<dyn Fn() + Send + Sync>;

/// [`run_watch`], calling `on_program_end` each time the program stops.
pub fn run_watch_with(
    entry: &Path,
    options: RunOptions,
    edits: Option<&Path>,
    on_program_end: Option<ProgramEndHook>,
) -> i32 {
    let script = match edits.map(EditScript::parse) {
        Some(Ok(script)) => Some(script),
        Some(Err(message)) => {
            eprintln!("Error: {}", message);
            return 1;
        }
        None => None,
    };
    options.configure_ui();
    let mut session = WatchSession::new(entry, options);
    session.on_program_end = on_program_end;
    match script {
        Some(script) => session.run_script(script),
        None => session.run_watching(),
    }
}

/// A watch session: the program's sources, what the front end kept of each file, the running
/// program and the plan of the code it runs.
pub struct WatchSession {
    entry: PathBuf,
    options: RunOptions,
    store: SourceStore,
    keys: CacheKeys,
    sender: Sender<SessionEvent>,
    events: Receiver<SessionEvent>,
    /// What the front end kept of each file, from the last build that succeeded. It is keyed by
    /// check keys, so it is valid whether or not that build's code runs.
    kept: KeptFiles,
    /// The live program, if one compiled; kept after `main` returns until a fresh one replaces
    /// it.
    program: Option<Program>,
    /// The plan of the code the program runs.
    running_plan: Option<BuildPlan>,
    /// The plan of the reload requested last, until the session knows whether it was applied.
    pending_plan: Option<BuildPlan>,
    /// Whether `main` is running.
    running: bool,
    /// Whether the session tried to start `main` already.
    attempted: bool,
    /// The exit status of the last run of `main`.
    status: Option<i32>,
    /// Whether the last change after the last run did not compile.
    failed: bool,
    /// The files the session watches: the program's, and the targets of unresolved imports.
    watched: Vec<PathBuf>,
    /// The workspace root of the last graph, which status lines show paths relative to.
    workspace_root: PathBuf,
    /// The output line at which an edit script's step stops the program, or 0.
    line_target: Arc<AtomicU64>,
    /// Lets the program thread stopped at a line continue.
    resume: Sender<()>,
    resume_receiver: Arc<Mutex<Receiver<()>>>,
    /// Called each time the program stops ([`run_watch_with`]).
    on_program_end: Option<ProgramEndHook>,
}

impl WatchSession {
    /// A session for the program whose entry file is `entry`. Nothing is compiled yet.
    pub fn new(entry: &Path, options: RunOptions) -> Self {
        let (sender, events) = mpsc::channel();
        let (resume, resume_receiver) = mpsc::channel();
        let keys = options
            .cache
            .as_ref()
            .map_or_else(CacheKeys::in_process, |cache| cache.keys().clone());
        Self {
            entry: entry.to_path_buf(),
            options,
            store: SourceStore::new(),
            keys,
            sender,
            events,
            kept: KeptFiles::default(),
            program: None,
            running_plan: None,
            pending_plan: None,
            running: false,
            attempted: false,
            status: None,
            failed: false,
            watched: vec![prism_resolver::absolute_path(entry)],
            workspace_root: PathBuf::new(),
            line_target: Arc::new(AtomicU64::new(0)),
            resume,
            resume_receiver: Arc::new(Mutex::new(resume_receiver)),
            on_program_end: None,
        }
    }

    /// The channel through which the session receives its events.
    pub fn sender(&self) -> Sender<SessionEvent> {
        self.sender.clone()
    }

    /// Whether `main` is running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// The number of modules the live program keeps mapped, if one compiled.
    pub fn live_modules(&self) -> Option<usize> {
        self.program
            .as_ref()
            .map(|program| program.live().module_count())
    }

    /// The files the session watches.
    pub fn watched(&self) -> &[PathBuf] {
        &self.watched
    }

    /// Makes a runtime error stop the program's thread and tell the session, rather than end the
    /// process (D13).
    pub fn install_stop_handler(&self) {
        let sender = Mutex::new(self.sender.clone());
        prism_runtime::set_watch_mode(Some(Arc::new(move |status, exited| {
            if let Ok(sender) = sender.lock() {
                // A program that ends itself with `exitProcess` ends its run as `main`
                // returning does (D10 of `standard-library`).
                let _ = sender.send(match exited {
                    true => SessionEvent::MainReturned(status),
                    false => SessionEvent::Stopped(status),
                });
            }
        })));
    }

    /// Makes the program stop at the output line the session sets as its line target, tell the
    /// session, and apply the reload the session requests once it lets it continue (D16).
    pub fn install_line_hook(&self) {
        let target = Arc::clone(&self.line_target);
        let sender = Mutex::new(self.sender.clone());
        let resume = Arc::clone(&self.resume_receiver);
        prism_runtime::set_line_hook(Some(Arc::new(move |count| {
            if count != target.load(Ordering::SeqCst) {
                return;
            }
            target.store(0, Ordering::SeqCst);
            if let Ok(sender) = sender.lock() {
                let _ = sender.send(SessionEvent::LineWritten(count));
            }
            if let Ok(resume) = resume.lock() {
                let _ = resume.recv();
            }
            reload::apply_now();
        })));
    }

    /// Stops the program at its `line`-th newline of the run.
    pub fn stop_at_line(&self, line: u64) {
        self.line_target.store(line, Ordering::SeqCst);
    }

    /// Lets the program stopped at a line continue, applying the reload requested meanwhile.
    pub fn resume(&self) {
        let _ = self.resume.send(());
    }

    /// Makes `target` read as `content` from now on, without changing the disk.
    pub fn overlay(&mut self, target: PathBuf, content: String) {
        self.store.set_overlay(target, content);
    }

    /// The next event.
    pub fn next_event(&self) -> Option<SessionEvent> {
        self.events.recv().ok()
    }

    /// Compiles the program and runs `main` in a fresh program, as the session's start does and
    /// as a change does once `main` returned or stopped (D13). A program that does not compile
    /// prints its diagnostics and `watch: waiting for changes`.
    pub fn start(&mut self) -> ReloadOutcome {
        let first = !self.attempted;
        self.attempted = true;
        let (graph, build, program, graph_time, linking) = match self.build_fresh() {
            Ok(built) => built,
            Err(error) => {
                eprint!("{}", error.render());
                eprintln!("watch: waiting for changes");
                self.failed = true;
                return ReloadOutcome::Failed;
            }
        };
        if first && self.options.explain_cache {
            let files: Vec<(&Path, CacheOutcome)> = build
                .files
                .iter()
                .map(|(path, outcome)| (path.as_path(), *outcome))
                .collect();
            eprint!(
                "{}",
                explain_cache(
                    self.options
                        .cache
                        .as_ref()
                        .map(crate::BuildCache::directory),
                    &graph.workspace_root,
                    &files,
                    &build.instances
                )
            );
        }
        if self.options.timings {
            print_load_timings(
                crate::Timings {
                    front_end: graph_time + build.front_end,
                    code_generation: build.code_generation,
                    linking,
                },
                &self.options,
            );
        }
        if !first {
            eprintln!("watch: running main again");
        }
        // The old program's modules are freed: its `main` returned, or its thread is parked.
        self.program = None;
        let main = build.main.clone();
        self.kept = build.kept;
        self.running_plan = Some(build.plan);
        self.pending_plan = None;
        self.failed = false;
        let sender = Mutex::new(self.sender.clone());
        let started = program.start(
            &main,
            self.options.arguments.clone(),
            self.options.check_allocations,
            move |status| {
                if let Ok(sender) = sender.lock() {
                    let _ = sender.send(SessionEvent::MainReturned(status));
                }
            },
        );
        self.program = Some(program);
        match started {
            Ok(_) => {
                self.running = true;
                ReloadOutcome::Started
            }
            Err(error) => {
                eprint!("{}", WorkspaceError::Backend(error).render());
                eprintln!("watch: waiting for changes");
                self.failed = true;
                ReloadOutcome::Failed
            }
        }
    }

    /// Reloads the program after `changed` files changed, first noticed at `detected` (D9):
    /// nothing when none of them reads differently; `main` in a fresh program when none runs;
    /// otherwise a reload of the units whose code changed, handed to the running program.
    pub fn reload(&mut self, changed: &[PathBuf], detected: Instant) -> ReloadOutcome {
        let changed: Vec<PathBuf> = changed
            .iter()
            .filter(|path| self.store.has_changed(path))
            .cloned()
            .collect();
        self.store.invalidate(&changed);
        // Only a change of a file the session watches reloads: an edit script may change any
        // file, and a file outside the program is read again once it joins it.
        if !changed.iter().any(|path| self.watched.contains(path)) {
            return ReloadOutcome::Unchanged;
        }
        if !self.running {
            return self.start();
        }
        self.settle_pending();
        let started = Instant::now();
        let prepared = self.prepare();
        let (build, prepared, mut timings) = match prepared {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return ReloadOutcome::Unchanged,
            Err(error) => {
                eprint!("{}", error.render());
                eprintln!("reload: not applied; the program keeps running its previous code");
                self.failed = true;
                return ReloadOutcome::Failed;
            }
        };
        timings.detect = started.saturating_duration_since(detected);
        let mut message = self.applied_message(&build, &prepared.splits);
        let units = prepared.unit_names();
        let program = self.program.as_mut().expect("a program runs");
        let writes = program.live_mut().hold(prepared);
        let modules = program.live().module_count();
        self.kept = build.kept;
        self.pending_plan = Some(build.plan);
        self.failed = false;
        let report_timings = self.options.timings;
        let requested = Instant::now();
        reload::request(PendingReload::new(move || {
            writes.apply();
            if report_timings {
                let milliseconds = |duration: Duration| duration.as_secs_f64() * 1000.0;
                let applied_after = requested.elapsed();
                message.push_str(&format!(
                    "timing: reload {:.3} ms (detect {:.3} ms, front end {:.3} ms, code generation {:.3} ms, linking {:.3} ms, applied after {:.3} ms)\ntiming: live modules {}\n",
                    milliseconds(detected.elapsed()),
                    milliseconds(timings.detect),
                    milliseconds(timings.front_end),
                    milliseconds(timings.code_generation),
                    milliseconds(timings.linking),
                    milliseconds(applied_after),
                    modules
                ));
            }
            AppliedReload { units, message }
        }));
        ReloadOutcome::Requested
    }

    /// Builds and links every unit of the program into a fresh program, with the time the graph
    /// and the linking took.
    fn build_fresh(
        &mut self,
    ) -> Result<(DependencyGraph, Build, Program, Duration, Duration), WorkspaceError> {
        let started = Instant::now();
        let graph = self.graph()?;
        let graph_time = started.elapsed();
        let entry = entry_in_graph(&graph, &self.entry)?;
        let mut linker = prism_codegen::Linker::new().map_err(WorkspaceError::Backend)?;
        let build = build_program(
            &BuildRequest {
                graph: &graph,
                entry: &entry,
                cache: self.options.cache.as_ref(),
                keys: &self.keys,
                kept: &self.kept,
                running: None,
            },
            &mut linker,
        )?;
        let (program, linking, _) = link_build(linker, &build)?;
        Ok((graph, build, program, graph_time, linking))
    }

    /// Builds the reload of the running program: the graph, the build of the units the running
    /// plan lacks, and their module, finalized but not applied. `None` when no unit changed.
    fn prepare(
        &mut self,
    ) -> Result<Option<(Build, prism_codegen::PreparedReload, ReloadTimings)>, WorkspaceError> {
        let started = Instant::now();
        let graph = self.graph()?;
        let graph_time = started.elapsed();
        let entry = entry_in_graph(&graph, &self.entry)?;
        let program = self.program.as_ref().expect("a program runs");
        let mut linker = program.live().linker().map_err(WorkspaceError::Backend)?;
        let build = build_program(
            &BuildRequest {
                graph: &graph,
                entry: &entry,
                cache: self.options.cache.as_ref(),
                keys: &self.keys,
                kept: &self.kept,
                running: self.running_plan.as_ref(),
            },
            &mut linker,
        )?;
        if build.units.is_empty() {
            self.kept = build.kept;
            return Ok(None);
        }
        let linking_started = Instant::now();
        linker.set_value_keys(build.value_keys.clone());
        for unit in &build.units {
            linker
                .define_artifact(unit.id.name(), &unit.artifact)
                .map_err(WorkspaceError::Backend)?;
        }
        let prepared = linker.prepare().map_err(WorkspaceError::Backend)?;
        let timings = ReloadTimings {
            detect: Duration::ZERO,
            front_end: graph_time + build.front_end,
            code_generation: build.code_generation,
            linking: linking_started.elapsed(),
        };
        Ok(Some((build, prepared, timings)))
    }

    /// Settles the reload requested last: if the program has not applied it, takes it back and
    /// frees its module; otherwise its plan becomes the running one (D12).
    fn settle_pending(&mut self) {
        let Some(program) = self.program.as_mut() else {
            return;
        };
        match reload::withdraw() {
            Some(_) => {
                program.live_mut().settle(false);
                self.pending_plan = None;
            }
            None => {
                if let Some(plan) = self.pending_plan.take() {
                    program.live_mut().settle(true);
                    self.running_plan = Some(plan);
                }
            }
        }
    }

    /// Handles the end of a run: `main` returned with `status`, or stopped with a runtime error
    /// when `stopped`.
    pub fn program_ended(&mut self, status: i32, stopped: bool) {
        if let Some(on_program_end) = &self.on_program_end {
            on_program_end();
        }
        self.running = false;
        self.status = Some(status);
        self.failed = false;
        self.line_target.store(0, Ordering::SeqCst);
        self.settle_pending();
        if stopped {
            eprintln!("watch: the program stopped with a runtime error; waiting for changes");
        } else {
            eprintln!(
                "watch: main returned with status {}; waiting for changes",
                status
            );
        }
    }

    /// The session's exit status: that of the last run of `main`, or 1 when the last change
    /// after it did not compile, or when `main` never ran.
    pub fn exit_status(&self) -> i32 {
        match (self.failed, self.status) {
            (false, Some(status)) => status,
            _ => 1,
        }
    }

    /// The program's dependency graph, rebuilt through the store. Records the files to watch,
    /// with the target of an import that does not resolve.
    fn graph(&mut self) -> Result<DependencyGraph, WorkspaceError> {
        validate_entry_file(&self.entry)?;
        let graph = match DependencyGraph::from_entry_with(&self.entry, &mut self.store) {
            Ok(graph) => graph,
            Err(error) => {
                // The files this attempt reached are watched too, so that fixing the file in
                // error, or a file importing it, is seen even when no build succeeded before.
                let reached = self
                    .store
                    .paths()
                    .map(Path::to_path_buf)
                    .chain(error_file(&error).map(Path::to_path_buf))
                    .collect::<Vec<PathBuf>>();
                for path in reached {
                    if !self.watched.contains(&path) {
                        self.watched.push(path);
                    }
                }
                self.watched.sort();
                return Err(match error {
                    // The source shown is what the store parsed, which may be an overlay's.
                    ResolveError::Parse {
                        path,
                        source,
                        error,
                    } => WorkspaceError::Parse {
                        source,
                        path,
                        error,
                    },
                    other => WorkspaceError::Resolve(other),
                });
            }
        };
        assert_workspace_boundary(&graph)?;
        // The library is in the executable: it never changes during a session, so it is not
        // watched, and its units are never rebuilt (D3 of `standard-library`).
        let mut watched: Vec<PathBuf> = graph
            .nodes
            .values()
            .filter(|node| !node.is_library())
            .map(|node| node.path.clone())
            .collect();
        watched.sort();
        // A file the program no longer imports is not watched, so the store reads it again if
        // an import brings it back: it may have changed meanwhile.
        let dropped: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|path| !graph.nodes.contains_key(*path))
            .cloned()
            .collect();
        self.store.invalidate(&dropped);
        self.watched = watched;
        self.workspace_root = graph.workspace_root.clone();
        Ok(graph)
    }

    /// The text the program thread prints once the reload of `build` is applied: `reload:
    /// applied <files>`, with the number of instances compiled, then one note per split slot
    /// ("Contract Changes Are Reported").
    fn applied_message(&self, build: &Build, splits: &[(String, SlotKind)]) -> String {
        let paths: HashMap<&str, &Path> = build
            .plan
            .units
            .iter()
            .map(|unit| (unit.module.as_str(), unit.path.as_path()))
            .collect();
        let files: Vec<String> = build
            .units
            .iter()
            .filter_map(|unit| match &unit.id {
                UnitId::File(module) => Some(self.shown(paths.get(module.as_str()).copied()?)),
                UnitId::Instance(_) => None,
            })
            .collect();
        let instances = build
            .units
            .iter()
            .filter(|unit| matches!(unit.id, UnitId::Instance(_)))
            .count();
        let mut message = format!("reload: applied {}", files.join(", "));
        match instances {
            0 => {}
            1 => message.push_str(" (1 instance)"),
            n => message.push_str(&format!(" ({} instances)", n)),
        }
        message.push('\n');
        for (symbol, kind) in splits {
            message.push_str(&match kind {
                SlotKind::Function => format!(
                    "reload: the signature of {} changed; code already running keeps calling its previous version\n",
                    symbol
                ),
                SlotKind::Closure => format!(
                    "reload: the captures of {} changed; values made before keep calling its previous version\n",
                    symbol
                ),
            });
        }
        message
    }

    /// `path` relative to the workspace root, with `/` separators.
    fn shown(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.workspace_root).unwrap_or(path);
        relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Runs the session following `script` (D16): starts the program, applies each group of
    /// edits at its trigger, and ends once no step is left and no program runs.
    pub fn run_script(&mut self, script: EditScript) -> i32 {
        self.install_stop_handler();
        self.install_line_hook();
        let mut groups = script.groups.into_iter().peekable();
        // The line a step waits for is set before the program can run past it: before it
        // starts, and before it continues from the line of the step before.
        self.arm(groups.peek());
        self.start();
        loop {
            let Some(group) = groups.peek() else {
                if !self.running {
                    return self.exit_status();
                }
                self.wait_for_end();
                continue;
            };
            match group.trigger {
                Trigger::Line(line) => {
                    if !self.running {
                        eprintln!(
                            "watch: the edit script's step \"after line {}\" was never reached",
                            line
                        );
                        return 1;
                    }
                    match self.next_event() {
                        Some(SessionEvent::LineWritten(_)) => {
                            let group = groups.next().expect("a group is next");
                            let changed = self.apply_edits(group);
                            self.reload(&changed, Instant::now());
                            self.arm(groups.peek());
                            self.resume();
                        }
                        Some(SessionEvent::MainReturned(status)) => {
                            self.program_ended(status, false)
                        }
                        Some(SessionEvent::Stopped(status)) => self.program_ended(status, true),
                        Some(SessionEvent::FilesChanged(..)) => {}
                        None => return self.exit_status(),
                    }
                }
                Trigger::Exit => {
                    if self.running {
                        self.wait_for_end();
                        continue;
                    }
                    let group = groups.next().expect("a group is next");
                    let changed = self.apply_edits(group);
                    self.arm(groups.peek());
                    self.reload(&changed, Instant::now());
                }
            }
        }
    }

    /// Stops the program at the line `next` waits for, if it is an `after line` step.
    fn arm(&self, next: Option<&EditGroup>) {
        if let Some(EditGroup {
            trigger: Trigger::Line(line),
            ..
        }) = next
        {
            self.stop_at_line(*line);
        }
    }

    /// Waits until the running program's `main` returns or it stops.
    fn wait_for_end(&mut self) {
        while self.running {
            match self.next_event() {
                Some(SessionEvent::MainReturned(status)) => self.program_ended(status, false),
                Some(SessionEvent::Stopped(status)) => self.program_ended(status, true),
                Some(SessionEvent::LineWritten(_)) => self.resume(),
                Some(SessionEvent::FilesChanged(..)) => {}
                None => return,
            }
        }
    }

    /// Applies the edits of `group` to the overlay, returning their targets.
    fn apply_edits(&mut self, group: EditGroup) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        for edit in group.edits {
            self.overlay(edit.target.clone(), edit.content);
            changed.push(edit.target);
        }
        changed
    }

    /// Reports the watched files that changed on disk since the last build read them, as a
    /// change first noticed now. A file saved after the build read it but before its directory
    /// was registered with the watcher has no event, so the session looks once its watch is
    /// registered; a change that also has an event is reloaded once, since the second report
    /// finds the content already read. Files the store holds no content for, or has marked to
    /// read again, are left to their events, so a build that stopped before reading a file
    /// does not report it again and again.
    fn catch_up(&mut self) {
        let changed: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|path| !self.store.is_stale(path))
            .filter(
                |path| match (self.store.content(path), self.store.current_content(path)) {
                    (Some(before), Ok(now)) => before != now,
                    _ => false,
                },
            )
            .cloned()
            .collect();
        if !changed.is_empty() {
            let _ = self
                .sender
                .send(SessionEvent::FilesChanged(changed, Instant::now()));
        }
    }

    /// Runs the session on file-system events (D15), until the process ends.
    pub fn run_watching(&mut self) -> i32 {
        self.install_stop_handler();
        let mut watcher = FileWatcher::start(self.sender());
        self.start();
        watcher.watch(&self.watched);
        self.catch_up();
        loop {
            match self.next_event() {
                Some(SessionEvent::FilesChanged(paths, detected)) => {
                    self.reload(&paths, detected);
                    watcher.watch(&self.watched);
                    self.catch_up();
                }
                Some(SessionEvent::MainReturned(status)) => self.program_ended(status, false),
                Some(SessionEvent::Stopped(status)) => self.program_ended(status, true),
                Some(SessionEvent::LineWritten(_)) => self.resume(),
                None => return self.exit_status(),
            }
        }
    }
}

/// The file a resolution error is about, if it names one.
fn error_file(error: &ResolveError) -> Option<&Path> {
    match error {
        ResolveError::PackagePathMismatch { file, .. }
        | ResolveError::PackageHeterogeneity { file, .. }
        | ResolveError::InconsistentWorkspaceRoot { file, .. }
        | ResolveError::ReservedPackage { file, .. } => Some(file),
        ResolveError::FileNotFound { target_path, .. } => Some(target_path),
        ResolveError::Io { path, .. } | ResolveError::Parse { path, .. } => Some(path),
        ResolveError::WorkspaceBoundaryViolation { .. }
        | ResolveError::InvalidImportPath { .. }
        | ResolveError::LibraryImportNotFound { .. } => None,
    }
}

// --- Edit scripts (D16)

/// When a group of edits is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// When the program writes its `n`-th newline of the current run.
    Line(u64),
    /// Once no program runs.
    Exit,
}

/// One edit: `target` reads as `content`, the replacement file's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub target: PathBuf,
    pub content: String,
}

/// Consecutive steps with the same trigger, applied as one reload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditGroup {
    pub trigger: Trigger,
    pub edits: Vec<Edit>,
}

/// An edit script: its groups, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditScript {
    pub groups: Vec<EditGroup>,
}

impl EditScript {
    /// Reads the script at `path`. Paths in it are relative to its directory.
    pub fn parse(path: &Path) -> Result<EditScript, String> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!("cannot read the edit script {}: {}", path.display(), error)
        })?;
        let directory = path.parent().unwrap_or(Path::new("."));
        Self::parse_text(&text, directory, &|replacement: &Path| {
            std::fs::read_to_string(replacement).map_err(|error| error.to_string())
        })
        .map_err(|error| format!("{}: {}", path.display(), error))
    }

    /// Parses the script `text`, whose paths are relative to `directory`, reading replacements
    /// with `read`.
    pub fn parse_text(
        text: &str,
        directory: &Path,
        read: &dyn Fn(&Path) -> Result<String, String>,
    ) -> Result<EditScript, String> {
        let mut groups: Vec<EditGroup> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let number = index + 1;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = || format!("line {} is not an edit step: '{}'", number, line);
            let (trigger, action) = line.split_once(':').ok_or_else(bad)?;
            let trigger = match trigger.trim().strip_prefix("after").map(str::trim) {
                Some("exit") => Trigger::Exit,
                Some(rest) => {
                    let count = rest.strip_prefix("line").ok_or_else(bad)?.trim();
                    match count.parse::<u64>() {
                        Ok(count) if count > 0 => Trigger::Line(count),
                        _ => return Err(bad()),
                    }
                }
                None => return Err(bad()),
            };
            let words: Vec<&str> = action.split_whitespace().collect();
            let [verb, target, replacement] = words[..] else {
                return Err(bad());
            };
            if verb != "edit" {
                return Err(bad());
            }
            let target = prism_resolver::absolute_path(&directory.join(target));
            let replacement = directory.join(replacement);
            let content = read(&replacement).map_err(|error| {
                format!(
                    "line {}: cannot read {}: {}",
                    number,
                    replacement.display(),
                    error
                )
            })?;
            let edit = Edit { target, content };
            match groups.last_mut() {
                Some(group) if group.trigger == trigger => group.edits.push(edit),
                _ => groups.push(EditGroup {
                    trigger,
                    edits: vec![edit],
                }),
            }
        }
        Ok(EditScript { groups })
    }
}

// --- File watching (D15)

/// The quiet period after the last relevant event before a change is reported.
const QUIET: Duration = Duration::from_millis(10);
/// The longest a change waits after its first event.
const LONGEST_WAIT: Duration = Duration::from_millis(100);
/// The interval of the polling fallback.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Watches the directories that hold the program's files, non-recursively, and reports changes
/// of those files as [`SessionEvent::FilesChanged`], debounced: after the first relevant event
/// it waits for [`QUIET`] without events, and at most [`LONGEST_WAIT`] (D15). Watching
/// directories rather than files sees editors that save by renaming a temporary file over the
/// original. When the platform's events are unavailable it polls.
pub struct FileWatcher {
    watcher: Box<dyn notify::Watcher + Send>,
    directories: HashSet<PathBuf>,
    /// Each watched file by its canonical spelling, which events use, with the session's
    /// spelling.
    files: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
}

impl FileWatcher {
    /// A watcher reporting to `events`, with nothing watched yet.
    pub fn start(events: Sender<SessionEvent>) -> FileWatcher {
        let (sender, receiver) = mpsc::channel::<notify::Result<notify::Event>>();
        let files: Arc<Mutex<HashMap<PathBuf, PathBuf>>> = Arc::default();
        let debounced = Arc::clone(&files);
        std::thread::Builder::new()
            .name("watch".to_string())
            .spawn(move || debounce(receiver, debounced, events))
            .expect("the watcher's thread starts");
        let native = notify::recommended_watcher(sender.clone());
        let watcher: Box<dyn notify::Watcher + Send> = match native {
            Ok(watcher) => Box::new(watcher),
            Err(_) => {
                eprintln!(
                    "watch: file events are unavailable; polling every {} ms",
                    POLL_INTERVAL.as_millis()
                );
                let config = notify::Config::default().with_poll_interval(POLL_INTERVAL);
                Box::new(
                    notify::PollWatcher::new(sender, config).expect("the polling watcher starts"),
                )
            }
        };
        FileWatcher {
            watcher,
            directories: HashSet::new(),
            files,
        }
    }

    /// Watches exactly `files`, registering the directories that hold them and dropping those
    /// that hold none any more.
    pub fn watch(&mut self, files: &[PathBuf]) {
        let mut canonical = HashMap::new();
        let mut directories = HashSet::new();
        for file in files {
            let Some(parent) = file.parent() else {
                continue;
            };
            let Ok(directory) = parent.canonicalize() else {
                continue;
            };
            if let Some(name) = file.file_name() {
                canonical.insert(directory.join(name), file.clone());
            }
            directories.insert(directory);
        }
        *self
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = canonical;
        for directory in self.directories.difference(&directories) {
            let _ = self.watcher.unwatch(directory);
        }
        for directory in directories.difference(&self.directories) {
            let _ = self
                .watcher
                .watch(directory, notify::RecursiveMode::NonRecursive);
        }
        self.directories = directories;
    }
}

/// The canonical spelling of `path`: its directory canonicalized, then its name, so that a
/// deleted or renamed file still has one.
fn canonical(path: &Path) -> Option<PathBuf> {
    let directory = path.parent()?.canonicalize().ok()?;
    Some(directory.join(path.file_name()?))
}

/// The watcher's thread: collects the events of watched files and reports each change once it
/// has been quiet for [`QUIET`], or [`LONGEST_WAIT`] after its first event.
fn debounce(
    receiver: Receiver<notify::Result<notify::Event>>,
    files: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
    events: Sender<SessionEvent>,
) {
    let relevant = |event: notify::Result<notify::Event>| -> Vec<PathBuf> {
        let Ok(event) = event else {
            return Vec::new();
        };
        if event.kind.is_access() {
            return Vec::new();
        }
        let files = files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        event
            .paths
            .iter()
            .filter_map(|path| files.get(&canonical(path)?).cloned())
            .collect()
    };
    while let Ok(event) = receiver.recv() {
        let mut changed: Vec<PathBuf> = relevant(event);
        if changed.is_empty() {
            continue;
        }
        let first = Instant::now();
        loop {
            let left = LONGEST_WAIT.saturating_sub(first.elapsed());
            if left.is_zero() {
                break;
            }
            match receiver.recv_timeout(QUIET.min(left)) {
                Ok(event) => changed.extend(relevant(event)),
                Err(_) => break,
            }
        }
        changed.sort();
        changed.dedup();
        if events
            .send(SessionEvent::FilesChanged(changed, first))
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<EditScript, String> {
        EditScript::parse_text(text, Path::new("/case"), &|path: &Path| {
            Ok(format!("content of {}", path.display()))
        })
    }

    #[test]
    fn steps_are_parsed_and_grouped_by_trigger() {
        let script = parse(
            "# a comment\n\
             after line 2: edit lib/math.prism lib/math.prism.v2\n\
             \n\
             after line 2: edit app/main.prism app/main.prism.v2\n\
             after exit: edit app/main.prism app/main.prism.v3\n\
             after line 1: edit main.prism main.prism.v4\n",
        )
        .unwrap();
        let triggers: Vec<(Trigger, usize)> = script
            .groups
            .iter()
            .map(|group| (group.trigger, group.edits.len()))
            .collect();
        assert_eq!(
            triggers,
            [
                (Trigger::Line(2), 2),
                (Trigger::Exit, 1),
                (Trigger::Line(1), 1)
            ]
        );
        let first = &script.groups[0].edits[0];
        assert_eq!(first.target, Path::new("/case/lib/math.prism"));
        assert_eq!(first.content, "content of /case/lib/math.prism.v2");
    }

    #[test]
    fn bad_lines_are_errors_naming_the_line() {
        for (text, line) in [
            ("after line two: edit a b\n", 1),
            ("\nbefore exit: edit a b\n", 2),
            ("after line 0: edit a b\n", 1),
            ("after exit edit a b\n", 1),
            ("after exit: change a b\n", 1),
            ("after exit: edit a\n", 1),
            ("after exit: edit a b c\n", 1),
        ] {
            let error = parse(text).unwrap_err();
            assert!(
                error.starts_with(&format!("line {} ", line)),
                "{:?}: {}",
                text,
                error
            );
        }
        let missing = EditScript::parse_text("after exit: edit a b\n", Path::new("/case"), &|_| {
            Err("no such file".to_string())
        })
        .unwrap_err();
        assert!(missing.contains("no such file"), "{}", missing);
    }
}
