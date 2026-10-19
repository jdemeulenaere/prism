# Desktop Specification

## Purpose
The desktop layer shows `runApp`'s apps in a native window. It covers the window's crate and build feature, the main thread that serves windows while the program keeps its own thread, window lifecycle, HiDPI scaling, CPU rasterization with the bundled font, the window's text metrics, redraws on change only, input mapping, display errors and the window's costs.

## Requirements

### Requirement: Window Code Is Linked Only Into the CLI
The window, its event loop, the rasterizer and the bundled font MUST be in a `prism-desktop` crate. Only `prism-cli` MUST depend on it, as an optional dependency enabled by its default `window` feature. No compiler crate, `prism-runtime`, `prism-compose` or `prism-ui` MUST depend on it or on a window, rasterization or font crate. A `prism` built without the `window` feature MUST have no window host (`ui`, "Apps Need a Window or Headless Mode").

#### Scenario: Only the CLI depends on the window crate
- **WHEN** a Rust test of `prism-desktop` reads every workspace member's `Cargo.toml`
- **THEN** `prism-cli` is the only member that names `prism-desktop`, as an optional dependency of its default feature `window`, and no member names `winit`, `softbuffer`, `tiny-skia` or `fontdue` except `prism-desktop`

#### Scenario: A build without the window feature
- **WHEN** `prism`, built with `cargo build -p prism-cli --no-default-features`, runs `fun main() { runApp { Text("hi") } }` with `import /prism/compose/ui` and `import /prism/compose/material3` and without `--headless`
- **THEN** stderr is exactly `panic: ui misuse: no window system is available; run with --headless` and a newline, and the exit status is 101

### Requirement: runApp Opens a Window
Under `prism run` without `--headless`, in a build with the `window` feature, a call of `runApp(title, width, height, content)` MUST open one window titled `title` with an inner size of `width` × `height` logical pixels, each at least 1. It MUST run the app in that window until the window closes. The app's root MUST have the window's actual inner size in logical pixels, which the platform may make differ from the requested size.

#### Scenario: Title and size of the window
- **WHEN** `fun main() { runApp(title = "Counter", width = 320, height = 200) { Text("hi") } }` runs with `prism run`
- **THEN** a window titled `Counter` opens with an inner size of 320 × 200 logical pixels, and the app's first `Resize` or initial size is 320 × 200 when the platform grants it

### Requirement: The Program Keeps Its Thread While the Main Thread Serves Windows
`main`, compositions, layout, handlers and the display list MUST run on the program thread that `prism run` starts, with its 8 MiB stack and its stack-overflow check, as for programs without windows. The window, its event loop, its surface and rasterization MUST run on the process's main thread. Input MUST cross from the main thread to the program thread, and frames from the program thread to the main thread, without generated code ever running on the main thread.

#### Scenario: Threads of a window app
- **WHEN** `crates/prism-desktop/tests/window_smoke.rs` runs with `PRISM_WINDOW_TESTS=1` on a machine with a display, and its program thread opens a window, presents a display list and closes the window
- **THEN** the window is created, rasterized and presented on the process's main thread, the host's events are received and its frames presented on the program thread, and the test ends with status 0

### Requirement: The Window System Starts Only When an App Opens a Window
A program that never calls `runApp` with a window MUST NOT create an event loop, connect to the display, load the bundled font or open a window. The event loop MUST be created when the first window opens and MUST be reused by every later window of the process.

#### Scenario: A program without an app
- **WHEN** a Rust test serves the main thread for a fake program that ends without opening a window
- **THEN** the server returns when the program ends, and its statistics show that no event loop was created and no font was loaded

### Requirement: Closing the Window Ends runApp
Closing the window, by its close button, the platform's close action, or Cmd+W or Cmd+Q on macOS, MUST close the window at once and end the app as the end of an event script does: `runApp` drops its composition, releasing every node, every handler and the values nodes own, and returns to its caller. Frames presented after the window closed MUST be discarded.

