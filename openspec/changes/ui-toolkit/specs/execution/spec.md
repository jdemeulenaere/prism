# Execution Delta

## ADDED Requirements

### Requirement: Headless Runs
`prism run --headless[=<width>x<height>] [--events <script>] <file.prism>` MUST run the program with no window: `runApp` runs headless, in a window of the given size when one is given and of its own `width` and `height` otherwise. Without `--events`, `runApp` MUST compose and lay out its content and return. The options MUST come before the file, and `--events` without `--headless` MUST be a usage error with status 1.

#### Scenario: A headless run of the given size
- **WHEN** `prism run --headless=320x200 --events script main.prism` runs on `fun main() { runApp(width = 800, height = 600) { } }` with `import /prism/ui`, and `script` holds `dump`
- **THEN** stdout is `window 320x200` and a newline, and the exit status is 0

#### Scenario: Events without headless mode
- **WHEN** `prism run --events script main.prism` runs
- **THEN** stderr holds the usage of `prism run`, nothing runs, and the exit status is 1

### Requirement: Event Scripts Are Parsed Before the Program Runs
`prism run` MUST read and parse the event script before it compiles or runs the program. A script it cannot read, or a line it cannot parse, MUST be reported on stderr as `Error: <script>:<line>: <message>`, with status 1 and without running the program.

#### Scenario: An unknown script command
- **WHEN** `prism run --headless --events events.txt main.prism` runs a program that prints before calling `runApp`, and `events.txt` holds `clik "+"` on its first line
- **THEN** stderr is exactly `Error: events.txt:1: unknown command 'clik'` and a newline, stdout is empty, and the exit status is 1
