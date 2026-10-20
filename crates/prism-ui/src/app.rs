//! Apps and the frame loop (D14 and D15 of `ui-toolkit`).
//!
//! [`run_app`] runs on the program thread:
//!
//! 1. It rejects a second app on the thread, and takes the configured host, without which it
//!    is `ui misuse: no window system is available; run with --headless`.
//! 2. It gives the thread a tree with a window root of the host's size, and starts the
//!    composition with [`UiApplier`] (the first pass).
//! 3. It loops: at the top of each iteration, its reload point applies a pending reload,
//!    whose hook invalidates the scopes it affects (D2 of `compose-hot-reload`); while a scope
//!    is invalid, it runs a pass and then releases the handler entries no node claimed (D23);
//!    while layout is dirty, it lays out; otherwise it takes the host's next event and
//!    dispatches it, which a pass and a layout follow before the next event (D14). A frame
//!    rebuilds the display list when it is stale and presents it; a dump prints; a wake-up
//!    only returns to the top, so a reload that woke the loop is applied and recomposed before
//!    the frame the host gives after it; the end of events stops. While the app runs, reloads are deferred to that reload point, so none
//!    lands during a pass, a handler, layout or drawing.
//! 4. It disposes of the composition, which removes and releases every node, then releases
//!    the handler table's entries and the tree, and returns.
//!
//! With frame statistics on (`--frame-stats`, D7 of `reactive-app-verification`), the loop
//! times each stage and accumulates its iterations into frames, a present ending each, and
//! `runApp` prints the report when it returns ([`crate::statistics`]). Off, each stage costs
//! one branch.
//!
//! An idle app blocks in the host and runs no pass, layout or display list build. The app's
//! own fields (host, measurer, dispatcher, layout buffers, display list) are this loop's; what
//! the setters and the applier reach (the tree, the handler table, the app's composition) is
//! the thread's state ([`crate::thread`]).

use crate::applier::UiApplier;
use crate::draw::DisplayList;
use crate::dump::{dump_display_list, dump_tree};
use crate::error::{UiResult, APP_RUNNING, NO_WINDOW_SYSTEM};
use crate::handlers::release_functions;
use crate::headless::{
    missing_kind, missing_target, no_focus, resolve_kind_target, resolve_text_target,
};
use crate::host::{
    call_final_frame_sink, configured_host, reload_timings, wait_for_reload, wake_for_reload,
    DumpKind, Host, HostEvent, Key,
};
use crate::input::{Action, Dispatcher};
use crate::layout::Layout;
use crate::runtime;
use crate::script::{ScriptCommand, ScriptLine, ScriptTarget};
use crate::statistics::{FrameStatistics, Stage};
use crate::text::TextMeasurer;
use crate::thread::{install_tree, release_dropped, with_state, RunningApp, State};
use prism_compose::{Composition, Handle, RootContent};
use prism_runtime::allocation_counter::thread_counts;
use prism_runtime::reload;
use std::time::{Duration, Instant};

/// What an app did, for tests of idling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub passes: u64,
    pub layouts: u64,
    pub builds: u64,
    pub events: u64,
}

/// Where dumps go: the program's output unless a test says otherwise.
pub type Output = Box<dyn FnMut(&str)>;

/// A running app.
pub struct App {
    host: Box<dyn Host>,
    measurer: Box<dyn TextMeasurer>,
    composition: Handle,
    dispatcher: Dispatcher,
    layout: Layout,
    list: DisplayList,
    dump: String,
    output: Output,
    pub counters: Counters,
    /// The frame statistics, when they are on.
    statistics: Option<Box<FrameStatistics>>,
    /// When `runApp` was called, and when the run started, for the first frame's times.
    called: Instant,
    run_started: Instant,
    /// Set by a present, which ends the frame at the end of its iteration.
    frame_presented: bool,
    /// The time of the current iteration spent in what is not measured: dumps, edits and
    /// waits for reloads.
    excluded: Duration,
}

/// Runs an app titled `title` of `width` × `height` with the configured host, composing
/// `content`, which it owns, until the host's events end.
pub fn run_app(
    title: &str,
    width: i64,
    height: i64,
    content: impl Into<RootContent>,
) -> UiResult<()> {
    let content = content.into();
    let called = Instant::now();
    if with_state(|state| state.app.is_some()) {
        release_functions(&mut vec![content.into_function()]);
        return Err(APP_RUNNING);
    }
    // `runApp` reached during a pass, through a function value or a wrapper that `E4030` does
    // not see, is the composition misuse before any window opens (compose, "One Pass at a
    // Time").
    if prism_compose::active_composer().is_some() {
        release_functions(&mut vec![content.into_function()]);
        return Err(prism_compose::composer::DURING_PASS.into());
    }
    let Some(host) = configured_host(title, width, height) else {
        release_functions(&mut vec![content.into_function()]);
        return Err(NO_WINDOW_SYSTEM);
    };
    let measurer = host.measurer();
    run_app_from(host, measurer, content, called).map(|_| ())
}

/// Runs an app against `host`, measuring text with `measurer`, and gives what it did. When the
/// app ends normally, the final-frame sink sees its last display list.
pub fn run_app_with(
    host: Box<dyn Host>,
    measurer: Box<dyn TextMeasurer>,
    content: impl Into<RootContent>,
) -> UiResult<Counters> {
    run_app_from(host, measurer, content.into(), Instant::now())
}

/// [`run_app_with`] of an app whose `runApp` was called at `called`.
fn run_app_from(
    host: Box<dyn Host>,
    measurer: Box<dyn TextMeasurer>,
    content: RootContent,
    called: Instant,
) -> UiResult<Counters> {
    // Reloads wait for the frame loop's reload point while the app runs, and apply at the
    // program's next call again once it ends, a pending one included (D2 of
    // `compose-hot-reload`).
    reload::defer_to_reload_points(wake_for_reload);
    let result = run_deferring(host, measurer, content, called);
    reload::apply_at_next_call();
    result
}

