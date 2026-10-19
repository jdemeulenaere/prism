# Tasks

Most scenarios of the `desktop` capability are tested by Rust tests in `crates/prism-desktop`, since executable fixtures cannot open windows. Those whose WHEN names a Rust test, a benchmark or the smoke test are tested where their task says. Those that need a person, closing a window by its button for instance, are in the manual checklist (group 8), whose results are recorded in design.md. The `execution` scenarios are CLI tests in `crates/prism-cli/tests/screenshot.rs`. "The counter screen" means the `Counter` composable of the `ui` spec's scenario "Counter screen", and its display list is the one `dump draw` prints for it headless at 400 × 300.

Prerequisites (design, "Baseline"): `ui-toolkit` is archived, which brings `state-handles`, `compose-transform`, `compose-runtime` and `standard-library`, and `hot-reload`'s `Program::start` and `WatchSession` have landed. Groups 2 and 3 need only `prism-ui`'s display list and `TextMeasurer`, so they can start as soon as those exist.

## 1. Baseline, crate and dependencies (D2, D4, D5)

- [x] 1.1 Compare the names the design takes from `ui-toolkit` and `hot-reload` (design, "Baseline") with the code those changes left, and fix every task below that names one differently:
  - `Host`, `HostEvent`, `Key`, `HostConfiguration` and `configure`;
  - `run_app`, `DisplayList` and `DrawCommand`, with their fields;
  - `TextMeasurer` and `FixedAdvanceMetrics`;
  - `Program::start`, `ProgramEvent` and `RunningProgram`;
  - `WatchSession`, and `reload::defer_to_reload_points`.

  Record any difference in design.md, "Baseline". Verify by reading the code.
- [x] 1.2 Create `crates/prism-desktop` (a workspace member, `license.workspace = true`) with `lib.rs` and the dependencies of design D5, pinned exactly and with their features: `winit = { version = "=0.30.13", default-features = false, features = [...] }`, `softbuffer = "=0.4.6"`, `tiny-skia = "=0.12.0"` and `fontdue = "=0.9.4"`, plus `prism-ui`. If `tiny-skia` 0.12.0 does not build, use `=0.11.4` and record it in D5. Add `opt-level = 3` for `tiny-skia`, `tiny-skia-path`, `fontdue` and `ttf-parser` under `[profile.dev.package.*]` in the workspace `Cargo.toml`. Verify that `cargo build -p prism-desktop` succeeds, and that `cargo tree -d -p prism-desktop --target aarch64-apple-darwin` shows a single `objc2`. Record its duplicates in D5.
- [x] 1.3 In `crates/prism-cli/Cargo.toml`, add `prism-desktop` as an optional dependency, with `[features] default = ["window"]` and `window = ["dep:prism-desktop"]`. Measure and record in design.md, D2 and D4:
  - the clean `cargo build -p prism-cli` time with and without `--no-default-features`;
  - the incremental rebuild time after touching `crates/prism-typechecker/src/lib.rs`, with and without the feature;
  - the size of `target/release/prism`;
  - the wall time of the startup benchmark (`run_timing.rs`) before and after the feature, which must grow by at most 1 ms.
- [x] 1.4 Add `crates/prism-desktop/tests/dependencies.rs`. It reads every workspace member's `Cargo.toml` and checks that only `prism-cli` names `prism-desktop`, as an optional dependency of its default `window` feature, and that no member but `prism-desktop` names `winit`, `softbuffer`, `tiny-skia` or `fontdue`. Verify with `cargo test -p prism-desktop --test dependencies`, for the scenario "Only the CLI depends on the window crate".
  - Apply note (2026-10-19): the test exists and passes, but until task 1.3 adds the dependency to `prism-cli` it checks only the negative half; task 1.3 sets its `CLI_WIRED` to `true`, which requires the optional dependency and the `window` feature. Done with task 1.3: it now checks the whole scenario.

## 2. The font and the window's text metrics (D6, D7, D8)

