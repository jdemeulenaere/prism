# Desktop Specification Delta

## Purpose
The desktop layer shows `runApp`'s apps in a native window. It covers the window's crate and build feature, the main thread that serves windows while the program keeps its own thread, window lifecycle, HiDPI scaling, CPU rasterization with the bundled font, the window's text metrics, redraws on change only, input mapping, display errors and the window's costs.

## ADDED Requirements

### Requirement: Window Code Is Linked Only Into the CLI
The window, its event loop, the rasterizer and the bundled font MUST be in a `prism-desktop` crate. Only `prism-cli` MUST depend on it, as an optional dependency enabled by its default `window` feature. No compiler crate, the backend, `prism-runtime`, `prism-compose` or `prism-ui` MUST depend on `prism-desktop` or on a window, rasterization or font crate. A `prism` built without the `window` feature MUST behave as `ui-toolkit` specifies for a build without a window host.

#### Scenario: Only the CLI depends on the window crate
- **WHEN** a Rust test of `prism-desktop` reads every workspace member's `Cargo.toml`
- **THEN** `prism-cli` is the only member that names `prism-desktop`, as an optional dependency of its default feature `window`, and no member names `winit`, `softbuffer`, `tiny-skia` or `fontdue` except `prism-desktop`

#### Scenario: A build without the window feature
- **WHEN** `prism`, built with `cargo build -p prism-cli --no-default-features`, runs `fun main() { runApp { Text("hi") } }` with `import /prism/ui` and without `--headless`
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
Closing the window, by its close button, the platform's close action, or Cmd+W or Cmd+Q on macOS, MUST close the window at once and end the app as the end of an event script does: `runApp` drops its composition, releasing every node and the values nodes own, and returns to its caller. Frames presented after the window closed MUST be discarded.

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
In a window, layout MUST measure text with the bundled font's metrics. A character's advance at a font size MUST be its glyph's horizontal advance at that size, rounded to the nearest whole logical pixel. A line's height MUST be the font's ascent minus its descent plus its line gap at that size, rounded likewise. Headless runs MUST keep `ui-toolkit`'s fixed-advance metric, so a window's layout may differ from the headless dump of the same program.

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
- **THEN** no allocation happens after the first frame