#### Scenario: Statements after runApp run
- **WHEN** `fun main() { runApp { Text("hi") }; println("closed") }` runs with `prism run --check-allocations`, and the window is closed by its close button
- **THEN** stdout is `closed` and a newline, and the exit status is 0

### Requirement: Apps Run One After Another
After an app's window has closed, a later call of `runApp` in the same process MUST open a new window and run its app in it, reusing the event loop.

#### Scenario: Two apps in sequence
- **WHEN** `fun main() { runApp(title = "One") { Text("1") }; runApp(title = "Two") { Text("2") } }` runs, and each window is closed in turn
- **THEN** a window titled `One` opens, and after it closes a window titled `Two` opens, and the program exits with status 0 after the second closes

### Requirement: Layout in Logical Pixels, Rasterization in Physical Pixels
The app MUST lay out in integer logical pixels whatever the display's scale factor. The window MUST be rasterized at physical pixels, the logical coordinates multiplied by the window's scale factor, which may be fractional. The logical size reported to the app MUST be the physical inner size divided by the scale factor, rounded down on each axis.

#### Scenario: A resize at a fractional scale
- **WHEN** a window at scale 1.5 is resized to an inner size of 601 × 451 physical pixels
- **THEN** the app receives `resize 400 300`

### Requirement: Edges Snap to Physical Pixels
Axis-aligned edges MUST be rounded to whole physical pixels and drawn without anti-aliasing, so adjacent rectangles neither overlap nor leave a gap at any scale factor. Rounded corners and glyphs MUST be anti-aliased.

#### Scenario: A rectangle at scale 2
- **WHEN** the rasterizer draws `clear 20x10 #FFFFFFFF` and `rect 2,2 4x3 #FF0000FF` at scale 2
- **THEN** the pixmap is 40 × 20 physical pixels, the pixels of columns 4 to 11 and rows 4 to 9 are `#FF0000FF`, and every other pixel is white

### Requirement: A Scale Change Redraws Without a Pass
When the window's scale factor changes and its logical size does not, the window MUST be redrawn at the new scale from the last display list, without a pass, a layout or a new display list.

#### Scenario: Moving to another display
- **WHEN** a window app's window moves from a display at scale 2 to one at scale 1 and keeps its logical size
- **THEN** the window is rasterized again at scale 1, and the app receives no event

### Requirement: Rasterization Is Deterministic
Rasterizing a display list at a given size and scale factor MUST give the same pixels on every run, and pixels within one unit per channel of the golden images on every supported platform. Display list commands MUST be drawn in order, each over the ones before it. A command entirely outside the window MUST be skipped, and a command partly outside it clipped.

#### Scenario: Golden images
- **WHEN** `crates/prism-desktop/tests/raster_golden.rs` rasterizes its display lists (rectangles, rounded rectangles, borders, the counter screen and text at sizes 12, 16 and 24) at scales 1, 1.5 and 2
- **THEN** every image matches its PNG under `tests/golden/` within one unit per channel

### Requirement: The Bundled Font
The window MUST draw text with Inter Regular, embedded in the `prism` executable and distributed with its SIL Open Font License 1.1 text. It MUST be parsed at most once per process, the first time a window needs it. No system font MUST be read.

#### Scenario: The font and its licence
- **WHEN** a Rust test of `prism-desktop` parses the embedded font and reads `fonts/OFL.txt`
- **THEN** the font parses, its family name is `Inter`, and the licence text names the SIL Open Font License, Version 1.1

### Requirement: Window Text Uses the Bundled Font's Metrics
In a window, layout MUST measure text with the bundled font's metrics. A character's advance at a font size MUST be its glyph's horizontal advance at that size, rounded to the nearest whole logical pixel. A line's height MUST be the font's ascent minus its descent plus its line gap at that size, rounded likewise. Headless runs MUST keep the fixed-advance metric of the `ui` requirement "Text Measurement", so a window's layout may differ from the headless dump of the same program.