- [x] 2.1 Add `crates/prism-desktop/fonts/Inter-Regular.ttf`, from `extras/ttf/` of Inter 4.1's release archive (`https://github.com/rsms/inter/releases/tag/v4.1`), and `fonts/OFL.txt`, Inter's `LICENSE.txt`. Record the file's size and BLAKE3 hash in design.md, D6. Add `src/font.rs`:
  - `font() -> &'static fontdue::Font`, through a `OnceLock` over `include_bytes!`, with `FontSettings { scale: 40.0, load_substitutions: false, collection_index: 0 }`;
  - `is_loaded() -> bool`, for the statistics.

  Verify with unit tests for the scenario "The font and its licence": the family name is `Inter`, `OFL.txt` names "SIL Open Font License, Version 1.1", and a second `font()` returns the same address. Measure the parse time in a release build and record it in D6. If it exceeds 10 ms, add the subset follow-up to Open Questions.
- [x] 2.2 Add `src/metrics.rs` with `FontMetrics: prism_ui::TextMeasurer`:
  - `advance` rounds `font().metrics(c, size).advance_width`, with a per-size 128-entry ASCII table and a map for other characters;
  - `line_height` rounds `horizontal_line_metrics(size).new_line_size`;
  - `baseline(size)` rounds the ascent.

  Verify with unit tests: the advance of every printable ASCII character at 16 equals the rounded `fontdue` advance; the line height at 16 is recorded in D7 (19 expected); the cache gives the same values as an uncached measurer; `中` measures as glyph 0's rounded advance. Then build a `prism-ui` tree through its test API: a `Text("Count: 0")` laid out with `FontMetrics` is the sum of the eight rounded advances wide and one line high, for the measurement half of "A text node's size in a window".

## 3. The rasterizer (D9, D12)