fn run_deferring(
    host: Box<dyn Host>,
    measurer: Box<dyn TextMeasurer>,
    content: RootContent,
    called: Instant,
) -> UiResult<Counters> {
    // Recompositions are counted only for a script that dumps them (D14 of
    // `strong-skipping`), from the first composition on.
    let counting = crate::host::counts_recompositions();
    if counting {
        let keys: Vec<u64> = crate::host::recomposition_names()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        prism_compose::counts::enable(&keys);
    }
    let started = App::start_at(host, measurer, content, called);
    let mut app = match started {
        Ok(app) => app,
        Err(error) => {
            if counting {
                prism_compose::counts::disable();
            }
            return Err(error);
        }
    };
    let result = app.run();
    // The final frame is not measured.
    let statistics = app.statistics.take();
    if result.is_ok() {
        app.present_final_frame();
    }
    let window = match &statistics {
        Some(_) => app.host.frame_statistics_lines(),
        None => Vec::new(),
    };
    let counters = app.counters;
    app.finish();
    if counting {
        prism_compose::counts::disable();
    }
    // The report comes after the composition is dropped, and only for an app that ended
    // normally: a runtime error ends the program instead.
    if let (Some(statistics), Ok(())) = (statistics, &result) {
        prism_runtime::flush_output();
        eprint!("{}", statistics.report(&window));
    }
    result.map(|()| counters)
}

impl App {
    /// Starts an app: its tree, its composition and the first pass.
    pub fn start(
        host: Box<dyn Host>,
        measurer: Box<dyn TextMeasurer>,
        content: impl Into<RootContent>,
    ) -> UiResult<App> {
        Self::start_at(host, measurer, content.into(), Instant::now())
    }

    /// Starts an app whose `runApp` was called at `called`, measuring its frames when frame
    /// statistics are on: the first frame holds the first composition.
    fn start_at(
        host: Box<dyn Host>,
        measurer: Box<dyn TextMeasurer>,
        content: RootContent,
        called: Instant,
    ) -> UiResult<App> {
        if with_state(|state| state.app.is_some()) {
            release_functions(&mut vec![content.into_function()]);
            return Err(APP_RUNNING);
        }
        let run_started = crate::host::frame_statistics();
        // Allocated before anything is measured, so it is not counted in the first frame.
        let mut statistics = run_started.map(|_| FrameStatistics::new(thread_counts().0));
        let composed = statistics.as_ref().map(|_| Instant::now());
        let (width, height) = host.size();
        install_tree(width, height);
        with_state(|state| state.app = Some(RunningApp { composition: None }));
        let composition = match Composition::start(Box::new(UiApplier), content) {
            Ok(composition) => composition,
            Err(misuse) => {
                Self::release_state();
                return Err(misuse.into());
            }
        };
        with_state(|state| {
            state.app = Some(RunningApp {
                composition: Some(composition),
            })
        });
        let mut app = App {
            host,
            measurer,
            composition,
            dispatcher: Dispatcher::new(),
            layout: Layout::new(),
            list: DisplayList::new(),
            dump: String::new(),
            output: Box::new(runtime::write_output),
            counters: Counters::default(),
            statistics: None,
            called,
            run_started: run_started.unwrap_or(called),
            frame_presented: false,
            excluded: Duration::ZERO,
        };
        app.end_of_pass();
        if let (Some(statistics), Some(composed)) = (statistics.as_mut(), composed) {
            let elapsed = composed.elapsed();
            statistics.passes(1);
            statistics.stage(Stage::Recomposition, elapsed);
            statistics.busy(elapsed);
        }
        app.statistics = statistics;
        Ok(app)
    }

    /// The current instant, when frames are measured.
    fn now(&self) -> Option<Instant> {
        self.statistics.as_ref().map(|_| Instant::now())
    }

    /// Adds the time since `started` to the current frame's stage, when frames are measured.
    fn record(&mut self, stage: Stage, started: Option<Instant>) {
        if let (Some(statistics), Some(started)) = (self.statistics.as_mut(), started) {
            statistics.stage(stage, started.elapsed());
        }
    }

    /// Ends the current frame with the program thread's allocation counts.
    fn end_frame(&mut self) {
        if let Some(statistics) = self.statistics.as_mut() {
            let (allocations, frees) = thread_counts();
            statistics.end_frame(allocations, frees);
        }
    }

    /// Runs `f`, which the statistics do not measure: neither its time nor its allocations
    /// count in the frame (dumps, edits and waits for reloads).
    fn unmeasured<R>(&mut self, f: impl FnOnce(&mut App) -> R) -> R {
        let Some(statistics) = self.statistics.take() else {
            return f(self);
        };
        let started = Instant::now();
        let before = thread_counts().0;
        let result = f(self);
        let mut statistics = statistics;
        statistics.exclude_allocations(thread_counts().0 - before);
        self.excluded += started.elapsed();
        self.statistics = Some(statistics);
        result
    }

    /// The frame statistics, when they are on.
    pub fn frame_statistics(&self) -> Option<&FrameStatistics> {
        self.statistics.as_deref()
    }

    /// Sends dumps to `output` instead of the program's output.
    pub fn set_output(&mut self, output: Output) {
        self.output = output;
    }

    /// The app's composition.
    pub fn composition(&self) -> Handle {
        self.composition
    }

    /// The display list as last built.
    pub fn display_list(&self) -> &DisplayList {
        &self.list
    }

    /// What the end of each pass does: release the unclaimed handler entries and drop what
    /// was released, outside every borrow.
    fn end_of_pass(&mut self) {
        with_state(|state| {
            let State {
                handlers, drops, ..
            } = state;
            handlers.release_unclaimed(drops);
        });
        release_dropped();
        self.counters.passes += 1;
    }

    /// Runs passes while a scope is invalid, then lays out if anything is dirty.
    pub fn settle(&mut self) -> UiResult<()> {
        let started = self.now();
        let passes = self.counters.passes;
        while Composition::recompose(self.composition)? {
            self.end_of_pass();
        }
        let passes = self.counters.passes - passes;
        if passes > 0 {
            self.record(Stage::Recomposition, started);
            if let Some(statistics) = self.statistics.as_mut() {
                statistics.passes(passes);
            }
        }
        let started = self.now();
        let measurer = &*self.measurer;
        let layout = &mut self.layout;
        if with_state(|state| layout.run(&mut state.tree, measurer)) {
            self.counters.layouts += 1;
            self.record(Stage::Layout, started);
        }
        Ok(())
    }

