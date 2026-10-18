# Execution Delta

## ADDED Requirements

### Requirement: Undisposed Compositions Are Reported
Under `--check-allocations`, when `main` returns, compositions of the program thread that were started and not disposed (compose, "Disposing a Composition Releases Everything") MUST be reported, after the output is flushed and before any report of live allocations, as `allocation check: compositions still live at exit: <n>` on stderr, and the program MUST exit with status 102. Without the option they MUST NOT be counted.

#### Scenario: A composition that is never disposed
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println("ok") } }` run with `--check-allocations`
- **THEN** stdout holds `ok`, stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

## MODIFIED Requirements

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory", "stack overflow" raised inside glue, and the "composition misuse" errors of the compose runtime (compose, "Misusing the Composition Protocol") MUST write no location line. No further code of the program MUST run, and no drop MUST run. Under `prism run --watch`, the error MUST end the program's run instead of the process, and the watch session continues (`hot-reload`).

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

#### Scenario: A runtime error under watch
- **WHEN** `prism run --watch --edits edits app/main.prism` runs the same program, with an edit script whose only step is `after exit` and fixes `zero` to return `1`
- **THEN** stderr holds `panic: division by zero` and `  at app/main.prism:3:33`, then `watch: the program stopped with a runtime error; waiting for changes`, and the program runs again

#### Scenario: A composition misuse has no location
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> endGroup(k) } }` run
- **THEN** stderr is exactly `panic: composition misuse: end of a group that was not started` and a newline, with no location line