- [x] 3.1 Add `src/raster.rs` with `Rasterizer::new()` and `rasterize(&mut self, list: &DisplayList, scale: f64, width: u32, height: u32)`:
  - the pixmap is kept and resized only on a size change;
  - `Clear` is a fill; `Rectangle` without a radius is a snapped span fill without anti-aliasing; with a radius its corners are anti-aliased circular arcs (radius clamped to half the smaller side; D12's revision at apply);
  - `Border` is the outer rounded rectangle's coverage minus the inset one's;
  - commands outside the pixmap are culled;
  - `pixmap()`, and `copy_to_surface(&self, buffer: &mut [u32])`, which converts to `0x00RRGGBB`.

  Verify with unit tests: the scenario "A rectangle at scale 2"; adjacent rectangles at scale 1.5 neither overlap nor leave a gap; a border lies inside its bounds; a command at negative coordinates is clipped; a command entirely outside counts as culled; the conversion of a known pixel.
- [x] 3.2 Add `src/glyphs.rs` with `GlyphCache` (keyed by glyph index and pixel size, cleared beyond 4,096 entries or on a scale change), and draw `Text` commands in `raster.rs`: one glyph per character at the pen position from `FontMetrics` advances, scaled and snapped, with the baseline from `FontMetrics::baseline`, blended source-over with the text colour. Verify with unit tests that record glyph positions: the scenarios "No kerning" (`AV` places `V` where `AX` places `X`) and "A character missing from the font" (glyph 0 drawn), the scenario "Glyph positions follow the measured advances", and a cache hit on the second rasterization.
- [x] 3.3 Add `tests/raster_golden.rs` and `tests/golden/*.png` with the cases of design D20, at scales 1, 1.5 and 2. Compare within one unit per channel, and regenerate with `PRISM_UPDATE_GOLDEN=1`. Look at every image before committing it. Verify with `cargo test -p prism-desktop --test raster_golden`, for the scenario "Golden images".
- [x] 3.4 Add `tests/raster_timing.rs` (`#[ignore]`d) with the shapes and targets of design D19 and the assertions of "Rasterization benchmark" under `PRISM_ASSERT_TIMING=1`. Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-desktop --test raster_timing -- --ignored --nocapture`, and record the medians in D19.

## 4. Hooks in prism-ui and the runtime (D15, D16, D18)

- [x] 4.1 In `crates/prism-ui/src/host.rs`, add `Host::measurer` (default `&FixedAdvanceMetrics`) and `Host::scale_factor` (default 1.0), `WindowFactory` and `HostConfiguration::Window(Arc<dyn WindowFactory>)`, and `set_final_frame_sink`. In `app.rs`, take the measurer from the host when the app starts. When the app ends, rebuild a stale display list and call the sink before dropping the composition. In `draw.rs`, derive `Clone` for `DisplayList` and `DrawCommand`. Verify with `prism-ui` unit tests: a host with its own measurer lays out with it; the sink is called once per app with the final list and the host's size and scale; and every headless fixture still passes unchanged (`cargo test -p prism-cli --test fixture_tests`).
- [x] 4.2 Add `PanicKind::DisplayFailed` to `crates/prism-runtime/src/panic.rs`, printing `display error: <detail>`, raised with `panic_without_location`. Verify with a runtime unit test of the message, in a child process for the status 101.

## 5. The window host and the main-thread server (D3, D4, D10, D11, D13, D14, D17)

- [x] 5.1 Add `src/host.rs` with `WindowHost: prism_ui::Host`:
  - `size`, from the window's reply, then from `Resize` messages;
  - `next_event`, with the `frame_due` rule of D11;
  - `present`, copying into the `FrameMailbox` (D10) and sending `UserEvent::FrameReady` unless the mailbox is closed;
  - `measurer` (`FontMetrics`) and `scale_factor`;
  - `wake()`, a `fn()` over a static sender (D17);
  - `Drop`, which sends `UserEvent::CloseWindow`.

  It talks to the main thread through a `WindowConnection` (an input `Receiver<WindowMessage>`, the mailbox, and a `FrameSink` trait that the `winit` proxy implements and tests fake). Verify with protocol tests without a window:
  - the scenario "Events in a burst give one frame";
  - a first `Frame` when the window opens;
  - `Resize` updates `size`, and the scenario "A resize relays out the app", with a `prism-ui` app built through its `test_support.rs` running against the host on a test thread;
  - `Close` ends the app (`None`);
  - `wake` gives a `Frame`;
  - a present after close does nothing;
  - the scenario "Frames faster than redraws": three presents before one mailbox take give the third list, once, and a take with nothing new reports that the last pixels can be presented again.
- [x] 5.2 Add `src/input.rs` with `InputState` (cursor position, modifiers, scale factor) and pure functions `pointer_button`, `cursor_moved`, `key_pressed(logical_key, text, modifiers, platform)` and `resized(physical, scale)`, each returning the `WindowMessage`s to send, or a close. Verify with unit tests of every mapping in D13, including the scenarios "A click at scale 2", "Typing, deleting and shortcuts" and "A resize at a fractional scale", a scale change with an unchanged logical size sending nothing to the app (the program half of "Moving to another display"), Cmd+W and Cmd+Q on macOS, a press before any cursor move, and a zero size.
- [x] 5.3 Add `src/statistics.rs`, `WindowStatistics`, with atomic counters and times shared by both threads: event loop created, windows opened, frames received, rasterized and presented, and the last and median frame times. Add `src/window.rs` with `WindowApp: winit::application::ApplicationHandler<UserEvent>`:
  - `resumed` creates the window (D14), the `softbuffer` context and surface, and replies to `OpenWindow`, or replies with the error;
  - `window_event` routes through `input.rs`, resizes the surface, and rasterizes and presents on `RedrawRequested` by the rule of D10, calling `pre_present_notify`;
  - `user_event` handles `FrameReady` (`request_redraw`) and `CloseWindow`;
  - closing follows D14.

  Verify with `cargo build -p prism-desktop` and the smoke test (5.6).
- [x] 5.4 Add `src/server.rs` with `MainThreadServer::new() -> (MainThreadServer, WindowSystem)`:
  - `serve()` (D3), creating the event loop at the first `OpenWindow` with `with_default_menu(false)` on macOS, and running `run_app_on_demand` per window;
  - `WindowSystem: Clone + Send`, with `factory() -> Arc<dyn WindowFactory>`, `program_ended()`, `close_all()` and `shut_down()`;
  - `MainThreadServer::with_starter`, so tests can fake an event loop that fails to start.

  `WindowFactory::open` sends `OpenWindow`, parses the font while it waits, and raises `display error: cannot open a window (<reason>)` on an error reply. Verify with Rust tests: the scenario "A program without an app" (no event loop, no font); and the scenario "No display", with a starter failing with `no display`, in a child process.
- [x] 5.5 Add `tests/allocations.rs` with a counting `#[global_allocator]` and the scenario "Allocation-counting test": 100 presents through a `WindowHost` with a fake proxy, each followed by the mailbox swap and a rasterization of the counter screen's list at a fixed size, after two warm-up frames (the mailbox and the main thread trade their two lists, so each grows once). Verify with `cargo test -p prism-desktop --test allocations`.
- [x] 5.6 Add `tests/window_smoke.rs` with `harness = false` in `Cargo.toml` and the steps of design D20. Skip with the line `window smoke test skipped: set PRISM_WINDOW_TESTS=1 on a machine with a display` unless enabled. It covers:
  - "Threads of a window app": thread identities are recorded by the test's fake program and by `WindowStatistics`;
  - "Title and size of the window": `Window::title()` and `inner_size()` are reported through a test hook of `window.rs`;
  - "An idle window";
  - two windows opened one after the other through the host API.

  It also reports the time to the first frame and the median frame time. The protocol test of "A resize relays out the app" is `tests/window_host.rs`, and the server's tests ("A program without an app", "No display") are `tests/server.rs`. Run `PRISM_WINDOW_TESTS=1 cargo test -p prism-desktop --test window_smoke` and `cargo test --release` likewise, and record the times in D19. Check that `cargo test --workspace` prints the skip line without the variable.

## 6. The CLI (D2, D3, D15, D17)

- [x] 6.1 If `Program::start` takes a `Sender<ProgramEvent>`, generalize it to any `impl Fn(ProgramEvent) + Send + 'static`, and adapt `WatchSession`. In `crates/prism-cli/src/main.rs::run_program`, under `#[cfg(feature = "window")]` and without `--headless`, build the `MainThreadServer`, configure `HostConfiguration::Window`, start the program with `program_ended` as its notifier, `serve`, then `join`. Keep the headless path and the no-feature path as `ui-toolkit` left them. `run.rs::run_file` stays the library entry, with no window. Split it into compile and start, so `main.rs` can serve between them. Verify that every executable fixture passes, and that `prism run examples/run/main.prism` behaves as before.
- [x] 6.2 Add `--screenshot <file.png>` to `run_program`'s options and usage (a usage error without a file name, or without the `window` feature), and `RunOptions::screenshot`. Set the final-frame sink to `prism_desktop::screenshot::writer(path)` (`src/screenshot.rs`: rasterize at the host's scale, `encode_png`, and `display error: cannot write the screenshot <path> (<reason>)` on failure). Add `crates/prism-cli/tests/screenshot.rs` for the scenarios "A headless screenshot" (decoding with `tiny_skia::Pixmap::decode_png` through `prism_desktop::screenshot::read`), "A screenshot without a file name" and "A screenshot that cannot be written". Verify with `cargo test -p prism-cli --test screenshot`.
- [x] 6.3 Under `--watch` with the `window` feature, run `WatchSession` on a thread named `watch-session` and serve the main thread. The session calls `close_all()` when the program stops and `shut_down()` when it ends. Verify with the existing watch tests, and with the manual check of "A watched window app" (8.6).
- [x] 6.4 Check the scenario "A build without the window feature": run `cargo build -p prism-cli --no-default-features`, then `target/debug/prism run main.prism` on its program, and expect the exact stderr and status 101. Then rebuild with the default features. Record the result in design.md, D2.