#### Scenario: A text node's size in a window
- **WHEN** a window app composes `Text("Count: 0")` at the default font size 16
- **THEN** the text node is as wide as the sum of the eight characters' rounded advances of Inter at 16 pixels, and as high as Inter's rounded line height at 16 pixels

### Requirement: Glyphs Are Drawn Where Text Was Measured
Each glyph of a text run MUST be drawn at the pen position that the measured advances of the characters before it give, so a text run is drawn exactly as wide as it was measured.

#### Scenario: Glyph positions follow the measured advances
- **WHEN** the rasterizer draws `Count: 0` at size 16 and scale 1
- **THEN** each glyph is drawn at the sum of the rounded advances of the characters before it, and the last glyph's pen position plus its advance is the measured width

### Requirement: Text Is Not Shaped
Each Unicode scalar value of a text run MUST be drawn as one glyph, the glyph the font maps it to, at the advance the measurer gives it. Kerning, ligatures, contextual forms, combining-mark positioning, right-to-left ordering and font fallback MUST NOT be applied. A character the font has no glyph for MUST be drawn as the font's missing glyph, at the missing glyph's advance.

#### Scenario: No kerning
- **WHEN** the rasterizer draws the text `AV` at size 16
- **THEN** `V` is drawn at the rounded advance of `A`, exactly as far right of `A` as in the text `AX`

#### Scenario: A character missing from the font
- **WHEN** the rasterizer draws the text `中` at size 16
- **THEN** it draws the font's missing glyph, and the text measures as one missing glyph's rounded advance

### Requirement: The App Draws a Frame After Each Burst of Events
The window host MUST ask the app for a frame once when the window opens, and once after it has delivered one or more events and has none left to deliver. It MUST NOT poll, spin or run a timer while it waits for events.

#### Scenario: Events in a burst give one frame
- **WHEN** the window host's main thread side sends two press-and-release pairs before the program thread asks for its next event
- **THEN** the host delivers the four events one by one, each followed by the app's pass and layout, and then one `Frame`

### Requirement: The Window Redraws Only When Needed
The main thread MUST draw only on the platform's redraw callback, after a new frame arrived, the window was resized or its scale factor changed, or the platform asked to redraw. A redraw while the frame, the size and the scale factor are unchanged MUST present the last pixels again without rasterizing. When several frames arrive before a redraw, only the latest MUST be drawn. The main thread MUST NOT poll, spin or run a timer.

#### Scenario: Frames faster than redraws
- **WHEN** the program thread presents three display lists before the main thread's next redraw callback
- **THEN** the window rasterizes and presents only the third, once

### Requirement: Idle Windows Do No Work
When no input arrives and nothing invalidates the app, a window app MUST run no pass, layout, display list build or rasterization after its first frame, and both its threads MUST be blocked.

#### Scenario: An idle window
- **WHEN** the window smoke test leaves its window open with no input for 500 ms after the first frame
- **THEN** the window statistics show no frame presented and no rasterization during that time

### Requirement: Pointer Input
A press of the primary (left) mouse button MUST be delivered to the app as a press, and its release as a release, at the cursor's last position in logical pixels: the physical position divided by the scale factor, rounded down. A release outside the window MUST be delivered at its position, which the app then hits as nothing. Cursor moves, other buttons, scrolling and touch MUST NOT be delivered.

#### Scenario: A click at scale 2
- **WHEN** the cursor moves to physical position 101,61 in a window at scale 2, and the left button is pressed and released there
- **THEN** the app receives a press at 50,30 and a release at 50,30

### Requirement: Keyboard Input
Each printable character of the text a key press produces, including its repeats, MUST be delivered to the app as a typed character. Backspace and Enter MUST be delivered as the keys `backspace` and `enter`. Control characters, and key presses while Ctrl or Cmd is held, MUST NOT type anything. Other keys and IME composition MUST NOT be delivered. On macOS, Cmd+W and Cmd+Q MUST close the window.

