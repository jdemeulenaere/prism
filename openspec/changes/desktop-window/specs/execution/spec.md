# Execution Delta

## ADDED Requirements

### Requirement: Screenshots of an App
`prism run --screenshot <file.png> <file.prism>`, in a build with the `window` feature, MUST write a PNG image of the app's final frame to `<file.png>` each time `runApp` returns, after the app's last event and before its composition is dropped. The image MUST be the final display list rasterized as a window draws it, at scale 1 in headless runs and at the window's scale factor otherwise, as 8-bit RGBA without premultiplied alpha. The option MUST come before the file.

#### Scenario: A headless screenshot
- **WHEN** `prism run --headless=40x30 --screenshot shot.png main.prism` runs on `fun main() { runApp { Box(modifier = Modifier.size(10).background(Color.Red)) { } } }` with `import /prism/ui`
- **THEN** the exit status is 0, and `shot.png` is a 40 × 30 image whose pixels at columns 0 to 9 and rows 0 to 9 are `#FFFF0000` and whose other pixels are white

### Requirement: Screenshot Errors
`--screenshot` without a file name MUST be a usage error with status 1, and nothing MUST run. When the file cannot be written, the program MUST end with the runtime error `display error: cannot write the screenshot <file.png> (<reason>)`, without a location line and with status 101.

#### Scenario: A screenshot without a file name
- **WHEN** `prism run --screenshot` runs with nothing after the option
- **THEN** stderr holds the usage of `prism run`, nothing runs, and the exit status is 1

#### Scenario: A screenshot that cannot be written
- **WHEN** `prism run --headless --screenshot missing/shot.png main.prism` runs an app, and the directory `missing` does not exist
- **THEN** stderr ends with `panic: display error: cannot write the screenshot missing/shot.png (` followed by the platform's reason, `)` and a newline, and the exit status is 101