    /// The loop: apply a pending reload, settle, then take and dispatch the next event, until
    /// events end.
    pub fn run(&mut self) -> UiResult<()> {
        loop {
            let iteration = self.now();
            self.reload_point()?;
            self.settle()?;
            if let (Some(statistics), Some(iteration)) = (self.statistics.as_mut(), iteration) {
                statistics.first_frame_ready(self.called.elapsed(), self.run_started.elapsed());
                statistics.busy(iteration.elapsed());
            }
            // Waiting for the next event is not busy time.
            let Some(event) = self.host.next_event() else {
                // Work after the last present forms a last frame.
                if self.statistics.as_ref().is_some_and(|s| s.has_work()) {
                    self.end_frame();
                }
                return Ok(());
            };
            let handled = self.now();
            self.excluded = Duration::ZERO;
            self.handle(event)?;
            if let (Some(statistics), Some(handled)) = (self.statistics.as_mut(), handled) {
                statistics.busy(handled.elapsed().saturating_sub(self.excluded));
            }
            if std::mem::take(&mut self.frame_presented) {
                self.end_frame();
            }
        }
    }

    /// The reload point at the top of the loop (D2 of `compose-hot-reload`): applies the
    /// pending reload, if any, whose hook invalidates the scopes it affects, and runs the
    /// passes that follow it, timed under `--timings`.
    pub fn reload_point(&mut self) -> UiResult<()> {
        let applying = self.now();
        if reload::reload_point().is_none() {
            return Ok(());
        }
        self.record(Stage::Reload, applying);
        if let Some(statistics) = self.statistics.as_mut() {
            statistics.reload();
        }
        let started = Instant::now();
        self.settle()?;
        let scopes = prism_compose::reload::take_invalidated_scopes();
        if reload_timings() {
            eprintln!(
                "timing: reload pass {:.3} ms ({} scopes)",
                started.elapsed().as_secs_f64() * 1000.0,
                scopes
            );
        }
        Ok(())
    }

    /// Dispatches one event.
    pub fn handle(&mut self, event: HostEvent) -> UiResult<()> {
        self.counters.events += 1;
        match event {
            HostEvent::Frame => {
                self.build_display_list(false);
                self.host.present(&self.list);
                self.frame_presented = self.statistics.is_some();
            }
            HostEvent::Dump(kind) => self.unmeasured(|app| app.print_dump(kind)),
            HostEvent::Script(line) => return self.script(line),
            // Back to the loop's top, whose reload point and passes run before the frame the
            // host makes due after a wake-up, so that frame shows the new code.
            HostEvent::Wake => {}
            input => self.input(input),
        }
        Ok(())
    }

    /// Dispatches a press, release, character, key or resize: an input event of the frame
    /// statistics, timed as their `events` stage.
    fn input(&mut self, event: HostEvent) {
        let started = self.now();
        match event {
            HostEvent::Press(x, y) => self.press(x, y),
            HostEvent::Release(x, y) => self.release(x, y),
            HostEvent::Character(character) => {
                let dispatcher = &mut self.dispatcher;
                let action = with_state(|state| dispatcher.character(&state.tree, character));
                self.perform(action);
            }
            HostEvent::Key(key) => self.key(key),
            HostEvent::Resize(width, height) => {
                with_state(|state| state.tree.resize(width, height))
            }
            HostEvent::Frame | HostEvent::Dump(_) | HostEvent::Script(_) | HostEvent::Wake => {
                return
            }
        }
        self.record(Stage::Events, started);
        if let Some(statistics) = self.statistics.as_mut() {
            statistics.input_event();
        }
    }

    fn press(&mut self, x: i64, y: i64) {
        let dispatcher = &mut self.dispatcher;
        with_state(|state| {
            if dispatcher.press(&state.tree, x, y) {
                state.tree.redraw = true;
            }
        });
    }

    fn release(&mut self, x: i64, y: i64) {
        let dispatcher = &mut self.dispatcher;
        let action = with_state(|state| {
            let (action, focus_changed) = dispatcher.release(&state.tree, x, y);
            if focus_changed {
                state.tree.redraw = true;
            }
            action
        });
        self.perform(action);
    }

    fn key(&mut self, key: Key) {
        let dispatcher = &mut self.dispatcher;
        let action = with_state(|state| dispatcher.key(&state.tree, key));
        self.perform(action);
    }

    /// Calls an action's handler as a handler of the app's composition, with its function
    /// value copied out of its entry and no state borrowed (D2, D14).
    fn perform(&mut self, action: Option<Action>) {
        let Some(action) = action else {
            return;
        };
        let function = with_state(|state| state.handlers.function(action.handler()));
        if let Some(function) = function {
            // SAFETY: the entry is live and its type was checked when its node claimed it; no
            // pass or release runs before the call returns.
            runtime::run_handler(self.composition, || unsafe { action.perform(&function) });
        }
    }

    /// Rebuilds the display list when it is stale, or always when `force` is set.
    pub fn build_display_list(&mut self, force: bool) {
        let started = self.now();
        let measurer = &*self.measurer;
        let list = &mut self.list;
        let dispatcher = &mut self.dispatcher;
        let built = with_state(|state| {
            if !force && !state.tree.redraw {
                return false;
            }
            let focused = dispatcher.focused(&state.tree);
            list.build(&state.tree, measurer, focused);
            state.tree.redraw = false;
            true
        });
        if built {
            self.counters.builds += 1;
            self.record(Stage::DisplayList, started);
        }
    }

    /// Gives the final-frame sink the display list as the app ends, rebuilt if stale, with the
    /// window's size and the host's scale factor (`desktop-window` D15).
    pub fn present_final_frame(&mut self) {
        self.build_display_list(false);
        let size = with_state(|state| state.tree.window_size());
        call_final_frame_sink(
            &self.list,
            (size.width, size.height),
            self.host.scale_factor(),
        );
    }

