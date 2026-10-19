# Hot Reload Specification Delta

## Purpose
Defines `prism run --watch`: a watch session that keeps a program running while its source files change. This change adds what compositions need from it: the contract of restart functions, and how edit scripts and pending reloads behave while a frame loop defers reloads to its reload points.

## ADDED Requirements

### Requirement: A Restart Function's Contract Covers Its Composable's
The contract of a restart function's slot MUST include the contract of the composable it restarts, so that a reload that changes that composable's parameter types gives the restart function a new slot too. Such a split MUST be reported only by the composable's own note, `reload: the signature of <symbol> changed; ...`.

#### Scenario: A changed parameter type splits the restart function
- **WHEN** a Rust test of `prism-codegen` computes the contracts of `compose fun Label(n: Int)`'s restart function and of `compose fun Label(n: Double)`'s
- **THEN** the two contracts differ, while the contracts of the restart functions of `compose fun Label(n: Int) { Text("a") }` and `compose fun Label(n: Int) { Text("b") }` are equal

### Requirement: Leaving Reload Points Applies a Pending Reload at the Next Call
When a frame loop stops deferring reloads to its reload points, a reload that is still pending MUST be applied at the program's next call, as if it had been requested then.

#### Scenario: A reload pending when runApp returns
- **WHEN** a Rust test requests a reload while a program defers reloads to reload points, and the program then stops deferring them and calls a function
- **THEN** the reload is applied at that call

## MODIFIED Requirements

### Requirement: Edit Triggers
The trigger `after line <n>` MUST stop the program when it writes its `n`-th newline in the current run. The session then applies the step's edits and runs the reload, and the program applies the reload at that point, if it compiled, and continues. While a frame loop defers reloads to its reload points, the program MUST still stop at that newline and let the session prepare the reload, but the reload MUST be applied at the loop's next reload point rather than at that output call. The trigger `after exit` MUST apply its edits once no program is running. Consecutive steps with the same trigger MUST form one reload.

#### Scenario: Two files edited in one reload
- **WHEN** the program of "A body edit compiles one file" runs under `after line 1: edit lib/math.prism lib/math.prism.v2` and `after line 1: edit app/main.prism app/main.prism.v2`
- **THEN** stderr holds one `reload: applied lib/math.prism, app/main.prism` line

#### Scenario: A line printed during a pass
- **WHEN** a headless counter app whose `Counter` prints `composing` runs under `--watch` with the edit script `after line 1: edit main.prism main.prism.v2`, which changes the label, and the event script `click "+"` and `dump`
- **THEN** the pass that printed `composing` finishes on the old code, and the dump holds the new label with the count `1`