#### Scenario: Typing, deleting and shortcuts
- **WHEN** a window receives the key presses `a`, `B` with Shift, Backspace, Ctrl+C, Cmd+A and Enter
- **THEN** the app receives the characters `a` and `B`, the key `backspace` and the key `enter`, in this order, and nothing for Ctrl+C and Cmd+A

### Requirement: Resizing the Window Resizes the App
A resize of the window MUST be delivered to the app as a resize to the new logical size, after which the app lays out again and presents a frame. Until that frame arrives, a redraw MUST draw the last display list at the new size, with the uncovered area cleared to white. A window with a zero physical width or height MUST draw nothing.

#### Scenario: A resize relays out the app
- **WHEN** a window app composes `Box(modifier = Modifier.fillMaxSize()) { }` and its window is resized to 200 × 100 logical pixels at scale 1
- **THEN** the app receives `resize 200 100`, and its next frame's box is 200 × 100

### Requirement: Display Errors
When `runApp` cannot open a window, for instance because no display is available, the program MUST end with the runtime error `display error: cannot open a window (<reason>)`, where the reason is the platform's, without a location line and with status 101.

#### Scenario: No display
- **WHEN** a Rust test serves the main thread with a window system that fails to start with the reason `no display`, and the program thread opens a window
- **THEN** the program thread's child process ends with stderr `panic: display error: cannot open a window (no display)` and a newline, and status 101

### Requirement: Watch Sessions Serve Windows From the Main Thread
Under `prism run --watch`, the watch session MUST run on a thread of its own and the main thread MUST serve windows, so a watched program can call `runApp`. When the program stops, by returning from `main` or by a runtime error, any window it left open MUST close.

#### Scenario: A watched window app
- **WHEN** `prism run --watch main.prism` runs a program whose `main` calls `runApp`, and the window is closed
- **THEN** the window opens, `runApp` returns when it closes, and the session prints `watch: main returned with status 0; waiting for changes`

### Requirement: Window Frames Meet Their Time Budget
On a release build on a developer machine, drawing one frame from its display list to presented pixels MUST take under 4 ms (median) for the counter app, and for a todo app showing a 1,000-item list, in a window of 800 × 600 logical pixels at scale 2. Rasterizing a display list MUST take time linear in the commands that reach the window and in the window's pixels. Benchmarks MUST report these times against the targets; they MUST NOT fail a test run on the time alone.

#### Scenario: Rasterization benchmark
- **WHEN** `crates/prism-desktop/tests/raster_timing.rs` runs with `PRISM_ASSERT_TIMING=1` on display lists of 1,000 and 10,000 nodes, all visible or mostly outside the window, at 800 × 600 logical pixels and scales 1 and 2
- **THEN** it reports the median rasterization time of each, the 1,000-node lists stay under 4 ms at scale 2, and the 10,000-node list takes at most 15 times as long as the 1,000-node list of its shape

### Requirement: Steady-State Window Frames Allocate Nothing
Once its buffers have grown, presenting a display list of an unchanged size and drawing it in a window of an unchanged size MUST allocate nothing in `prism-desktop`, on either thread.

#### Scenario: Allocation-counting test
- **WHEN** `crates/prism-desktop/tests/allocations.rs` presents 100 frames of the same display list through the window host and rasterizes each one, with a counting allocator
- **THEN** no allocation happens after the first two frames, by which both of the mailbox's lists have grown

### Requirement: Window Frames in Frame Statistics
With frame statistics on, an app in a window MUST also report, after its `frame` line, `frame stats: raster median <a> ms, p99 <b> ms, max <c> ms (<k> frames presented)`, the main thread's time to rasterize and present each presented frame, and `frame stats: input to present ...`, the time from the program thread taking a frame's first event to the main thread presenting that frame.

