# Desktop Specification Delta

## Purpose
Running apps in a native window. This change adds the window's part of the frame statistics that `--frame-stats` reports, and the window frame budget of a large list, to the capability that `desktop-window` creates.

## ADDED Requirements

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
