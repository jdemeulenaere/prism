# Execution Specification Delta

## MODIFIED Requirements

### Requirement: Program Arguments and Options
Arguments after `--` MUST be passed to the program and never interpreted by `prism`; no built-in reads them yet. The options go before the file:

- `--timings` MUST print to stderr the time spent in each stage (front end, code generation, linking) and the total time from the start of `prism` until `main` is called, before `main` runs. With `--watch`, it MUST also print the stages of each reload (`hot-reload`).
- `--watch` MUST run the program in a watch session that reloads it when its files change (`hot-reload`), passing the same arguments to every run of `main`.
- `--edits <script>` MUST drive that session's reloads from a script. It requires `--watch`, and without it is a usage error with status 1.

#### Scenario: Arguments after the separator
- **WHEN** `prism run main.prism -- --timings x` runs on `fun main() { println(1) }`
- **THEN** it prints `1` to stdout, nothing to stderr, and exits with status 0

#### Scenario: Reporting timings
- **WHEN** `prism run --timings main.prism` runs on `fun main() { println(1) }`
- **THEN** stderr holds one line per stage and one line with the total time to `main`, each in milliseconds, and stdout holds `1`

#### Scenario: Edits without watching
- **WHEN** `prism run --edits edits main.prism` runs
- **THEN** it prints the usage of `prism run` to stderr, runs nothing and exits with status 1

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory", and "stack overflow" raised inside glue, MUST write no location line. No further code of the program MUST run, and no drop MUST run. Under `prism run --watch`, the error MUST end the program's run instead of the process, and the watch session continues (`hot-reload`).

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

#### Scenario: A runtime error under watch
- **WHEN** `prism run --watch --edits edits app/main.prism` runs the same program, with an edit script whose only step is `after exit` and fixes `zero` to return `1`
- **THEN** stderr holds `panic: division by zero` and `  at app/main.prism:3:33`, then `watch: the program stopped with a runtime error; waiting for changes`, and the program runs again