#### Scenario: A window app's frame statistics
- **WHEN** the window smoke test runs with `PRISM_WINDOW_TESTS=1` and frame statistics on, and presents two different display lists after one input event each
- **THEN** the window's report lines read `raster` with `(2 frames presented)` and `input to present` with two samples

### Requirement: Window Statistics Count Only Presented Frames
A frame replaced in the frame mailbox before it was drawn MUST have no raster or input-to-present sample. Measuring MUST allocate nothing on either thread after the window has opened.

#### Scenario: A replaced frame has no sample
- **WHEN** a Rust test of `prism-desktop` puts three display lists into the frame mailbox, each stamped with its first event, before the drawing side takes one, and the drawing side then takes and records it
- **THEN** the statistics hold one presented frame, one raster sample and one input-to-present sample, measured from the last list's stamp

### Requirement: Window Frames of a Large List Meet Their Budget
The manual verification MUST measure toggling items of `examples/apps/todo/large.prism` in an 800 × 600 logical-pixel window on a release build, and the 99th percentile from input to presented pixels SHOULD be under 16 ms at the display's scale factor.

#### Scenario: Toggling items of a thousand-item list in a window
- **WHEN** the manual verification of `reactive-app-verification` runs `prism run --frame-stats examples/apps/todo/large.prism` on a release build, toggles about twenty items and closes the window
- **THEN** the `input to present` line's 99th percentile is under 16 ms, and the run is recorded with its date, machine and scale factor

## Design Rationale