    fn print_dump(&mut self, kind: DumpKind) {
        self.dump.clear();
        match kind {
            DumpKind::Tree => {
                let dispatcher = &mut self.dispatcher;
                let dump = &mut self.dump;
                with_state(|state| {
                    let focused = dispatcher.focused(&state.tree);
                    dump_tree(&state.tree, focused, dump);
                });
            }
            DumpKind::Draw => {
                self.build_display_list(false);
                dump_display_list(&self.list, &mut self.dump);
            }
            DumpKind::Recompositions => {
                let names: std::collections::HashMap<u64, String> =
                    crate::host::recomposition_names().into_iter().collect();
                let mut lines: Vec<(String, prism_compose::counts::Counts)> =
                    prism_compose::counts::take()
                        .into_iter()
                        .filter_map(|(key, counts)| Some((names.get(&key)?.clone(), counts)))
                        .collect();
                lines.sort_by(|a, b| a.0.cmp(&b.0));
                self.dump.push_str("recompositions\n");
                for (name, counts) in lines {
                    self.dump.push_str(&format!(
                        "  {} ran {} skipped {}\n",
                        name, counts.ran, counts.skipped
                    ));
                }
            }
        }
        (self.output)(&self.dump);
    }

    /// The point of a script target.
    fn point(&self, line: usize, target: &ScriptTarget) -> UiResult<(i64, i64)> {
        match target {
            ScriptTarget::Point(x, y) => Ok((*x, *y)),
            ScriptTarget::Text { text, occurrence } => {
                with_state(|state| resolve_text_target(&state.tree, text, *occurrence))
                    .ok_or_else(|| missing_target(line, text))
            }
            ScriptTarget::Kind { kind, occurrence } => {
                with_state(|state| resolve_kind_target(&state.tree, *kind, *occurrence))
                    .ok_or_else(|| missing_kind(line, *kind, *occurrence))
            }
        }
    }

    fn require_focus(&mut self, line: usize) -> UiResult<()> {
        let dispatcher = &mut self.dispatcher;
        match with_state(|state| dispatcher.focused(&state.tree)) {
            Some(_) => Ok(()),
            None => Err(no_focus(line)),
        }
    }

