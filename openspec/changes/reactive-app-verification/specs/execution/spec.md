# Execution Delta

## ADDED Requirements

### Requirement: Live Runtime Objects Are Reported
Under `--check-allocations`, when `main` returns with no live composition, the program thread's live groups, slots, state cells and nodes MUST be reported after the composition report and before the allocation report, one line per non-zero count in this order: `allocation check: groups still live at exit: <n>`, then `slots`, `state cells` and `nodes` in the same form. Any line MUST make the exit status 102. Without the option nothing MUST be counted.

#### Scenario: A leaked group, cell and node are reported
- **WHEN** a Rust test leaks one group, one state cell and one node through the runtime's test hooks, with no composition live, and runs the program's finish report
- **THEN** the report is exactly the lines `allocation check: groups still live at exit: 1`, `allocation check: state cells still live at exit: 1` and `allocation check: nodes still live at exit: 1`

### Requirement: A Live Composition Hides What It Holds
When a composition is still live as `main` returns under `--check-allocations`, only the report of undisposed compositions MUST be printed, since its groups, slots, cells and nodes are expected to be live.

#### Scenario: A live composition hides what it holds
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println("ok") } }` run with `--check-allocations`
- **THEN** stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

#### Scenario: An app that ends cleanly reports nothing
- **WHEN** `fun main() { runApp(width = 200, height = 100) { val c = remember { mutableStateOf(0) }; Text("n = ${c.value}") } }` runs with `import /prism/ui` under `prism run --check-allocations --headless`
- **THEN** stderr is empty and the exit status is 0

### Requirement: Frame Statistics Option
The option `--frame-stats`, before the file, MUST make every `runApp` of the program measure its frames and print its report to stderr when it returns (`ui`, "Frame Statistics Report"), in headless runs, in windows and under `--watch`. It MUST NOT change the program's stdout or its exit status, and a program that never calls `runApp` MUST print nothing more.

#### Scenario: Frame statistics without an app
- **WHEN** `prism run --frame-stats main.prism` runs on `fun main() { println(1) }`
- **THEN** stdout is `1` and a newline, stderr is empty, and the exit status is 0

#### Scenario: Frame statistics of a headless app
- **WHEN** `prism run --frame-stats --headless --events events main.prism` runs the counter screen of `ui`'s "Counter screen" in `runApp`, with the script `click "+"` and `frame`
- **THEN** stdout is empty, stderr starts with a line `frame stats: frames 1, events 2, passes 2, reloads 0, ` and holds the lines of the report, and the exit status is 0