Decisions from `desktop-window` (planned autonomously on 2026-10-18, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`). What the app does in a window, apart from its text metrics, is the `ui` spec's; `--screenshot` is in `execution`.

- **The boundary** (D1). This capability implements `prism_ui::Host` for windows and rasterizes display lists. `ui-toolkit` defines the host interface, the frame loop and the display list; `compose-hot-reload` applies reloads between frames through the wake-up seam below; `reactive-app-verification` reads the window's frame times for `--frame-stats`.
- **A `prism-desktop` crate behind a default `window` feature of `prism-cli`** (D2). Only the `prism` binary links windows, so compiler edits never rebuild `winit`, `softbuffer`, `tiny-skia` or `fontdue`, and `cargo build -p prism-cli --no-default-features` builds a headless-only `prism` that links no AppKit. The rasterizer crates get `opt-level = 3` in the dev profile, since unoptimized rasterization is 10 to 30 times slower. Measured on 2026-10-19 (macOS 27, arm64): a cold build 2 s longer (in parallel with the compiler's crates), a compiler edit's rebuild 0.1 s longer, the release `prism` 1.47 MB larger (0.41 MB of it the font). Rejected: a separate binary crate (the fixture runner needs `CARGO_BIN_EXE_prism`), a separate rasterizer crate (no second consumer yet), a non-default feature (`cargo run` would silently be headless-only), a `dlopen`ed library.
- **The program keeps its thread; the main thread serves windows** (D3). `main`, compositions, handlers, layout and display lists stay on the 8 MiB program thread with its stack check, `hot-reload`'s interrupt and the per-thread state of `prism-compose` and `prism-ui`; the main thread runs the event loop, the window, the surface and rasterization, and no generated code. `prism_cli::windows::run_with_windows` compiles, starts the program with `program_ended` as its end notifier and serves until it ends; `run_file` stays the library's entry without windows. Rejected: running the composition inside the event loop's callbacks (hopping threads mid-call breaks per-thread state and the stack check), running the whole program on the main thread (its stack is the platform's), a child process for the window.
- **Lazy start, one event loop** (D4). Nothing runs before the first `OpenWindow`; then the event loop is created (without macOS's default menu) and kept for the process, since `winit` cannot create a second, and each app runs it with `run_app_on_demand`. A program without windows costs a channel and, on macOS, the frameworks `dyld` loads at startup: about 0.4 ms of wall time measured (total to `main` unchanged, 10.4 ms cold and 2.0 ms warm on `examples/run/main.prism`), under the 1 ms budget that would call for the `dlopen` split.
- **Pinned crates** (D5). `winit` 0.30.13, `softbuffer` 0.4.6 (0.4.7 moved to `objc2` 0.6, which would compile two `objc2` families beside `winit` 0.30), `tiny-skia` 0.12.0 and `fontdue` 0.9.4, without `winit`'s Adwaita font stack and `softbuffer`'s KMS backend, X11 and Wayland through `dlopen`.
- **Inter 4.1 Regular, embedded and parsed once** (D6). The OFL 1.1 face with no Reserved Font Name, pinned by its BLAKE3 hash, parsed on the program thread while the main thread creates the window (6.4 ms in release), without substitution tables since nothing is shaped. Rejected: a subset now (no tooling, and the parse is under its 10 ms target), the variable font, Roboto or DejaVu, system fonts.
- **Windows lay text out with Inter's metrics, rounded per character** (D7). Advances and the line height are Inter's rounded to whole logical pixels (19 at 16 pixels, against the headless 20), and glyphs are drawn exactly at those positions, so wrapping, centring and carets agree with the pixels. Headless runs keep the fixed-advance metric, so a window's layout differs from the headless dump of the same program; fixtures pin the headless layout and golden images the window's text. Rejected: fixed advances in windows (broken-looking text), fractional advances (would change the integer `TextMeasurer` and its exact dumps), Inter's metrics headless (dumps would depend on the font file).
- **No shaping** (D8). One glyph per Unicode scalar value through the character map: no kerning (Inter's is in `GPOS`, which `fontdue` ignores), ligatures, combining-mark positioning, bidi, complex scripts or fallback; a missing character draws glyph 0. A shaping engine would change `TextMeasurer` from characters to runs.
- **HiDPI** (D9). Layout in integer logical pixels; physical coordinates are `round(v × scale)`, so adjacent rectangles neither overlap nor gap at fractional scales; straight edges are not anti-aliased, corners and glyphs are; borders lie inside their bounds; the logical size is `floor(physical / scale)`; the glyph cache is keyed by pixel size. Rejected: rasterizing at logical size and upscaling, layout in physical pixels, anti-aliasing every edge.
- **A latest-frame mailbox; the main thread rasterizes** (D10). `present` copies the list into the mailbox with `clone_from` and announces it; the main thread swaps it with its own list on its redraw, so frames faster than redraws overwrite each other and the steady state allocates nothing, and the main thread can redraw a resize or a scale change at once from the last list. Rejected: rasterizing on the program thread (no immediate redraw, larger copies), a channel of lists (an allocation per frame, an unbounded queue), one shared list under a lock.
- **Redraw only after a change** (D11). The host asks for one frame after each burst of events; the main thread uses `ControlFlow::Wait` and draws on the platform's redraw callback, rasterizing only when the frame, the size or the scale changed. No timer, polling or spin: an idle window blocks both threads (0% CPU measured over 10 s). Rejected: a 60 Hz timer, `ControlFlow::Poll`, drawing in `about_to_wait`.
- **The rasterizer computes its coverage** (D12). Full-frame CPU rasterization with culling into a `tiny-skia` pixmap: span fills, exact circular corner arcs and border differences in integer premultiplied arithmetic, glyph masks from `fontdue` blended by its own loop, and one shift-and-or pass into `softbuffer`'s buffer. `tiny-skia`'s path filler allocates per fill, which would break "Steady-State Window Frames Allocate Nothing", so it keeps only the pixmap and PNG coding. Damage regions are a follow-up; every target is met by a factor of ten.
- **Input mapping** (D13). The handler turns each `winit` `WindowEvent` into a `WindowInput` and maps it with pure functions: the left button at the floored logical cursor, text at or above U+0020 (not U+007F) as characters, Backspace and Enter as keys, nothing with Ctrl or Cmd held, Alt allowed (it composes characters on macOS), IME off. Cursor moves deliver nothing, since nothing uses hover. `WindowSystem::deliver` sends a `WindowInput` to the open window through the event loop's proxy and `request_inner_size` asks the platform for a resize, so the opt-in `crates/prism-cli/tests/window_input.rs` drives real windows of real programs through the same path without a person. Rejected: posting platform events (an accessibility permission and an unlocked display), `--events` for windows (a follow-up feature).
- **Window lifecycle** (D14). Windows open visible (a hidden window gets no redraw callback on some platforms) with `runApp`'s title and logical size, resizable. The close button, Cmd+W and Cmd+Q close at once: the mailbox closes, the app gets `Close`, the window is dropped and the event loop exits, and `runApp` returns, so output is flushed and the allocation check runs. Dropping the host closes the window too. Rejected: the default menu, whose Quit ends the process without flushing.
- **Display errors** (D16). `display error: <detail>`, status 101, no location: a missing display is the environment's problem, not a program bug, so it is not `ui misuse`. Rejected: falling back to headless.
- **Watch sessions leave the main thread** (D17). With the `window` feature, `prism_cli::windows::watch_with_windows` runs the session on a `watch-session` thread with the main thread's 8 MiB stack, and the main thread serves every run's windows. When the program stops, the session closes the window it left (a stopped program thread never drops its host); when the session ends, the server stops. `WindowHost::wake` is the plain `fn()` reload wake-up for `defer_to_reload_points`, turned into `HostEvent::Wake` and then a `Frame`: the wake-up returns the frame loop to its reload point, so the frame presents the reloaded code. Rejected: the wake-up as a frame by itself, which was built before the reload was applied and left the new code unshown until the next input.
- **Hooks in `prism-ui`** (D18). `Host::measurer` (a boxed measurer, read once when the app starts), `Host::scale_factor`, the `WindowFactory` closure, a final-frame sink and `Clone` on display lists; none reachable from Prism, and `prism-ui` keeps no window dependency.
- **Costs and tests** (D19, D20). Release medians on 2026-10-19: the counter frame rasterized and converted in 0.33 ms at scale 2, 1,000 visible nodes in 0.43 ms, ten times the nodes at most ×5.3; the first frame about 80 ms after `runApp` in a real window. Golden PNGs within one unit per channel (`PRISM_UPDATE_GOLDEN=1` regenerates, and changed images are reviewed by eye); protocol and input-mapping tests without a window; an allocation-counting test; an opt-in smoke test (`window_smoke.rs`) and window input test (`window_input.rs`), both `harness = false` and skipped without `PRISM_WINDOW_TESTS=1`. What needs real hardware (the platform's own mouse and keyboard events, Retina sharpness, live resizing by hand, moving between displays) is a ROADMAP item for the user.

Decisions from `reactive-app-verification` (planned autonomously on 2026-10-18 and 2026-10-19, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **The window's frame statistics** (D7). `WindowStatistics` keeps two `const` histograms beside its counters: raster time, measured by the main thread when it presents a frame, and input to present, from the instant the program thread took the frame's first event, which travels with its display list in the frame mailbox, to that present. A list the latest-wins mailbox replaced before it was drawn has no sample. `runApp`'s report reads them after the window closed and prints `raster` (with the frames presented) and `input to present` after its `frame` line. Live allocations in a window move by a few between frames, since the input channel's blocks are allocated on the main thread and freed on the program thread; the constant-live property is measured headless (`example-apps`).
- **The 16 ms window budget of a 1,000-item list** (D9). Bounded from below by the headless frame (2.3 ms p99) and from above by the raster target (4 ms at scale 2), and checked by hand on a release build in a visible window, since `--events` cannot drive a window. On 2026-10-19 the screen was locked, the system throttled the window app (10.4 ms frames against 2.1 ms headless in the same process), and the 19.8 ms p99 measured there settles nothing; the check is left to the user.
