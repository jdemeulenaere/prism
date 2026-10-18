# Proposal: desktop-window

## Why

After `ui-toolkit`, a Prism app composes, lays out, builds a display list and reacts to input, but only headless: `runApp` without `--headless` ends with `ui misuse: no window system is available; run with --headless`, and nobody can see or click what a program builds. This change is item 31 of "Then: the road to Milestone 2" in `ROADMAP.md`, the window half of `docs/architecture.md` §5 ("Rendering Through a Display List", `compose-architecture` D1 and D20). It puts `runApp` on screen in a native window: rasterized on the CPU, with the bundled Inter font, driven by the mouse and keyboard, and sharp on HiDPI displays. The program keeps running on its own thread, while macOS requires the window's event loop on the process's main thread.

## What Changes

- **A `prism-desktop` crate** (D2). It holds the window, the main-thread server, the rasterizer, the bundled font and the window host that implements `prism-ui`'s `Host`. It depends on `prism-ui` and on four pinned crates: `winit` 0.30.13, `softbuffer` 0.4.6, `tiny-skia` 0.12.0 and `fontdue` 0.9.4 (D5). Only the `prism` binary links it, through a default `window` feature of `prism-cli`. No compiler crate, `prism-ui` or the backend depends on it, so `cargo build -p prism-cli --no-default-features` builds a headless-only `prism`.
- **The program keeps its thread, and the main thread serves windows** (D3). `prism run` still runs `main` on its 8 MiB program thread. Composition, layout, handlers and the display list stay there. The CLI's main thread waits for that thread's requests. When `runApp` opens a window, the main thread runs the `winit` event loop and owns the window, its surface and the rasterizer. Input crosses to the program thread over a channel, and display lists cross back through a latest-frame mailbox.
- **Lazy start** (D4). A program that never calls `runApp` creates no event loop, loads no font and opens no window, so non-UI programs start as before. The event loop is created at the first `runApp` and kept for the process. Each app runs it until its window closes, so apps can run one after another.
- **Which runs open a window** (D3, D15). `prism run main.prism` opens a window when `main` calls `runApp`. The window gets `runApp`'s `title` and an inner size of `width` × `height` logical pixels. `--headless` keeps `ui-toolkit`'s headless host. A build without the `window` feature, or a library caller of `run_file`, keeps `ui-toolkit`'s "no window system" error. A machine without a display gives the new runtime error `display error: cannot open a window (<reason>)`.
- **Closing the window ends `runApp`** (D14). The close button, and Cmd+W or Cmd+Q on macOS, close the window at once. `runApp` then drops its composition and returns, so the statements after it run, and `main` ends normally under `--check-allocations`.
- **Rendering** (D9, D12). The rasterizer draws each display list into a `tiny-skia` pixmap at physical pixels and copies it into the `softbuffer` surface. Clears, rectangles, rounded rectangles and borders come from `tiny-skia`, and glyph coverage from `fontdue`. Axis-aligned edges snap to physical pixels and are drawn without anti-aliasing, while rounded corners and glyphs are anti-aliased. Commands outside the window are skipped.
- **HiDPI** (D9). Layout stays in integer logical pixels. The window's scale factor, which may be fractional, multiplies them into physical pixels for rasterization. Resizes report `floor(physical / scale)` logical pixels to the app. A scale change between displays is redrawn from the last display list without a pass.
- **Text** (D6, D7, D8). Inter 4.1 Regular (SIL Open Font License 1.1, no Reserved Font Name) is embedded in the binary and parsed once, the first time a window needs it. Windows lay text out with Inter's metrics, through `ui-toolkit`'s `TextMeasurer`: each character advances by its glyph's advance, rounded to whole logical pixels, and glyphs are drawn exactly at those positions. Headless runs keep the fixed-advance metric, so a window's layout differs from the headless dumps (D7). There is no shaping: one glyph per character, with no kerning, ligatures, combining marks, right-to-left text or font fallback. A character missing from Inter draws the font's missing-glyph box (D8).
- **Redraw only on change** (D10, D11). After it delivers input, the window host asks the app for one frame once the queue is empty. The main thread draws on the platform's next redraw callback, which paces drawing to the display. Neither thread polls or runs a timer, so an idle window uses no CPU.
- **Input** (D13). The left button maps to `ui-toolkit`'s `Press` and `Release` at the cursor's logical position. Typed text maps to `Character` events, and Backspace and Enter to `Key` events. Shortcuts with Ctrl or Cmd type nothing. Resizes map to `Resize`. Moves, other buttons, scrolling and IME are not dispatched.
- **`prism run --screenshot <file.png>`** (D15). When `runApp` returns, the app's final display list is rasterized and written as a PNG: at scale 1 for headless runs, at the window's scale factor for windows.
- **Hooks in `prism-ui`** (D18), small additions to `ui-toolkit`'s code:
  - `Host::measurer` and `Host::scale_factor`, with headless defaults;
  - a `WindowFactory` for `HostConfiguration::Window`;
  - a final-frame sink for screenshots;
  - `Clone` on `DisplayList`;
  - a wake-up for reload points, which `compose-hot-reload` will register.
