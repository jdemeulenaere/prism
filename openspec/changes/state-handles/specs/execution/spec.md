# Execution Delta

## ADDED Requirements

### Requirement: State Errors Are Runtime Errors at the Access
The runtime errors of state, `stale state handle`, `state accessed while borrowed`, `state written during composition` and `state created during composition outside remember`, MUST end the program as every runtime error does (execution, "Runtime Errors End the Program"), with the location of the access, write or creation that failed.

#### Scenario: Location of a stale access
- **WHEN** `app/main.prism` reads a stale handle with `println(kept.value)` on line 9, `kept.value` starting at column 13
- **THEN** stderr is exactly `panic: stale state handle` and `  at app/main.prism:9:13`, each followed by a newline, and the exit status is 101

### Requirement: Program Cells Are Released Before the Allocation Check
Under `--check-allocations`, every cell that the program owns MUST be freed, its value dropped, when `main` returns, before compositions still live are reported and before the allocation check reports. Without the check, program cells MUST NOT be freed.

#### Scenario: A program cell holding a composition
- **WHEN** `compose fun Empty() { }` and `fun main() { val s = mutableStateOf(composition { Empty() }) }` run with `--check-allocations`
- **THEN** the program exits with status 0: releasing the cell disposes the composition before live compositions are counted