    /// Runs one script line, each event in it followed by a pass and a layout.
    fn script(&mut self, line: ScriptLine) -> UiResult<()> {
        let number = line.line;
        match line.command {
            ScriptCommand::Click(target) => {
                let (x, y) = self.point(number, &target)?;
                self.input(HostEvent::Press(x, y));
                self.settle()?;
                self.input(HostEvent::Release(x, y));
            }
            ScriptCommand::Press(target) => {
                let (x, y) = self.point(number, &target)?;
                self.input(HostEvent::Press(x, y));
            }
            ScriptCommand::Release(target) => {
                let (x, y) = self.point(number, &target)?;
                self.input(HostEvent::Release(x, y));
            }
            ScriptCommand::Type(text) => {
                for (index, character) in text.chars().enumerate() {
                    if index > 0 {
                        self.settle()?;
                    }
                    self.require_focus(number)?;
                    self.handle(HostEvent::Character(character))?;
                }
            }
            ScriptCommand::Key(key) => {
                self.require_focus(number)?;
                self.input(HostEvent::Key(key));
            }
            ScriptCommand::Resize(width, height) => self.input(HostEvent::Resize(width, height)),
            ScriptCommand::Frame => self.handle(HostEvent::Frame)?,
            // A person's pause: the thread sleeps, in no frame (D15 of `strong-skipping`).
            ScriptCommand::Pause(milliseconds) => {
                self.unmeasured(|_| std::thread::sleep(Duration::from_millis(milliseconds)))
            }
            ScriptCommand::Dump(kind) => self.unmeasured(|app| app.print_dump(kind)),
            // The session requests the reload, which the loop's reload point applies next
            // (D12 of `compose-hot-reload`). The session's work is not the app's.
            ScriptCommand::Edit(edits) => self.unmeasured(|_| reload::request_scripted_edit(edits)),
            ScriptCommand::WaitReload => {
                // What the program printed so far is seen before the loop blocks.
                prism_runtime::flush_output();
                if !self.unmeasured(|_| wait_for_reload()) {
                    return Err(crate::error::UiError::Script {
                        line: number,
                        detail: "no reload arrived within 10 seconds".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Ends the app: disposes of its composition, then releases every handler entry and node
    /// left, and the tree.
    pub fn finish(self) {
        let _ = Composition::dispose(self.composition);
        release_dropped();
        Self::release_state();
    }

    fn release_state() {
        with_state(|state| {
            let State {
                tree,
                handlers,
                drops,
                ..
            } = state;
            tree.release_all(handlers, drops);
            handlers.release_all(drops);
            state.app = None;
        });
        release_dropped();
        install_tree(0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{HANDLER_WITHOUT_APP, OUTSIDE_COMPOSITION};
    use crate::handle::NodeId;
    use crate::handlers::HandlerKind;
    use crate::headless::HeadlessHost;
    use crate::host::{configure, HostConfiguration};
    use crate::modifier::{Element, ModifierMirror};
    use crate::properties::{Role, Shape};
    use crate::script::parse_script;
    use crate::setters;
    use crate::test_support::*;
    use crate::text::FixedAdvanceMetrics;
    use prism_compose::test_support::content;
    use prism_compose::{invalidate, Composer};
    use prism_runtime::FunctionValue;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    fn log() -> Log {
        Rc::new(RefCell::new(Vec::new()))
    }

    /// A host yielding `events`.
    struct ScriptedHost {
        size: (i64, i64),
        events: std::vec::IntoIter<HostEvent>,
        presented: Rc<Cell<u64>>,
    }

    impl Host for ScriptedHost {
        fn size(&self) -> (i64, i64) {
            self.size
        }

        fn next_event(&mut self) -> Option<HostEvent> {
            self.events.next()
        }

        fn present(&mut self, _list: &DisplayList) {
            self.presented.set(self.presented.get() + 1);
        }
    }

    fn scripted(events: Vec<HostEvent>) -> (Box<dyn Host>, Rc<Cell<u64>>) {
        let presented = Rc::new(Cell::new(0));
        let host = ScriptedHost {
            size: (400, 300),
            events: events.into_iter(),
            presented: presented.clone(),
        };
        (Box::new(host), presented)
    }

    /// Invalidates the running app's root scope, as a state write would.
    fn invalidate_app() {
        let composition = with_state(|state| state.app.unwrap().composition.unwrap());
        invalidate(Composition::root_scope(composition).unwrap());
    }

    /// `Button(onClick) { Text(label) }` as material3 builds it, keyed `key`.
    fn button(k: Composer, key: i64, label: &str, on_click: FunctionValue) {
        let node = NodeId(k.start_node_group(key, 3).unwrap().0);
        let handler = setters::register(HandlerKind::Click, on_click).unwrap();
        let chain = ModifierMirror::encode(&[
            background(0xFF67_50A4, Shape::new(0, 50)),
            clickable(handler, true, Some(Role::Button)),
        ]);
        setters::set_modifier(node, &chain).unwrap();
        setters::set_content_color(node, 0xFFFF_FFFF).unwrap();
        let row = NodeId(k.start_node_group(key + 1, 2).unwrap().0);
        let chain = ModifierMirror::encode(&[default_min_size(58, 40), padding_each(24, 8, 24, 8)]);
        setters::set_modifier(row, &chain).unwrap();
        setters::set_arrangement(row, 4, 0).unwrap();
        setters::set_alignment(row, 13).unwrap();
        text(k, key + 2, label);
        k.end_group().unwrap();
        k.end_group().unwrap();
    }

    /// `Text(text)`, keyed `key`.
    fn text(k: Composer, key: i64, text: &str) {
        let node = NodeId(k.start_node_group(key, 4).unwrap().0);
        setters::set_text(node, text).unwrap();
        k.end_group().unwrap();
    }

    /// The counter screen: a column holding `Count: <n>` and a button that increments `n`.
    fn counter(count: Rc<Cell<i64>>, log: Log) -> FunctionValue {
        content(move |k| {
            log.borrow_mut().push("compose".to_string());
            let column = NodeId(k.start_node_group(1, 1).unwrap().0);
            setters::set_modifier(column, &ModifierMirror::encode(&[padding(16)])).unwrap();
            setters::set_arrangement(column, 8, 8).unwrap();
            text(k, 2, &format!("Count: {}", count.get()));
            let count = count.clone();
            button(
                k,
                10,
                "+",
                handler("increment", move || {
                    count.set(count.get() + 1);
                    invalidate_app();
                }),
            );
            k.end_group().unwrap();
        })
    }

    fn headless(script: &str) -> Box<dyn Host> {
        Box::new(HeadlessHost::new((400, 300), parse_script(script).unwrap()))
    }

    fn run_with_output(
        host: Box<dyn Host>,
        content: FunctionValue,
    ) -> (UiResult<Counters>, String) {
        let output = Rc::new(RefCell::new(String::new()));
        let sink = output.clone();
        let result = (|| {
            let mut app = App::start(host, Box::new(FixedAdvanceMetrics), content)?;
            app.set_output(Box::new(move |text| sink.borrow_mut().push_str(text)));
            let result = app.run();
            let counters = app.counters;
            app.finish();
            result.map(|()| counters)
        })();
        let text = output.borrow().clone();
        (result, text)
    }

    #[test]
    fn an_idle_loop_runs_nothing() {
        let log = log();
        let (host, presented) =
            scripted(vec![HostEvent::Frame, HostEvent::Frame, HostEvent::Frame]);
        let counters = run_app_with(
            host,
            Box::new(FixedAdvanceMetrics),
            counter(Rc::new(Cell::new(0)), log.clone()),
        )
        .unwrap();
        assert_eq!(
            counters,
            Counters {
                passes: 1,
                layouts: 1,
                builds: 1,
                events: 3
            }
        );
        assert_eq!(presented.get(), 3, "every frame is presented");
        assert_eq!(*log.borrow(), ["compose"]);
    }

    /// `runApp`'s content is composable root content, called with the composer and the
    /// force bit (D4 of `compose-library-in-prism`), and released when the app ends.
    #[test]
    fn composable_content_runs_as_the_app_root() {
        let log = log();
        let seen = log.clone();
        let (host, _) = scripted(vec![HostEvent::Frame]);
        let content = prism_compose::test_support::composable_content(move |k, changed| {
            assert!(k.current_scope().is_ok());
            push_line(&seen, format!("compose {}", changed));
        });
        let counters = run_app_with(host, Box::new(FixedAdvanceMetrics), content).unwrap();
        assert_eq!(counters.passes, 1);
        assert_eq!(*log.borrow(), ["compose 1"]);
        assert_eq!(prism_compose::live_compositions_report(), None);
    }

    #[test]
    fn an_event_is_followed_by_exactly_one_pass() {
        let log = log();
        let count = Rc::new(Cell::new(0));
        let (host, _) = scripted(vec![
            HostEvent::Press(45, 64),
            HostEvent::Release(45, 64),
            HostEvent::Frame,
            HostEvent::Press(300, 200),
            HostEvent::Release(300, 200),
        ]);
        let counters = run_app_with(
            host,
            Box::new(FixedAdvanceMetrics),
            counter(count.clone(), log.clone()),
        )
        .unwrap();
        assert_eq!(count.get(), 1);
        assert_eq!(counters.passes, 2, "the first pass and one after the click");
        assert_eq!(counters.layouts, 2);
        assert_eq!(counters.builds, 1);
    }

    #[test]
    fn a_scripted_counter_session_with_dumps() {
        let count = Rc::new(Cell::new(0));
        let (result, output) = run_with_output(
            headless("# two clicks\nclick \"+\"\nclick \"+\"\ndump\npress \"+\"\nrelease 300 200\ndump draw"),
            counter(count.clone(), log()),
        );
        result.unwrap();
        assert_eq!(count.get(), 2);
        assert_eq!(
            output,
            "window 400x300\n\
             \x20 column #1 0,0 96x100 arrangement spacedBy(8) padding(16)\n\
             \x20   text #2 16,16 64x20 \"Count: 2\"\n\
             \x20   box #3 16,44 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)\n\
             \x20     row #4 16,44 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)\n\
             \x20       text #5 41,54 8x20 \"+\"\n\
             clear 400x300 #FFFFFFFF\n\
             text 16,16 size 16 #FF000000 \"Count: 2\"\n\
             rect 16,44 58x40 #FF6750A4 radius 20\n\
             text 41,54 size 16 #FFFFFFFF \"+\"\n"
        );
    }

    #[test]
    fn resizing_the_window() {
        let (result, output) = run_with_output(
            headless("resize 200 100\ndump"),
            content(|k| {
                let node = NodeId(k.start_node_group(1, 3).unwrap().0);
                let chain = ModifierMirror::encode(&[fill_max(true, true, 1.0)]);
                setters::set_modifier(node, &chain).unwrap();
                k.end_group().unwrap();
            }),
        );
        result.unwrap();
        assert_eq!(output, "window 200x100\n  box #1 0,0 200x100 fillMaxSize\n");
    }

    #[test]
    fn unclaimed_handlers_are_released_after_their_pass() {
        reset_drops();
        // Each pass registers a handler owning its environment: the node takes the new one
        // and releases the old, as the scenario "Replaced handlers are freed" does.
        let passes = Rc::new(Cell::new(0));
        let seen = passes.clone();
        let (host, _) = scripted(vec![
            HostEvent::Press(10, 10),
            HostEvent::Release(10, 10),
            HostEvent::Press(10, 10),
            HostEvent::Release(10, 10),
        ]);
        let names = ["first", "second", "third"];
        run_app_with(
            host,
            Box::new(FixedAdvanceMetrics),
            content(move |k| {
                let pass = seen.get();
                seen.set(pass + 1);
                let node = NodeId(k.start_node_group(1, 3).unwrap().0);
                let kept =
                    setters::register(HandlerKind::Click, handler(names[pass], invalidate_app))
                        .unwrap();
                // A handler made in the pass and never claimed.
                setters::register(HandlerKind::Click, handler("unclaimed", || {})).unwrap();
                let chain = ModifierMirror::encode(&[size(20, 20), clickable(kept, true, None)]);
                setters::set_modifier(node, &chain).unwrap();
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        assert_eq!(passes.get(), 3);
        assert_eq!(
            drops(),
            [
                "drop unclaimed",
                "drop first",
                "drop unclaimed",
                "drop second",
                "drop unclaimed",
                "drop third"
            ]
        );
    }

    #[test]
    fn equal_one_word_handlers_are_one_entry() {
        let held = Rc::new(RefCell::new(Vec::new()));
        let record = held.clone();
        let (host, _) = scripted(Vec::new());
        let mut app = App::start(
            host,
            Box::new(FixedAdvanceMetrics),
            content(move |k| {
                let node = NodeId(k.start_node_group(1, 3).unwrap().0);
                let id = setters::register(HandlerKind::Click, inline_handler(7)).unwrap();
                let chain = ModifierMirror::encode(&[size(20, 20), clickable(id, true, None)]);
                setters::set_modifier(node, &chain).unwrap();
                record.borrow_mut().push(with_state(|state| {
                    (
                        state.tree.node(node).unwrap().chain[1].element.handler(),
                        state.handlers.len(),
                    )
                }));
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        app.settle().unwrap();
        invalidate_app();
        app.settle().unwrap();
        assert_eq!(with_state(|state| state.handlers.len()), 1);
        let first = held.borrow()[0].0;
        assert_eq!(
            *held.borrow(),
            [(first, 1), (first, 1)],
            "the second registration gives the node's entry"
        );
        let (calls, _) = inline_calls();
        app.handle(HostEvent::Press(10, 10)).unwrap();
        app.handle(HostEvent::Release(10, 10)).unwrap();
        assert_eq!(inline_calls(), (calls + 1, 7));
        app.finish();
    }

    #[test]
    fn disposal_releases_every_node_and_handler() {
        reset_drops();
        let (host, _) = scripted(vec![HostEvent::Frame]);
        run_app_with(
            host,
            Box::new(FixedAdvanceMetrics),
            content(|k| {
                let column = NodeId(k.start_node_group(1, 1).unwrap().0);
                let _ = column;
                button(k, 10, "a", handler("a", || {}));
                button(k, 20, "b", handler("b", || {}));
                let field = NodeId(k.start_node_group(30, 6).unwrap().0);
                let edit =
                    setters::register(HandlerKind::Text, text_handler("edit", |_| {})).unwrap();
                setters::set_on_value_change(field, edit).unwrap();
                k.end_group().unwrap();
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        let mut dropped = drops();
        dropped.sort();
        assert_eq!(dropped, ["drop a", "drop b", "drop edit"]);
        with_state(|state| {
            assert!(state.tree.is_empty());
            assert!(state.handlers.is_empty());
            assert!(state.app.is_none());
        });
        assert_eq!(prism_compose::live_compositions_report(), None);
    }

    #[test]
    fn typing_runs_a_pass_after_each_character() {
        // The typing app: a field whose value is the program's state, and a text below.
        let value = Rc::new(RefCell::new(String::new()));
        let state = value.clone();
        let (result, output) = run_with_output(
            headless("click 100 18\ntype \"ab\"\nkey backspace\ntype \"c\"\ndump"),
            content(move |k| {
                let column = NodeId(k.start_node_group(1, 1).unwrap().0);
                let _ = column;
                let field = NodeId(k.start_node_group(2, 6).unwrap().0);
                let current = state.borrow().clone();
                setters::set_text(field, &current).unwrap();
                let writer = state.clone();
                let edit = setters::register(
                    HandlerKind::Text,
                    text_handler("edit", move |text| {
                        *writer.borrow_mut() = text.to_string();
                        invalidate_app();
                    }),
                )
                .unwrap();
                setters::set_on_value_change(field, edit).unwrap();
                let chain = ModifierMirror::encode(&[
                    default_min_size(200, -1),
                    Element::TextFieldFrame {
                        unfocused: crate::properties::Color(0xFF79_747E),
                        focused: crate::properties::Color(0xFF67_50A4),
                    },
                    padding(8),
                ]);
                setters::set_modifier(field, &chain).unwrap();
                k.end_group().unwrap();
                text(k, 3, &format!("Hello {}", current));
                k.end_group().unwrap();
            }),
        );
        result.unwrap();
        assert_eq!(*value.borrow(), "ac");
        assert_eq!(
            output,
            "window 400x300\n\
             \x20 column #1 0,0 200x56\n\
             \x20   textfield #2 0,0 200x36 \"ac\" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)\n\
             \x20   text #3 0,36 64x20 \"Hello ac\"\n"
        );
    }

    #[test]
    fn script_failures() {
        let (result, output) = run_with_output(
            headless("dump\nclick \"Minus\""),
            counter(Rc::new(Cell::new(0)), log()),
        );
        assert_eq!(
            result.unwrap_err().message(),
            "event script line 2: no node shows the text \"Minus\""
        );
        assert!(
            output.starts_with("window 400x300\n"),
            "earlier dumps are printed"
        );
        let (result, _) = run_with_output(
            headless("type \"x\""),
            counter(Rc::new(Cell::new(0)), log()),
        );
        assert_eq!(
            result.unwrap_err().message(),
            "event script line 1: no text field is focused"
        );
        let (result, _) =
            run_with_output(headless("key enter"), counter(Rc::new(Cell::new(0)), log()));
        assert_eq!(result.unwrap_err(), no_focus(1));
        with_state(|state| assert!(state.app.is_none() && state.tree.is_empty()));
    }

    static RELOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn label() -> &'static str {
        match RELOADED.load(std::sync::atomic::Ordering::SeqCst) {
            true => "new",
            false => "old",
        }
    }

    /// The child-process half of `a_reload_waits_for_the_running_handler`: a counter whose
    /// label is the code's version, and whose handler requests a reload that switches it.
    fn reload_in_handler_child() -> ! {
        prism_runtime::reload::set_applied_hook(Some(prism_compose::reload::reload_applied));
        let log = log();
        let count = Rc::new(Cell::new(0));
        let (content_log, handler_log) = (log.clone(), log.clone());
        let content = content(move |k| {
            push_line(&content_log, format!("compose {} {}", label(), count.get()));
            let column = NodeId(k.start_node_group(1, 1).unwrap().0);
            setters::set_modifier(column, &ModifierMirror::encode(&[padding(16)])).unwrap();
            text(k, 2, &format!("{} {}", label(), count.get()));
            let (count, handler_log) = (count.clone(), handler_log.clone());
            button(
                k,
                10,
                "+",
                handler("increment", move || {
                    count.set(count.get() + 1);
                    invalidate_app();
                    prism_runtime::reload::request(prism_runtime::reload::PendingReload::new(
                        || {
                            RELOADED.store(true, std::sync::atomic::Ordering::SeqCst);
                            prism_runtime::reload::AppliedReload {
                                // The column's node group, under the root.
                                composable_bodies: vec![1],
                                ..Default::default()
                            }
                        },
                    ));
                    push_line(&handler_log, format!("handler {}", label()));
                }),
            );
            k.end_group().unwrap();
        });
        let result = run_app_with(
            headless("click \"+\""),
            Box::new(FixedAdvanceMetrics),
            content,
        );
        assert!(result.is_ok());
        for line in log.borrow().iter() {
            println!("\nlog: {}", line);
        }
        std::process::exit(0)
    }

    fn push_line(log: &Log, line: String) {
        log.borrow_mut().push(line);
    }

    /// compose-hot-reload, ui: "A reload waits for the running handler". The handler finishes in
    /// its old code, and the reload is applied before the next pass, which runs the new code.
    #[test]
    fn a_reload_waits_for_the_running_handler() {
        let output = run_in_child(
            "app::tests::a_reload_waits_for_the_running_handler",
            reload_in_handler_child,
        );
        assert_eq!(output.status.code(), Some(0), "{:?}", output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.strip_prefix("log: "))
                .collect::<Vec<_>>(),
            ["compose old 0", "handler old", "compose new 1"]
        );
    }

    /// The child-process half of `waiting_for_a_reload_that_never_comes`.
    fn wait_in_child() -> ! {
        crate::host::set_wait_for_reload(std::time::Duration::from_millis(10));
        let commands = crate::script::parse_script_with(
            "wait reload\n",
            &crate::script::ScriptContext {
                watching: true,
                directory: std::path::PathBuf::new(),
                read: &|_| Err("no file".to_string()),
            },
        )
        .unwrap();
        let result = run_app_with(
            Box::new(HeadlessHost::new((400, 300), commands)),
            Box::new(FixedAdvanceMetrics),
            counter(Rc::new(Cell::new(0)), log()),
        );
        crate::error::raise(&result.unwrap_err())
    }

    /// compose-hot-reload, ui: "Waiting for a reload that never comes".
    #[test]
    fn waiting_for_a_reload_that_never_comes() {
        let output = run_in_child(
            "app::tests::waiting_for_a_reload_that_never_comes",
            wait_in_child,
        );
        assert_eq!(output.status.code(), Some(101));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "panic: event script line 1: no reload arrived within 10 seconds\n"
        );
    }

    /// The child-process half of `script_failures_end_the_process`.
    fn fail_in_child() -> ! {
        let result = run_app_with(
            headless("click \"Minus\""),
            Box::new(FixedAdvanceMetrics),
            counter(Rc::new(Cell::new(0)), log()),
        );
        crate::error::raise(&result.unwrap_err())
    }

    #[test]
    fn script_failures_end_the_process() {
        let output = run_in_child("app::tests::script_failures_end_the_process", fail_in_child);
        assert_eq!(output.status.code(), Some(101));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "panic: event script line 1: no node shows the text \"Minus\"\n"
        );
    }

    #[test]
    fn one_app_at_a_time_and_a_host_is_needed() {
        let _lock = crate::host::test_lock();
        configure(HostConfiguration::None);
        reset_drops();
        assert_eq!(
            run_app("Prism", 800, 600, handler("content", || {})),
            Err(NO_WINDOW_SYSTEM)
        );
        assert_eq!(drops(), ["drop content"]);
        // runApp from a handler of a running app.
        configure(HostConfiguration::Headless {
            size: Some((200, 100)),
            commands: parse_script("click \"nest\"").unwrap(),
        });
        let nested = Rc::new(RefCell::new(None));
        let result = nested.clone();
        run_app(
            "Prism",
            800,
            600,
            content(move |k| {
                let result = result.clone();
                button(
                    k,
                    1,
                    "nest",
                    handler("nest", move || {
                        *result.borrow_mut() = Some(run_app("Prism", 1, 1, content(|_| {})));
                    }),
                );
            }),
        )
        .unwrap();
        assert_eq!(nested.borrow_mut().take(), Some(Err(APP_RUNNING)));
        configure(HostConfiguration::None);
    }

    /// The scenario "Starting or disposing a composition during a pass", for `runApp` reached
    /// through a function value: the composition misuse comes before any host is made, so no
    /// window opens (with no window system configured, the host would fail otherwise).
    #[test]
    fn run_app_during_a_pass_opens_no_host() {
        let _lock = crate::host::test_lock();
        configure(HostConfiguration::None);
        reset_drops();
        let nested = Rc::new(RefCell::new(None));
        let result = nested.clone();
        let composition = Composition::start(
            Box::new(prism_compose::NodeTree::new()),
            content(move |_| {
                *result.borrow_mut() = Some(run_app("Prism", 1, 1, handler("inner", || {})));
            }),
        )
        .unwrap();
        assert_eq!(
            nested.borrow_mut().take(),
            Some(Err(crate::error::UiError::Composition(
                "composition operation during a pass"
            )))
        );
        assert_eq!(drops(), ["drop inner"]);
        Composition::dispose(composition).unwrap();
    }

    #[test]
    fn handlers_and_setters_outside_the_app() {
        install_tree(1, 1);
        assert_eq!(
            setters::register(HandlerKind::Click, inline_handler(0)),
            Err(HANDLER_WITHOUT_APP)
        );
        // A handler of the app sets a property outside composition.
        let seen = Rc::new(RefCell::new(None));
        let record = seen.clone();
        let node = Rc::new(Cell::new(NodeId(0)));
        let remembered = node.clone();
        let (host, _) = scripted(vec![HostEvent::Press(10, 10), HostEvent::Release(10, 10)]);
        run_app_with(
            host,
            Box::new(FixedAdvanceMetrics),
            content(move |k| {
                let id = NodeId(k.start_node_group(1, 3).unwrap().0);
                remembered.set(id);
                let node = remembered.clone();
                let record = record.clone();
                let handler = setters::register(
                    HandlerKind::Click,
                    handler("set", move || {
                        *record.borrow_mut() = Some(setters::set_text(node.get(), "x"));
                    }),
                )
                .unwrap();
                let chain = ModifierMirror::encode(&[size(20, 20), clickable(handler, true, None)]);
                setters::set_modifier(id, &chain).unwrap();
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        assert_eq!(seen.borrow_mut().take(), Some(Err(OUTSIDE_COMPOSITION)));
        let _ = node;
    }

    /// Wide metrics: every character advances 10 and a line is 30 high.
    struct WideMetrics;

    impl TextMeasurer for WideMetrics {
        fn advance(&self, _character: char, _font_size: i64) -> i64 {
            10
        }

        fn line_height(&self, _font_size: i64) -> i64 {
            30
        }
    }

    /// A host with its own measurer and scale factor, which dumps what it presents.
    struct MeasuringHost {
        events: std::vec::IntoIter<HostEvent>,
        presented: std::sync::Arc<std::sync::Mutex<String>>,
    }

    impl Host for MeasuringHost {
        fn size(&self) -> (i64, i64) {
            (400, 300)
        }

        fn next_event(&mut self) -> Option<HostEvent> {
            self.events.next()
        }

        fn present(&mut self, list: &DisplayList) {
            dump_display_list(list, &mut self.presented.lock().unwrap());
        }

        fn measurer(&self) -> Box<dyn TextMeasurer> {
            Box::new(WideMetrics)
        }

        fn scale_factor(&self) -> f64 {
            1.5
        }
    }

    /// `desktop-window` D18: `runApp` lays out with the host's measurer, and the final-frame
    /// sink sees each app's last display list once, with the window's size and the host's
    /// scale factor.
    #[test]
    fn a_host_with_its_own_measurer_and_the_final_frame_sink() {
        let _lock = crate::host::test_lock();
        let presented = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let shown = presented.clone();
        let factory: crate::host::WindowFactory = std::sync::Arc::new(move |_, _, _| {
            Box::new(MeasuringHost {
                events: vec![HostEvent::Frame, HostEvent::Resize(200, 100)].into_iter(),
                presented: shown.clone(),
            })
        });
        configure(HostConfiguration::Window(factory));
        // The sink is process-wide, so it records only the apps of this test's thread.
        let thread = std::thread::current().id();
        let sunk = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = sunk.clone();
        crate::host::set_final_frame_sink(Some(Box::new(move |list, size, scale| {
            if std::thread::current().id() == thread {
                let mut dump = String::new();
                dump_display_list(list, &mut dump);
                record.lock().unwrap().push((dump, size, scale));
            }
        })));
        let count = Rc::new(Cell::new(0));
        run_app("Prism", 1, 1, counter(count.clone(), log())).unwrap();
        run_app("Prism", 1, 1, counter(count, log())).unwrap();
        crate::host::set_final_frame_sink(None);
        configure(HostConfiguration::None);
        // Lines are 30 high: the button starts at 16 + 30 + 8, and its row is 30 + 16 high.
        let first = "clear 400x300 #FFFFFFFF\n\
                     text 16,16 size 16 #FF000000 \"Count: 0\"\n\
                     rect 16,54 58x46 #FF6750A4 radius 23\n\
                     text 40,62 size 16 #FFFFFFFF \"+\"\n";
        assert_eq!(*presented.lock().unwrap(), first.repeat(2));
        let sunk = sunk.lock().unwrap();
        assert_eq!(sunk.len(), 2, "once per app");
        // The resize to 200 x 100 leaves the button 100 - 16 - 54 high.
        let last = first
            .replace("clear 400x300", "clear 200x100")
            .replace("58x46 #FF6750A4 radius 23", "58x30 #FF6750A4 radius 15");
        assert_eq!(sunk[0], (last.clone(), (200, 100), 1.5));
        assert_eq!(sunk[1], (last, (200, 100), 1.5));
    }
}
