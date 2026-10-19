# Design: desktop-window

## Context

See `proposal.md` for motivation. This is item 32 of "Then: the road to Milestone 2" in `ROADMAP.md`, the window half of `docs/architecture.md` §5. It implements `compose-architecture` D1 (the `prism-desktop` crate) and the window half of D20: CPU rasterization with `tiny-skia`, glyphs from the bundled Inter font through `fontdue`, presentation with `softbuffer` in a `winit` window, and the window's event loop on the main thread while the program keeps its thread. It fills the seams that `ui-toolkit` D2, D12 and D15 left for it: the `Host` interface, the window's text metrics and the window factory. It also uses the seam of `hot-reload` D2 and D4: the session leaves the main thread, and frame loops get reload points. Each decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`, because the user was not available. Fast compilation is a primary criterion (user, 2026-10-19). This change adds no language feature and no compiler stage, so its compile-time cost is the cost of building `prism` itself (D2, D5), and each decision states its run-time cost.

**Baseline.** This change was proposed while `hot-reload` was being implemented and `standard-library`, `compose-runtime`, `compose-transform`, `state-handles` and `ui-toolkit` were proposed but not implemented. It needs `ui-toolkit` archived, which brings the others, and `hot-reload`'s `Program::start` split and watch session. It relies on these names from those packets:

- **`ui-toolkit`:**
  - the `prism-ui` crate, its per-thread state, and the rule that no Rust reference into it is live while program code runs (its D2);
  - `host.rs`, with `Host { size, next_event, present }`, `HostEvent { Press, Release, Character, Key, Resize, Frame, Dump, Script }`, `Key { Backspace, Enter }` and `configure(HostConfiguration)`, where `HostConfiguration` is `None`, `Headless { .. }` or a window factory (its D15, task 8.1);
  - `app.rs::run_app` and its loop: passes while invalid, layout while dirty, otherwise `next_event`, with one pass and layout after each dispatched event and `present` on `Frame` (its D14, D15);
  - `DisplayList { commands: Vec<DrawCommand>, text: String }` and `DrawCommand::{Clear, Rectangle, Border, Text}` with integer logical bounds, ARGB colours, corner radii and text ranges (its D13);
  - `trait TextMeasurer { advance, line_height }` and `FixedAdvanceMetrics` (its D12);
  - `PanicKind::UiMisuse`, `ui misuse: no window system is available; run with --headless`, `--headless` and `--events` in `main.rs` and `run.rs` (its D16, D17).
- **`hot-reload`:**
  - `Program::start(main, arguments, check_allocations, events) -> RunningProgram`, `ProgramEvent::{MainReturned(i32), Stopped}` and `RunningProgram::join` (its task 2.8);
  - `crates/prism-cli/src/watch.rs::WatchSession`, run today on the CLI's main thread (its D2);
  - `prism_runtime::reload::defer_to_reload_points(wake: fn())` and `reload_point()` (its D4).
- **What exists today:**
  - `Program::run` (`crates/prism-codegen/src/program.rs`) spawns the program thread with `PROGRAM_STACK_SIZE` (8 MiB), sets the process-wide `STACK_LIMIT` for that thread's stack, and joins it.
  - Runtime errors end the process with `std::process::exit(101)` from the program thread, after flushing output.
  - `prism-runtime` has no dependencies, and `prism-cli`'s binary is the only executable.

The delta specs add a new capability and an ADDED execution requirement. They modify no requirement of `ui-toolkit`, whose `ui` and `execution` deltas are not in the main specs yet: its rule for a build without a window host stays true for builds without the `window` feature and for library callers.

**Baseline update (`ui-toolkit` revision, user direction 2026-10-19).** `ui-toolkit` now follows Jetpack Compose's modules and APIs: programs import `/prism/compose/ui` (with `runApp`), `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` instead of `/prism/ui`, and sizes are written `10.dp` (`ui-toolkit` D7, D21). A `Dp` is one logical pixel, which this change scales by the window's scale factor as it scaled `Int` sizes. Nothing this change relies on moves: `prism_ui::Host`, `HostEvent`, the display list's four commands and the text metrics interface (`ui-toolkit` D12, D13, D15) are unchanged.

**Apply note (Rust-only groups ahead of the CLI, 2026-10-19).** Groups 1 to 5 were implemented, as far as they need no CLI, while `compose-transform` was being applied and before `ui-toolkit` and `hot-reload` were archived, touching only the new `prism-desktop` crate, the workspace manifest and the hooks of D18 in `prism-ui`. Task 1.1 compared the names above with the code:
- **As planned:** `Host { size, next_event, present }`, `HostEvent` (its `Script` carries a `ScriptLine`), `Key { Backspace, Enter }` (in `input.rs`, re-exported by `host.rs`), `HostConfiguration::{None, Headless { size, commands }, Window(..)}` and `configure`; `run_app(title, width, height, content) -> UiResult<()>`, beside `run_app_with(host, measurer, content)` and `App`; `TextMeasurer { advance(char, i64) -> i64, line_height(i64) -> i64 }` and `FixedAdvanceMetrics`; `DisplayList { commands, text }` with private reused buffers, and `DrawCommand::{Clear { width, height, color }, Rectangle { bounds, color, corner_radius }, Border { bounds, width, color, corner_radius }, Text { x, y, font_size, color, text: Range<usize> }}`, where `Color(i64)` is ARGB with an unspecified sentinel and `Rect` has `i64` fields; `WatchSession` in `crates/prism-cli/src/watch.rs`; and `prism_runtime::reload::{defer_to_reload_points(wake: fn()), reload_point()}`.
- **Different:** `WindowFactory` is a closure type, not a trait, and the app owns a boxed measurer (D18's apply note). `Program::start(main, arguments, check_allocations, on_return: impl FnOnce(i32) + Send + 'static) -> RunningProgram` already takes a closure and there is no `ProgramEvent`, so task 6.1's generalization is not needed: the CLI passes `move |_status| windows.program_ended()`. `prism-ui` raises its runtime errors through `error::raise`, which writes `panic: <message>` and exits with 101, until `ui-toolkit` task 3.1 adds its `PanicKind`s; `prism-desktop`'s `error::raise_display_error` follows the same seam until task 4.2 adds `PanicKind::DisplayFailed`, which, like 4.2 itself, waits for the integration step because `prism-runtime` is being changed by `compose-transform`.

## Goals / Non-Goals

**Goals:**
- `runApp` on screen on macOS, the development platform, and on Linux (X11 and Wayland) and Windows through the same crates, without platform code of our own beyond macOS's close shortcuts.
- Sharp output at any scale factor, with layout untouched by it.
- No cost for programs that do not open a window: no event loop, no font, no display connection.
- No new work in the compiler, and no rebuild of the window crates when compiler crates change.
- Deterministic rasterization, pinned by golden images, and window frames well inside a 60 Hz budget.

**Non-Goals:**
- A GPU renderer, damage-region redraws, text shaping, IME and multiple windows (proposal, "Not in scope").
- Reloads between frames (`compose-hot-reload`) and frame statistics for users (`reactive-app-verification`).

## Decisions

### D1. The boundary with the other UI changes (autonomous)

| Concern | `desktop-window` (this change) | Elsewhere |
|---|---|---|
| `Host`, `HostEvent`, the frame loop | implements `Host` for windows; adds `measurer`, `scale_factor`, the window factory, the final-frame sink | defined and driven by `ui-toolkit` |
| Display list | rasterized, in windows and screenshots | built by `ui-toolkit` |
| Text metrics | Inter's, in windows (D7) | the interface and the headless metric are `ui-toolkit`'s |
| Program thread | unchanged; the main thread waits on a server instead of a join | `codegen-core`, `hot-reload`'s `Program::start` |
| Reloads | a wake-up seam that turns into a `Frame` | applied between frames by `compose-hot-reload` |
| Frame statistics | counters and times kept by the window server, read by tests | `--frame-stats` is `reactive-app-verification`'s |

Rejected: folding the window into `ui-toolkit` (the ROADMAP split it out, process row P10), and splitting the rasterizer into a crate of its own now (D2).

### D2. A `prism-desktop` crate behind a default `window` feature of `prism-cli` (autonomous; implements `compose-architecture` D1)

`crates/prism-desktop` depends on `prism-ui` (and through it `prism-runtime` and `prism-compose`), `winit`, `softbuffer`, `tiny-skia` and `fontdue`. `prism-cli` declares it as `prism-desktop = { path = "../prism-desktop", optional = true }` with `[features] default = ["window"]` and `window = ["dep:prism-desktop"]`. Only `main.rs` and `run.rs` use it, under `#[cfg(feature = "window")]`. Nothing else in the workspace depends on it, so:

- **Compiler work is unaffected.** Editing the lexer, the type checker or the backend rebuilds those crates and relinks `prism`, but never rebuilds `prism-desktop` or its dependencies. `cargo test -p prism-typechecker`, and every crate test except `prism-cli`'s, builds none of them.
- **Cold builds pay once.** A clean `cargo build` compiles about 40 more crates on macOS: `winit` and the `objc2` 0.5 family, `softbuffer`, `tiny-skia`, `fontdue` and `ttf-parser`. D5 keeps the set small by sharing one `objc2` version, and task 1.3 records the time.
- **`cargo build -p prism-cli --no-default-features`** builds a `prism` without any of them, whose `runApp` reports `ui-toolkit`'s "no window system" error.
- **In the dev profile,** `tiny-skia`, `tiny-skia-path`, `fontdue` and `ttf-parser` get `opt-level = 3`, as Cranelift does today. Unoptimized rasterization is 10 to 30 times slower, which would make `cargo run` windows lag, and these crates never change, so the slower first compile is paid once.

Rejected:
- **A `prism` binary crate separate from `prism-cli`:** the fixture runner uses `CARGO_BIN_EXE_prism`, which needs the binary in `prism-cli`.
- **A rasterizer crate apart from the window crate:** only `prism-cli` would use either. A split helps once a second consumer (Tier 2 apps, a GPU backend) exists.
- **A non-default feature:** `cargo run -- run app.prism` would silently be headless-only.
- **A `prism-desktop` dynamic library loaded with `dlopen` at the first `runApp`:** see D4.

### D3. The program keeps its thread; the main thread serves windows (autonomous; implements the threading of `compose-architecture` D20)

`prism run` compiles on the main thread as today, then starts the program with `Program::start` and, instead of joining it at once, serves the main thread:

```
let (server, windows) = prism_desktop::MainThreadServer::new();   // channels only
prism_ui::host::configure(HostConfiguration::Window(windows.factory()));
let running = program.start(main, arguments, check, move |event| windows.program_ended(event))?;
server.serve();          // returns once the program has ended
running.join()
```

- **`MainThreadServer::serve`** blocks on its request channel. Each request is one of:
  - `OpenWindow { title, width, height, reply }`: the server creates the event loop if needed (D4), runs it with `run_app_on_demand` until that window closes, and then waits again;
  - `Shutdown`, sent by `program_ended` or by the watch session (D17): `serve` returns.
- **`WindowFactory::open`,** called by `runApp` on the program thread, sends `OpenWindow` and blocks for the reply. The reply holds the window's actual logical size and scale factor, a `Receiver<WindowMessage>` of input, and the frame mailbox with the event loop's proxy (D10). While it waits, it parses the font (D6).
- **What runs where.** On the program thread: `main`, compositions, handlers, layout, display lists, hit testing, and text measurement. On the main thread: the event loop, the window, the `softbuffer` surface, rasterization, PNG encoding for windowed screenshots, and nothing compiled from Prism. So the program's stack, its stack-overflow check, `hot-reload`'s interrupt sentinel and the per-thread state of `prism-compose` and `prism-ui` are exactly as without windows.
- **Runtime errors** still end the process from the program thread with status 101, after flushing output. The window disappears with the process. The smoke test and the manual checklist cover this.

Rejected:
- **Running the composition on the main thread inside the event loop's callbacks.** `runApp` is called deep inside `main` on the program thread, so the app would have to hop threads in mid-call: the program thread would block while generated code ran on the main thread. That breaks the per-thread state of `prism-compose` and `prism-ui`, and the process-wide `STACK_LIMIT`, which is set for the program thread's stack (a check against the main thread's stack would be wrong both ways). It also breaks `hot-reload`'s single-loader rule for slots (its D3), and it ties the program's stack to the platform's main-thread size (8 MiB on macOS, 1 MiB on Windows, `ulimit` on Linux), against "The program runs on its own thread with an 8 MiB stack".
- **Running the whole program on the main thread** when it might open a window. Whether it will is unknown before `main` runs. It has the same stack problem.
- **A child process for the window,** with frames over shared memory. That doubles the moving parts for no isolation gain, since a runtime error should end both anyway.

Run-time cost for programs without windows: one channel and a blocked `recv` on the main thread instead of a `join`.

### D4. The window system starts lazily and is reused (autonomous)

Nothing in `prism-desktop` runs before the first `OpenWindow`. Then `EventLoop::<UserEvent>::with_user_event().build()` creates the event loop, with `with_default_menu(false)` on macOS (D14). It is kept in the server for the life of the process, since `winit` cannot create a second one (`RecreationAttempt`). Each app runs it with `EventLoopExtRunOnDemand::run_app_on_demand` and a fresh `WindowApp` handler, which creates the window in `resumed` and calls `exit()` when the window closes. `run_app_on_demand` exists on macOS, Windows, X11 and Wayland. Under `--watch` the same loop serves every fresh run of `main`.

**Linking cost on macOS.** `winit` links AppKit, Foundation, CoreGraphics and QuartzCore by `#[link]` attributes, so `dyld` loads them when `prism` starts, even for programs that never open a window. A C program linked with those four frameworks took 0.9 ms longer to start and exit than the same program without them: 2.5 ms against 1.6 ms median over 80 runs (macOS 27, arm64, 2026-10-18). That is outside `--timings`' "total to main", which counts from `main`. Task 1.3 measures `prism run`'s wall time on the startup benchmark before and after. On Linux, `winit` and `softbuffer` load X11 and Wayland with `dlopen` (`x11-dl`, `wayland-dlopen`), so nothing is loaded at startup. On Windows, the system libraries they use are loaded by every process anyway.

Rejected:
- **Loading `prism-desktop` as a dynamic library with `dlopen` at the first `runApp`.** It saves the 0.9 ms on macOS, but it needs a second artifact next to `prism` that `cargo build -p prism-cli` would not build, and a C ABI between two Rust artifacts. If the wall-time benchmark shows more than 1 ms of regression, the follow-up is to revisit this.
- **Creating the event loop at startup:** it costs every program the connection to the window server.

### D5. Pinned crate versions and features (autonomous)

| Crate | Version | Released | Licence | Features |
|---|---|---|---|---|
| `winit` | `=0.30.13` | 2026-03-02 | Apache-2.0 | `default-features = false`, `rwh_06`, `x11`, `wayland`, `wayland-dlopen`, `wayland-csd-adwaita-notitle` |
| `softbuffer` | `=0.4.6` | 2024-09-07 | MIT OR Apache-2.0 | `default-features = false`, `x11`, `x11-dlopen`, `wayland`, `wayland-dlopen` |
| `tiny-skia` | `=0.12.0` | 2026-02-02 | BSD-3-Clause | `default-features = false`, `std`, `simd`, `png-format` |
| `fontdue` | `=0.9.4` | 2026-07-29 | MIT OR Apache-2.0 OR Zlib | default (`simd`, `hashbrown`) |

Every version is more than two weeks old. Exact pins keep `Cargo.lock` changes deliberate, as for Cranelift (`=0.136.2`).

- **`softbuffer` 0.4.6, not 0.4.8.** On Apple platforms, 0.4.7 moved to `objc2` 0.6 and `objc2-*` 0.3, while `winit` 0.30 uses `objc2` 0.5 and `objc2-*` 0.2. With 0.4.8, two `objc2` families would be compiled. 0.4.7 and 0.4.8 add Android, wasm64, getters and a docs fix, none of which this change uses. `compose-architecture` built 0.4.8 here. Task 1.2 builds 0.4.6 and records `cargo tree -d -p prism-desktop`.
- **`tiny-skia` 0.12.0, not 0.11.4.** It is the current release, and it fixes a panic when drawing small anti-aliased rectangles, which 1-pixel carets and borders at fractional scales can hit. Its only breaking change is to `RadialGradient`, which this change does not use. `compose-architecture` built 0.11.4 here. If 0.12.0 fails to build, task 1.2 falls back to 0.11.4 and records it.
- **`winit` 0.30.13, not 0.31.** 0.31 is still in beta (0.31.0-beta.3, 2026-09-04).
- **Features** drop `winit`'s Adwaita title-bar font stack (`ab_glyph`) and `softbuffer`'s KMS backend. They keep X11 and Wayland through `dlopen`, so building on Linux needs no system packages.

**Applied (task 1.2, 2026-10-19).** `tiny-skia` 0.12.0 and `softbuffer` 0.4.6 build here (macOS 27, arm64, Rust 1.99), so no fallback was needed. `cargo tree -d -p prism-desktop --target aarch64-apple-darwin` shows a single `objc2` (0.5.3). Its duplicates are small leaf crates: `bitflags` 1 and 2, `core-foundation` 0.9 and 0.10, `core-graphics` 0.23 (`winit`) and 0.24 (`softbuffer`), `core-graphics-types` 0.1 and 0.2, and `miniz_oxide` 0.8 (`png`) and 0.9 (`flate2`, under `png`). The crate's normal dependency tree holds 71 crates, and building them cold took 39 s here in the dev profile.

Rejected: `wgpu` with `vello` or `glyphon` (`compose-architecture` D20: a large dependency tree and GPU-dependent output); `minifb` or `pixels` (fewer platforms, or `wgpu` underneath); `ab_glyph` or `swash` for glyphs (`fontdue` is the smallest and fastest rasterizer of the three, and shaping is out of scope, D8).

### D6. The bundled font: Inter 4.1 Regular, embedded and parsed once (autonomous; implements the font of `compose-architecture` D20)

`crates/prism-desktop/fonts/Inter-Regular.ttf` is the static Regular face from Inter 4.1's release archive (`extras/ttf/Inter-Regular.ttf`, released 2024-11-16). It sits beside its licence, `fonts/OFL.txt`: SIL Open Font License 1.1, copyright the Inter Project Authors, with no Reserved Font Name, so it may be embedded and later subset under its own name. `font.rs` embeds the file with `include_bytes!` and parses it in a `OnceLock<fontdue::Font>` on first use, with `FontSettings { scale: 40.0, load_substitutions: false, collection_index: 0 }`. `fontdue` builds the outline of every glyph the character map reaches when it loads a font. Skipping the substitution table keeps it from loading the alternates that it never draws without shaping (D8). One `&'static Font` serves both threads. The program thread starts parsing while the main thread creates the window (D3).

Run-time cost: the parse, once per process, measured by task 2.1 against a 10 ms target in release builds. Size: about 0.4 MB of the executable.

**Applied (task 2.1, 2026-10-19).** The archive is `Inter-4.1.zip` from the GitHub release `v4.1` of `rsms/inter`, published by `rsms` on 2024-11-16. Its 33,707,794 bytes match the asset size the GitHub API reports for that release, and its SHA-256 is `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`. `fonts/Inter-Regular.ttf` is its `extras/ttf/Inter-Regular.ttf`: 411,640 bytes, BLAKE3 `0c6305cd0a7a9f94daac842019537c6727fc0fadd4d44c336ec530317b42a42c`, which a unit test pins. `fonts/OFL.txt` is its `LICENSE.txt`, which declares no Reserved Font Name. The family name (name 1) is `Inter`; `fontdue`'s `Font::name()` gives the full name, `Inter Regular`, so the test reads the `name` table. The first `font()` takes 6.4 ms in a release build, under the 10 ms target, so no subset follow-up is needed.

Rejected:
- **Subsetting Inter now:** it needs `fonttools`, which this machine does not have. If the parse misses its target, a subset to Latin, Latin-1 and General Punctuation is the follow-up, made once with `pyftsubset` and committed with its command.
- **Inter's variable font:** `fontdue` reads no variation axes.
- **Roboto (Apache-2.0) or DejaVu Sans:** §5 chose Inter, which matches Compose Desktop's look, and Roboto would also do. DejaVu is larger and less like Compose.
- **System fonts:** not hermetic, they differ per machine, and they need a font-discovery dependency.

### D7. Windows lay text out with Inter's metrics, rounded per character (autonomous; answers `ui-toolkit` D12)

`metrics.rs::FontMetrics` implements `TextMeasurer` over the bundled font:

- `advance(c, size)` is `font.metrics(c, size).advance_width` rounded to the nearest integer. It is cached per font size in a 128-entry table for ASCII and a map for other characters.
- `line_height(size)` is `horizontal_line_metrics(size).new_line_size`, rounded. For Inter 4.1 at 16 pixels that is 19, where the headless metric gives 20. Task 2.2 confirmed it.
- The baseline is `round(ascent)` below the top of the line box: 16 at 16 pixels.

The rasterizer draws each glyph at the pen position these rounded advances give, scaled to physical pixels, not at `fontdue`'s fractional advances. So a text run is drawn exactly as wide as it was measured. Wrapping, centring in buttons, and carets after the last character all agree with what is drawn.

`WindowHost::measurer()` returns `FontMetrics`, and headless runs keep `FixedAdvanceMetrics`. A window's layout is therefore not the headless dump's: "Count: 0" is 64 pixels wide headless and as wide as Inter's eight rounded advances in a window. That is `ui-toolkit`'s documented trade-off. Fixtures pin the headless layout, and the golden images pin the window's text drawing (D20).

Run-time cost: one table lookup per character measured.

Rejected:
- **The fixed-advance metric in windows:** Inter glyphs in fixed cells look broken. A monospace font with an advance of exactly half an em is rare, and none looks like Compose.
- **Fractional advances:** they change `ui-toolkit`'s integer `TextMeasurer` and its exact dumps, for a sub-pixel gain at Milestone 2. Rounding each advance places glyphs up to half a logical pixel from their ideal positions, as hinted text on many platforms does. Sub-pixel positioning through a fixed-point measurer is a follow-up.
- **Inter's metrics headless:** rejected by `ui-toolkit` D12, since dumps would depend on the font file.

### D8. No shaping: one glyph per character (autonomous; states the limits of `fontdue`)

`fontdue` maps characters to glyphs through the character map, and it reads kerning only from the legacy `kern` table. Inter has its kerning in `GPOS`, which `fontdue` ignores. So:

- every Unicode scalar value is one glyph at its own rounded advance;
- there is no kerning (`AV` is spaced like `AX`), and no ligatures or contextual alternates;
- combining marks are not positioned over their base: they advance like other characters, usually by zero;
- there is no bidirectional reordering, no complex-script shaping (Arabic, Indic, Thai), no vertical text and no emoji;
- a character Inter lacks draws Inter's missing glyph (glyph 0) at that glyph's advance, and no fallback font is tried.

This matches `String`'s length, which counts Unicode scalar values, and the toolkit's wrapping, which works on characters. A shaping engine (`rustybuzz` with a fallback chain) is the follow-up for international text. It would change `TextMeasurer` from characters to runs.

### D9. HiDPI: logical layout, physical rasterization, snapped edges (autonomous)

- **The scale factor** is `window.scale_factor()`, an `f64` that may be fractional (1.25, 1.5 or 1.75 on Windows and Linux). The logical size reported to the app is `floor(physical / scale)` on each axis. The pixmap and the surface have the window's physical inner size.
- **Coordinates.** A logical coordinate `v` maps to `round(v × scale)` physical pixels. A rectangle's physical edges are its rounded corners in that mapping, so adjacent rectangles never overlap or gap, whatever the scale.
- **Axis-aligned rectangles** (radius 0) are filled without anti-aliasing at those snapped edges. They are crisp at every scale, and pixel-exact for the golden tests and screenshots.
- **Rounded rectangles** keep the snapped edges and anti-alias only their corners, with exact circular arcs: the radius is scaled and clamped to half the smaller side, and a pixel in a corner's square is covered by `clamp(r + 0.5 − d, 0, 1)`, where `d` is the distance from the pixel's centre to the corner circle's centre (revised at apply, D12).
- **Borders** are the outer rounded rectangle minus the inner one, inset by `round(width × scale)` physical pixels (at least 1), with an inner radius of `max(radius − width, 0)` scaled: a pixel's coverage is the outer coverage minus the inner one, clamped at 0. Unlike a centred stroke, the border then lies entirely inside the bounds, as `ui-toolkit` lays it out. Without radii, it is anti-aliasing-free.
- **Glyphs** are rasterized at `font_size × scale` pixels, and each is placed at the snapped pen position plus its bitmap offset. The glyph cache is keyed by glyph index and pixel size, so a scale change gives new entries rather than blurry scaled ones.
- **A scale change** (`ScaleFactorChanged`, for instance between a laptop and an external display) keeps the logical size when the platform scales the window with it. Then only the main thread redraws, from the last display list. If the logical size changes too, `winit` follows with `Resized`, which is delivered as a resize (D13).

Rejected: rasterizing at logical size and upscaling (blurry); layout in physical pixels (dumps and apps would depend on the display); anti-aliasing every edge (blurry seams between adjacent rectangles at fractional scales).

### D10. Frames cross in a latest-frame mailbox; the main thread rasterizes (autonomous)

`WindowHost::present(&list)` locks the window's `FrameMailbox { list: DisplayList, sequence: u64, closed: bool }`, copies the list in with `clone_from` (which reuses the mailbox's vectors and string), increments `sequence`, and sends `UserEvent::FrameReady` through the event loop proxy. The main thread handles that event with `window.request_redraw()`.

On `RedrawRequested`, the main thread rasterizes only when the sequence, the size or the scale factor changed since it last did. To do so, it swaps the mailbox's list with its own current list (O(1)), rasterizes, converts to the surface's pixel format, and presents. Otherwise it presents its last pixels again.

- **Latest frame wins:** frames that arrive faster than the display redraws overwrite each other, so there is no queue and no back-pressure on the program.
- **No allocation in the steady state:** the two lists' buffers trade places and keep their capacities (D19).
- **The main thread keeps the last list,** so a resize or scale change redraws at once, before the program's new layout arrives (D11).

Rasterizing on the main thread costs it a few milliseconds per frame (D19), during which input waits in the platform's queue. That is how single-threaded toolkits work, and it is well within a frame.

Rejected:
- **Rasterizing on the program thread and sending pixels:** the main thread could not redraw a resize or scale change without a round trip, and 7.7 MB frames at scale 2 would be copied once more.
- **Sending each list through a channel:** an allocation per frame and an unbounded queue.
- **Sharing one list under a lock while rasterizing:** it would block the program's next `present` for the whole rasterization.

### D11. Redraw only after a change; the platform paces frames (autonomous)

- **On the program thread,** `WindowHost::next_event` keeps a `frame_due` flag, true when the window opens.
  - It returns queued input first, and sets the flag after each input event.
  - When the queue is empty and the flag is set, it clears the flag and returns `HostEvent::Frame`.
  - Otherwise it blocks on the channel.

  `ui-toolkit`'s loop runs a pass and layout after each event before it asks again, so a burst of events gives one pass per event and one frame at the end. That is `ui-toolkit` D15's "a window host asks for a frame after each change", coalesced.
- **On the main thread,** the event loop's control flow is `ControlFlow::Wait`. A new frame, a resize, a scale change or an expose leads to `request_redraw`. The platform delivers `RedrawRequested` on its own drawing cycle: on macOS, through the view's `drawRect` tied to the display's refresh. `window.pre_present_notify()` is called before each present, as `winit` asks.
- **No timer, no polling and no spin:** an idle window blocks both threads, so a frame loop never runs without a cause.
- **Vsync** is the platform's: `softbuffer` presents a CPU buffer, so tearing depends on the compositor (none on macOS, Wayland or Windows with DWM).

Rejected: a 60 Hz timer (work when nothing changed); `ControlFlow::Poll` (a busy loop); drawing in `about_to_wait` (unpaced).

### D12. The rasterizer (autonomous)

`raster.rs::Rasterizer` owns a `tiny_skia::Pixmap` of the physical size, the glyph cache (`glyphs.rs`) and the `&'static Font`. `rasterize(&list, scale, physical_size)` resizes the pixmap only when the size changed, then runs the commands in order:

| Command | Drawing |
|---|---|
| `Clear { color }` | a fill of the whole pixmap |
| `Rectangle { bounds, color, corner_radius }` | snapped spans without anti-aliasing, or a rounded rectangle with anti-aliased corners (D9) |
| `Border { bounds, width, color, corner_radius }` | the outer rounded rectangle's coverage minus the inner one's (D9) |
| `Text { x, y, font_size, color, text }` | one glyph per character at measured positions (D7, D8), each coverage mask blended source-over into the pixmap's premultiplied pixels with the text colour |

- **Culling.** A command whose physical bounds miss the pixmap is skipped, and others are clipped to it. A 1,000-item list in an 800 × 600 window rasterizes only its visible rows.
- **Glyph cache.** Each entry holds `fontdue`'s metrics and coverage bitmap, keyed by glyph index and pixel size. It is cleared when it passes 4,096 entries, or when the scale factor changes.
- **Pixel format.** `softbuffer` takes `0x00RRGGBB` words. After a clear, every command draws over opaque pixels, so the premultiplied pixmap is opaque, and the conversion is one shift-and-or pass per pixel into the surface's buffer.
- **Full frames.** The whole window is redrawn each frame. Damage regions (`softbuffer`'s `present_with_damage`, and a list diff) are a follow-up, if the measurements in D19 ask for them.
- **Glyph blending** is the toolkit's own loop over the coverage mask, rather than a `tiny-skia` mask fill: a mask fill would build a `Mask` and a paint shader per glyph.

The same `Rasterizer` makes screenshots and golden images, without a window (D15, D20).

**Revision at apply: the rasterizer computes coverage itself (autonomous, 2026-10-19).** The design planned `tiny-skia`'s `fill_rect` and `fill_path` for rectangles, rounded rectangles and borders. `tiny-skia` 0.12's path filler allocates on every fill: an edge list (`Vec::with_capacity(64)`) and alpha runs as wide as the pixmap. The counter screen's button is a rounded rectangle, so a steady-state frame could not meet D19's "allocates nothing". `raster.rs` therefore fills spans and computes coverage itself: solid spans for straight rows, a distance to the corner's centre for corner pixels, and the difference of two such coverages for borders, all blended source-over in integer premultiplied arithmetic (`div255` rounding), like the glyphs. `tiny-skia` keeps the pixmap and the PNG encoding and decoding, and stays the tool for later shapes (paths, gradients, images), where an allocation per path is acceptable. Run-time cost: one square root per corner pixel; straight rows are plain span fills. It also makes rectangles and corners the same on every architecture, since no SIMD floating point is involved (glyph coverage still comes from `fontdue`). Rejected: `tiny-skia` paths with a per-frame allocation (against D19); caching each rounded rectangle's coverage mask (memory per distinct size, for no measured need).

### D13. Input mapping (autonomous)

`input.rs` turns `winit` window events into the host's messages with pure functions over small structs, so tests need no `winit` event (whose `KeyEvent` cannot be constructed outside `winit`):

- **`CursorMoved { position }`** records the cursor position in logical pixels, `floor(physical / scale)`, and delivers nothing (`ui-toolkit` dispatches no moves).
- **`MouseInput`** with `MouseButton::Left` delivers `Press(x, y)` or `Release(x, y)` at the recorded position. A press before any cursor move is dropped. Other buttons, `MouseWheel` and `Touch` deliver nothing. On every platform, the window that saw the press receives the release, even outside it.
- **`KeyboardInput`** with `ElementState::Pressed` (repeats included):
  - Cmd+W and Cmd+Q on macOS close the window (D14);
  - with Ctrl or Super held (`ModifiersChanged` keeps the state), nothing;
  - `NamedKey::Backspace` delivers `Key(Backspace)`, and `NamedKey::Enter` delivers `Key(Enter)`;
  - otherwise, each character of `event.text` at or above U+0020, other than U+007F, delivers `Character(c)`.

  Alt (Option on macOS) is allowed, since it composes characters there.
- **`Resized(physical)`** resizes the surface and delivers `Resize(logical)` when the logical size changed (D9). A zero width or height resizes nothing and draws nothing.
- **`ScaleFactorChanged`** redraws (D9). **`CloseRequested`** closes (D14). Focus, occlusion, IME, file drops and theme changes deliver nothing.

IME stays off (`set_ime_allowed(false)`, `winit`'s default), so dead keys and Option-compositions arrive as `text` on macOS and Windows. Full IME is a follow-up with text selection.

Rejected: delivering cursor moves (nothing uses hover yet, and each would cost a channel message); delivering `text` for key presses with Ctrl or Cmd held (`Cmd+A` would type `a`).

### D14. Window lifecycle (autonomous)

- **Creation.** `Window::default_attributes()` with `with_title(title)` and `with_inner_size(LogicalSize::new(max(width, 1), max(height, 1)))`. The window is visible at once: a window created hidden gets no redraw callback on some platforms, so it could never present its first frame. Until that frame, the main thread draws a white clear.
- **Resizing** is allowed, with no minimum size. Window options (fixed size, position, icon, fullscreen) are a follow-up, as new `runApp` parameters.
- **Closing.** On `CloseRequested`, the main thread:
  - marks the mailbox closed and sends `WindowMessage::Close` to the program thread;
  - drops the surface and the window, so the window disappears at once;
  - calls `exit()`, so `run_app_on_demand` returns and `serve` waits for the next request.

  The host's `next_event` returns `None` at `Close`, `ui-toolkit`'s loop drops the composition, and `runApp` returns. Later `present` calls see the closed mailbox and do nothing.
- **macOS shortcuts.** The default menu is disabled. Its Quit item calls `terminate:`, which would end the process without flushing the program's output or running the allocation check. So Cmd+Q and Cmd+W close the window as the close button does, and Cmd+Q does not end the program: `main` continues after `runApp`.
- **The program side closes too.** Dropping the `WindowHost`, when `runApp` returns for any reason, sends `UserEvent::CloseWindow` through the proxy, which closes the window the same way. So no window outlives its app.
- **Activation.** `winit`'s defaults apply on macOS: a regular app with a Dock icon, brought to the front when its window opens. The icon stays until the process exits.

Rejected: keeping the window open until `runApp` returns (it would linger while the composition is dropped); Cmd+Q ending the process (lost output, no leak check); the default `winit` menu.

### D15. `prism run --screenshot <file.png>` (autonomous)

`ui-toolkit`'s `run_app` gains a final-frame sink (D18): when the app ends, after the last event and before the composition is dropped, it rebuilds the display list if stale and calls `sink(&list, (width, height), host.scale_factor())`. The CLI sets the sink to `prism_desktop::screenshot::writer(path)` when `--screenshot` is given. That writer rasterizes the list on the program thread with its own `Rasterizer`, at scale 1 headless and at the window's last scale factor in windows, and encodes the result with `Pixmap::encode_png` as 8-bit RGBA without premultiplied alpha. Every `runApp` overwrites the file, so the last app wins. A write error raises `display error: cannot write the screenshot <path> (<io error>)`.

`--screenshot` needs the `window` feature, because it needs the rasterizer: without it, the option is a usage error. It works headless with or without `--events`. Fixtures do not compare screenshots (§5 D21), but `crates/prism-cli/tests/screenshot.rs` checks the size and exact pixels of one, which D9 makes reliable for rectangles.

Rejected:
- **A script command `screenshot "<file>"`:** it would change `ui-toolkit`'s grammar for a debugging aid, and it would make the file part of fixtures.
- **Screenshots of the presented surface:** `softbuffer` cannot read the surface back on every platform, and rasterizing the same list gives the same pixels anyway (D12).
- **Always at scale 1:** blurry on the display people use.

### D16. Display errors (autonomous)

`PanicKind::DisplayFailed` in `prism-runtime`, after `ui-toolkit`'s kinds, raised with `panic_without_location` and status 101, prints `display error: <detail>`. Its details are:

- `cannot open a window (<reason>)`: the event loop or the window could not be created, for instance on Linux without `DISPLAY` or `WAYLAND_DISPLAY`. The main thread puts `winit`'s error text in the reply, and the program thread raises it.
- `cannot write the screenshot <path> (<reason>)`.

A missing display is an environment problem, not a program bug, so it is not `ui misuse`. Running out of memory in a pixmap goes through the allocator as any allocation does.

Rejected: falling back to headless when no display is available (a program that wants a window would silently run without one, which `ui-toolkit` D15 also rejects).

### D17. Watch sessions leave the main thread (autonomous; completes `hot-reload` D2)

With the `window` feature, `prism run --watch` starts `WatchSession` on a thread named `watch-session` and serves the main thread as in D3. The session sends `Shutdown` to the server when it ends (Ctrl+C or an `--edits` script's end). Each fresh run of `main` (`hot-reload` D13) opens its windows through the same server and event loop (D4). When the program stops, by returning from `main` or with a runtime error, the session calls `windows.close_all()`. That sends `CloseWindow` to any open window, since a stopped program thread never drops its host.

For reloads, `WindowHost` exposes `wake()`, a plain `fn()` over a static sender of the open window's channel, as `defer_to_reload_points(wake: fn())` requires. It sends `WindowMessage::Wake`, which `next_event` turns into `HostEvent::Frame`, so `ui-toolkit`'s loop reaches its top, where `compose-hot-reload` puts its reload point. This change adds and tests the seam from Rust, and `compose-hot-reload` registers it.

Rejected: keeping the session on the main thread and the window on another thread (macOS forbids it); polling for reloads with a timer.

### D18. What changes in `prism-ui` (autonomous)

These are small additions to `ui-toolkit`'s code, so that the window host is a `Host` like the headless one:

- **`Host::measurer(&self) -> &'static dyn TextMeasurer`,** by default `&FixedAdvanceMetrics`, read once when the app starts (`ui-toolkit` D12: "the app takes its measurer when it starts"). The window host returns `&FontMetrics` (D7).
- **`Host::scale_factor(&self) -> f64`,** by default 1.0, for screenshots (D15).
- **`pub trait WindowFactory: Send + Sync { fn open(&self, title: &str, width: i64, height: i64) -> Box<dyn Host>; }`,** with `HostConfiguration::Window(Arc<dyn WindowFactory>)`. This is the "window factory for `desktop-window`" of `ui-toolkit` task 8.1.
- **`host::set_final_frame_sink(Box<dyn FnMut(&DisplayList, (i64, i64), f64) + Send>)`,** called by `run_app` when the app ends (D15).
- **`#[derive(Clone)]` on `DisplayList` and `DrawCommand`,** for `clone_from` (D10).

None of them is reachable from Prism, and `prism-ui` still has no window dependency. Rejected: a window host inside `prism-ui` behind a feature (the crate would build `winit` in some configurations, against `ui-toolkit` D2).

**Apply note (task 4.1, 2026-10-19).** The code `ui-toolkit` left differs in two places, and the hooks follow the code:
- **`Host::measurer` returns `Box<dyn TextMeasurer>`,** not `&'static dyn TextMeasurer`. `App` already owns its measurer as a box (`run_app_with(host, measurer, content)`), and `FontMetrics` caches advances in a `RefCell`, so it is not `Sync` and cannot be a `static`. `run_app` calls it once, before the first pass.
- **`WindowFactory` stays the type `ui-toolkit` defined,** `Arc<dyn Fn(&str, i64, i64) -> Box<dyn Host> + Send + Sync>`, rather than a trait with an `open` method: a closure needs no extra type, and `HostConfiguration::Window(WindowFactory)` already exists.
- **The final-frame sink** is `host::set_final_frame_sink(Option<FinalFrameSink>)`. `run_app_with` calls `App::present_final_frame` when the app ends normally, which rebuilds a stale list and passes the tree's window size, so a `resize` script command counts, and `Host::scale_factor`. An app that ends with an error calls nothing, since the error ends the program.

### D19. Costs, targets and how they are measured

| Run-time operation | Cost |
|---|---|
| a program without windows | one channel, a blocked `recv` on the main thread; on macOS, the frameworks' load time (0.9 ms, D4) |
| first window | event loop and window creation, in parallel with the font parse; first frame within 150 ms of `runApp` (target, reported by the smoke test) |
| present | O(commands + text bytes) copy into the mailbox, one proxy message, no allocation once grown |
| rasterization | O(visible commands × their pixel area + glyphs drawn) plus O(window pixels) for the clear and the conversion |
| glyph cache miss | one `fontdue` rasterization of the glyph |
| input event | one channel message; pointer moves cost no message |
| idle window | nothing on either thread |
| memory | the pixmap (4 bytes per physical pixel, 7.7 MB at 1600 × 1200), the main thread's display list, the mailbox's list, the glyph cache |

Targets, on a release build on a developer machine, at 800 × 600 logical pixels and scale 2 (1600 × 1200 physical):

- **Counter app frame,** from `present` to presented pixels: under 4 ms median. The clear and the conversion pass over 1.9 million pixels dominate it.
- **Todo app frame with 1,000 items,** about 15 rows visible: under 4 ms median, because culling skips what is outside the window.
- **Rasterizing 1,000 visible nodes** (a 40 × 25 grid of labelled rectangles): under 4 ms at scale 1 and under 8 ms at scale 2.
- **Ten times the nodes** take at most 15 times as long (linearity).

How they are measured:

- **`crates/prism-desktop/tests/raster_timing.rs`** (ignored), in the style of `compile_timing.rs`. It builds display lists directly, of 1,000 and 10,000 nodes in two shapes, "visible grid" and "todo column", at scales 1 and 2. It reports the median of 50 rasterizations after a warm-up, the glyph cache warm, and once with a cold cache. Under `PRISM_ASSERT_TIMING=1` it asserts the targets and linearity. The medians are recorded here at apply time.
- **The smoke test** (D20) reports the time to the first frame and the median frame time of the counter screen in a real window.
- **`crates/prism-desktop/tests/allocations.rs`:** with a counting `#[global_allocator]`, 100 presents and rasterizations of the same list at the same size allocate nothing after the first.
- **The startup benchmark** (`run_timing.rs`) runs before and after the change. Its wall time must not grow by more than 1 ms (D4).

The spec's scenario "Rasterization benchmark" asks for the 1,000-node lists under 4 ms at scale 2, stricter than the 8 ms above, and the benchmark asserts the spec's target. Medians at apply time (release build, 2026-10-19, macOS 27 on arm64, warm glyph cache; cold in parentheses): the counter frame, rasterized and converted into a surface buffer, 0.085 ms (0.26) at scale 1 and 0.33 ms (1.06) at scale 2; the grid of 1,000 nodes 0.18 ms (0.30) at scale 1 and 0.43 ms (0.55) at scale 2, of 10,000 nodes 0.97 and 1.48 ms, growth ×5.3 and ×3.4; the todo column of 1,000 nodes 0.072 and 0.19 ms, of 10,000 nodes 0.49 and 0.60 ms, growth ×6.8 and ×3.1. The clear and the conversion of 1.9 million pixels at scale 2 take most of a frame, and every target is met by a factor of ten or more, so damage regions are not needed.

Compile-time cost: no new compiler stage, analysis, MIR or backend work. Building `prism` cold compiles the crates of D2. An incremental rebuild after a compiler edit relinks a larger `prism`: task 1.3 measures both.

### D20. A new `desktop` capability, and how the change is tested

The window is a host of the toolkit, not a toolkit feature, so it gets its own capability. `execution` gains `--screenshot`. There is no new error code, since display errors are runtime errors.

- **Golden images** (`tests/raster_golden.rs`, `tests/golden/*.png`). Hand-built display lists cover:
  - rectangles at scales 1, 1.5 and 2, rounded rectangles, and borders with and without radii;
  - text at 12, 16 and 24 pixels, `AV` against `AX`, and `中`;
  - the counter screen's list from `ui-toolkit`'s "A dump of the counter screen";
  - culled and clipped commands.
  - at apply, also the counter screen laid out with the window's metrics at 200 × 120 (`counter-window`), which pins the window's layout as `ui-toolkit`'s tree builder makes it.

  Each image is compared within one unit per channel, which absorbs FMA and SIMD differences between architectures. `PRISM_UPDATE_GOLDEN=1` rewrites the images, and a changed golden image is reviewed by eye in the diff.
- **Unit tests** of `raster.rs` (snapping, culling, the pixel conversion, glyph placement from the measurer), `metrics.rs` (advances, line height, the cache), `input.rs` (every mapping in D13), and the window host's protocol in `host.rs`. The protocol tests drive the host through its channels with no window: input first, one frame after a burst, resize, close, wake, presents after close, and the "no display" error in a child process.
- **The smoke test** (`tests/window_smoke.rs`, `harness = false`, so its `main` runs on the process's main thread, as `winit` requires on macOS). It runs only with `PRISM_WINDOW_TESTS=1` and, on Linux, `DISPLAY` or `WAYLAND_DISPLAY`. Otherwise it prints `window smoke test skipped` and passes, so `cargo test --workspace` stays CI-safe. It:
  - serves the main thread and, on a program thread, opens a 320 × 200 window;
  - presents the counter screen's list, waits up to 5 s for the first presented frame, and checks the threads and the statistics;
  - waits 500 ms idle and checks that nothing was rasterized;
  - presents a changed list and checks one more frame;
  - drops the host and checks that the window closes and `serve` returns within 1 s.
- **The CLI test** `crates/prism-cli/tests/screenshot.rs` covers the execution scenarios.
- **The manual checklist** (tasks, group 8) covers what no test can: real clicks, typing, live resizing, moving between displays, closing shortcuts, and idle CPU.

## Risks / Trade-offs

- **[Window layout differs from headless dumps]** (D7) → It is documented in the `desktop` spec, golden images pin the window's text, and a headless mode with the window's metrics is a follow-up.
- **[0.9 ms of framework loading on macOS for every `prism run`]** (D4) → It is measured, and the trigger for the `dlopen` alternative is recorded.
- **[macOS live resize]** AppKit runs a modal loop while a window is resized. If proxy events wait until the resize ends, the app's relayout is late. → The main thread redraws the last list at the new size during the resize (D10). The checklist verifies it, and a synchronous relayout request is the fallback.
- **[Main-thread rasterization delays input]** by a few milliseconds per frame (D10). → Frames happen only after changes, and the targets keep them under 4 ms.
- **[Golden images across platforms]** `tiny-skia` and `fontdue` use SIMD and floating point. → One unit of tolerance per channel, exact rectangles, and regeneration with review.
- **[`softbuffer` 0.4.6 is older than 0.4.8]** → Nothing it lacks is used, and moving to `winit` 0.31 later brings `objc2` 0.6 with a current `softbuffer` together.
- **[Prerequisites unimplemented]** Every name taken from `ui-toolkit` and `hot-reload` comes from their packets. → Task group 1 checks them against the code.

## Migration Plan

No program changes meaning. A program that calls `runApp` without `--headless` used to end with `ui misuse: no window system is available; run with --headless`. It now opens a window, and it still gets that error in builds without the `window` feature. Programs without `runApp` run as before.

## Open Questions

None blocking. The follow-ups go to `ROADMAP.md` at archive:
- shaping and font fallback, a fixed-point `TextMeasurer`, and a subset font;
- damage-region redraws and a GPU backend;
- IME, selection, the clipboard, cursors and hover;
- window options and several windows;
- a headless mode with the window's metrics, and `--events` for windows;
- the `dlopen` split, if the startup benchmark asks for it;
- licence notices in a packaged release: the OFL text and `tiny-skia`'s BSD notice.
