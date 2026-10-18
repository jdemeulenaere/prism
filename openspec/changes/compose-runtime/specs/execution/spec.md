# Execution Delta

## ADDED Requirements

### Requirement: Undisposed Compositions Are Reported
Under `--check-allocations`, when `main` returns, compositions of the program thread that were started and not disposed (compose, "Disposing a Composition Releases Everything") MUST be reported, after the output is flushed and before any report of live allocations, as `allocation check: compositions still live at exit: <n>` on stderr, and the program MUST exit with status 102. Without the option they MUST NOT be counted.

#### Scenario: A composition that is never disposed
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println("ok") } }` run with `--check-allocations`
- **THEN** stdout holds `ok`, stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

#### Scenario: An undisposed composition and what it holds
- **WHEN** `fun Greeting(k: Int, name: String) { startRestartGroup(k, 3); println("hello " + name); endRestartGroup(k) { c -> Greeting(c, name.clone()) } }` is declared, and `fun main() { val c = startComposition { k -> Greeting(k, "a" + "b") } }` runs with `--check-allocations` and `import /prism/composer`
- **THEN** stdout is `hello ab`, stderr is the line `allocation check: compositions still live at exit: 1` followed by the line of live allocations, which counts the restart function's environment and its string, and the exit status is 102

## MODIFIED Requirements

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory", "stack overflow" raised inside glue, and the "composition misuse" errors of the compose runtime (compose, "Composition Misuse Is a Runtime Error") MUST write no location line. No further code of the program MUST run, and no drop MUST run.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

#### Scenario: A composition misuse has no location
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> endGroup(k) } }` run
- **THEN** stderr is exactly `panic: composition misuse: end of a group that was not started` and a newline, with no location line