## 7. Integration checks

- [x] 7.1 Run the integration checks:
  - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings) and `cargo test --workspace`;
  - `cargo run -q -p prism-cli -- check` on every file under `examples/`, and `cargo run -q -p prism-cli -- fmt --check examples/`;
  - `cargo run -q -p prism-cli -- run --check-allocations examples/run/main.prism`, which must behave as before;
  - the startup benchmark of `run_timing.rs`, whose total to `main` must stay under 15 ms and whose wall time must grow by at most 1 ms (D4).

  Record the medians in design.md, D19.

## 8. Manual verification on macOS (and Linux if available)

Each check runs a release build of `prism` and is recorded, with the date and the machine, in design.md under a new "Manual verification" section.

- [ ] 8.1 `prism run --check-allocations` on the counter program (the `ui` spec's counter screen in `runApp(title = "Counter")`) opens a window titled `Counter`. Clicking `+` counts up, a press on `+` released elsewhere does not, and the text stays sharp on a Retina display.
- [ ] 8.2 Closing that window by its close button, by Cmd+W and by Cmd+Q, each in a fresh run, ends `runApp`. A `println("closed")` after it prints, the status is 0, and the allocation check reports nothing ("Statements after runApp run").
- [x] 8.3 A program with two `runApp` calls opens the second window after the first closes ("Two apps in sequence").
  - Review note (2026-10-19): checked by `crates/prism-cli/tests/window_input.rs` ("two-apps"), with the closes delivered through the window's handler.
- [ ] 8.4 A `TextField` app accepts typed characters, Backspace and key repeat, and ignores Cmd+A and arrows. Live resizing relays out without the window showing stale or stretched content for more than a frame. Moving the window between a Retina display and a scale-1 display keeps the layout and redraws sharply ("Moving to another display").
- [x] 8.5 An idle window app uses 0% CPU in Activity Monitor, or `top`, over 10 s.
- [x] 8.6 `prism run --watch` on the counter program opens the window, and closing it prints `watch: main returned with status 0; waiting for changes` ("A watched window app"). A runtime error in a handler (`Button(onClick = { println(1 / zero()) })`) ends the run with status 101, its message on stderr, and the window gone.
  - Review note (2026-10-19): checked by `crates/prism-cli/tests/window_input.rs` ("watch" and "runtime-error"); under `--watch` the error stops the program and closes its window, and without it the process ends with status 101.
- [x] 8.7 On a Linux machine with X11 or Wayland, if one is available, 8.1 and 8.2 behave alike, and without `DISPLAY` and `WAYLAND_DISPLAY` the program ends with `display error: cannot open a window (...)`. If none is available, record that.
- [x] 8.8 (review) Add `crates/prism-cli/tests/window_input.rs`, opt-in with `PRISM_WINDOW_TESTS=1`, which runs real programs in real windows and delivers clicks, key presses, resizes and closes through the window's own handler (`WindowSystem::deliver`), and list in design.md's "Manual verification" what is left for a person, with the commands. 8.1, 8.2 and 8.4 stay open: they need real hardware (the platform's mouse and keyboard events, Retina sharpness, live resizing by hand, a second display).

## Workflow follow-up

- At archive, remove from `docs/architecture.md` §5 what this change implements: the window half of "Rendering Through a Display List", whose scenario "Window app runs and exits cleanly" becomes "Statements after runApp run"; the `prism-desktop` part of "Compose Runtime Crates Behind C Entry Points"; and `--screenshot` from "Headless Rendering and Scripted Input". In the design notes, record that D20's versions are those of this change's D5 (`softbuffer` 0.4.6 and `tiny-skia` 0.12.0), and that windows lay text out with Inter's metrics while headless runs keep the fixed metric (D7).
- At archive, promote D2 to D17 into a "Design Rationale" section of the new `desktop` spec, and D15 into `execution`'s.
- At archive, add the follow-ups of the proposal and of design.md's Open Questions to `ROADMAP.md`, naming `desktop-window`. Note for `compose-hot-reload` that `WindowHost::wake` is the reload wake-up to register with `defer_to_reload_points` (D17), and for `reactive-app-verification` that `WindowStatistics` holds the window's frame times for `--frame-stats` (D19).