- **Watch sessions** (D17). Under `prism run --watch`, `hot-reload`'s session moves to its own thread, and the main thread serves windows.
- **Costs** (D19). The window server costs a non-UI program one channel. Rasterization is linear in the visible commands and in the pixels, and a steady-state frame allocates nothing. Release-build targets: under 4 ms per frame, from display list to presented pixels, for the counter app and for a 1,000-item todo list at 800×600 logical on a scale-2 display. An ignored benchmark rasterizes display lists of 1,000 and 10,000 nodes. The compiler gains no work: no language, MIR or backend change.
- **Tests** (D20). Golden PNGs of the rasterizer, deterministic up to one unit per channel. Rust tests of the window host's protocol and of input mapping, with no window. An allocation-counting test. An opt-in smoke test that opens a real window on the main thread (`harness = false`), skipped without `PRISM_WINDOW_TESTS=1` or a display. A CLI test of headless screenshots. A manual verification checklist.
- **Not in scope.** All of the following are left to other changes or follow-ups:
  - reloads between frames (`compose-hot-reload`), and `--frame-stats` with live counts (`reactive-app-verification`);
  - a GPU renderer, partial redraws by damage region, and subpixel text positioning;
  - kerning, shaping, font fallback, emoji, other weights and system fonts;
  - IME, text selection, the clipboard, cursors and hover, scrolling, touch and accessibility;
  - several windows at once, window options (resizable, position, icon), menus;
  - `--events` driving a window, and a headless mode with the window's font metrics;
  - a subset font, and the licence notices of a packaged release.

## Capabilities

### New Capabilities
- `desktop`: running apps in a native window. It covers the window crate's boundary and build feature, the main-thread server and the program thread, lazy start, window lifecycle (title, size, closing, apps in sequence), HiDPI scaling, rasterization and its determinism, the bundled font, window text metrics and shaping limits, the redraw strategy and idle behaviour, input mapping, display errors, watch sessions with windows, and the window's costs.

### Modified Capabilities
- `execution`: `prism run --screenshot <file.png>` (ADDED). The options and the "no window system" rule that `ui-toolkit` adds stay as they are.

## Impact

- New `crates/prism-desktop`, a workspace member:
  - sources: `lib.rs`, `server.rs`, `window.rs`, `host.rs`, `input.rs`, `raster.rs`, `glyphs.rs`, `font.rs`, `metrics.rs`, `screenshot.rs` and `statistics.rs`;
  - assets: `fonts/Inter-Regular.ttf` and `fonts/OFL.txt`;
  - tests: `tests/raster_golden.rs` with `tests/golden/*.png`, `tests/allocations.rs`, `tests/raster_timing.rs` (ignored) and `tests/window_smoke.rs` (`harness = false`, opt-in).
- `crates/prism-ui`: `Host::measurer` and `Host::scale_factor` with defaults, `WindowFactory`, the final-frame sink, `DisplayList: Clone`, and the reload wake-up seam.
- `crates/prism-runtime`: `PanicKind::DisplayFailed` (`display error: <detail>`).
- `crates/prism-codegen`: `Program::start`'s end-of-program notifier accepts a closure, so the CLI can route program events to the main-thread server.
- `crates/prism-cli`:
  - an optional `prism-desktop` dependency and a default `window` feature;
  - `main.rs`: the main-thread server around `prism run` and `prism run --watch`, and `--screenshot`;
  - `run.rs`: `RunOptions::screenshot`;
  - `tests/screenshot.rs`.
- `Cargo.toml` (workspace): the new member, and `opt-level = 3` in the dev profile for `tiny-skia`, `tiny-skia-path`, `fontdue` and `ttf-parser`.
- Specs: the new `desktop` capability and the `execution` delta. No diagnostics change: there is no new error code.
- `docs/architecture.md` §5: on archive, the window half of "Rendering Through a Display List" and the `prism-desktop` part of "Compose Runtime Crates Behind C Entry Points" move out.
- **Prerequisites:** `ui-toolkit` must be archived first, which brings `state-handles`, `compose-transform`, `compose-runtime` and `standard-library`. `hot-reload` must also be archived, or at least its `Program::start` split and its watch session must have landed. See `design.md`, "Baseline".
