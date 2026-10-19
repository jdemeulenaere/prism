# Hot Reload Specification

## Purpose
Defines `prism run --watch`: a watch session that keeps a program running while its source files change. It covers how changes are detected, what a reload compiles, where and when a reload is applied, which code old frames and old function values call after it (slot contracts), what happens to top-level values, how long old code stays mapped, what errors and runtime errors do to the session, how `main` runs again after it returns, how scripted edits drive reloads deterministically, and the reload latency target. What a single run of a program does is in `execution`; the function table and the code it holds are in `code-generation`.

## Requirements

### Requirement: Watching a Running Program
`prism run --watch <file.prism>` MUST compile and run the program as `prism run` does, and keep the process running after `main` returns. It MUST watch every file of the program and the target path of every unresolved import. When one of them changes, it MUST reload the program, patching the running program without restarting it. The session MUST end only when interrupted, when its edit script ends, or when its workspace root is removed.

#### Scenario: Saving an edited body while the program runs
- **WHEN** `prism run --watch main.prism` runs `fun value(): Int = 1` and `fun main() { var n = 0; while (value() == 1) { n += 1 }; println(value()) }`, and `main.prism` is then saved on disk with `fun value(): Int = 2`
- **THEN** the program prints `2` without being restarted, and stderr holds `reload: applied main.prism` and then `watch: main returned with status 0; waiting for changes`

### Requirement: Only the Program's Files Are Watched
A change of a file that is not one of the program's files, nor the target of one of its unresolved imports, MUST not reload the program. A file the program stops importing is no longer watched, so it MUST be read again when an import brings it back.

#### Scenario: An edit to a file outside the program does nothing
- **WHEN** `fun main() { println(1) }` runs under `after exit: edit notes.prism notes.prism.v2`, and no file of the program imports `notes.prism`
- **THEN** stdout is exactly `1`, stderr is exactly `watch: main returned with status 0; waiting for changes`, and the session exits with status 0

#### Scenario: A file imported again is read again
- **WHEN** a Rust test of the watch session runs `main.prism`, which imports `helper.prism`, then edits `main.prism` to drop the import, then changes `helper.prism` on disk so that `value` divides by zero, and then edits `main.prism` to import it again
- **THEN** the last run of `main` stops with `panic: division by zero` in `helper.prism`

### Requirement: Session Status Lines
The program's output MUST go to stdout, and the session's status lines to stderr, each starting with `reload:` or `watch:`. An applied reload MUST print `reload: applied <files>`. `<files>` lists the files whose code units the reload defines, compiled or loaded from the build cache, relative to the workspace root, in topological order, separated by `, `. When the reload defines instance units, the list MUST be followed by ` (<n> instance)` or ` (<n> instances)`.

#### Scenario: A body edit compiles one file
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(1)); println(twice(1)) }`, and the edit script `after line 1: edit lib/math.prism lib/math.prism.v2` replaces the body with `n * 3`
- **THEN** stdout holds `2` and `3`, stderr is exactly `reload: applied lib/math.prism` and `watch: main returned with status 0; waiting for changes`, and the session exits with status 0

### Requirement: Reloads Check Only Files Whose Check Key Changed
A reload MUST parse again only the files whose content changed. It MUST check again only the files whose check key changed, using the key `prism check` uses for its build cache (`build-cache`). A change that leaves every file's content unchanged MUST do nothing.

#### Scenario: A signature change compiles its importers
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2` and `lib/text.prism` declares `public fun greet(): String = "hi"`, `app/main.prism` imports both and declares `fun main() { println(twice(1)); println(greet()) }`, and `after line 1: edit lib/math.prism lib/math.prism.v2` changes the declaration to `public fun twice(n: Int, k: Int = 2): Int = n * k`
- **THEN** stderr begins with `reload: applied lib/math.prism, app/main.prism`, which does not name `lib/text.prism`

### Requirement: Reloads Compile Only Changed Units
A reload MUST compile exactly the code units whose code key in the program's build plan (`build-cache`) is new or changed: the units of the files it checked again, and the instance units whose generic's record or type arguments changed. It MUST define them into one new JIT module, linked against the program's function table, and leave the code and slots of every other unit unchanged.

#### Scenario: A generic body edit compiles its instances
- **WHEN** `lib/pick.prism` declares `public fun <T : Copy> pick(a: T, b: T): T = a`, `app/main.prism` imports it and declares `fun main() { println(pick(1, 2)); println(pick(1, 2)) }`, and `after line 1: edit lib/pick.prism lib/pick.prism.v2` changes the body to `b`
- **THEN** stdout holds `1` and `2`, and stderr begins with `reload: applied lib/pick.prism (1 instance)`

### Requirement: Reloads Are Applied on the Program's Thread at Its Next Call
A reload MUST be compiled while the program keeps running. It MUST be applied on the program's own thread at the next safe point: entry to a compiled body, entry to glue, or a runtime call at which an edit script stops the program. All of a reload's slot changes MUST take effect together, between two operations of the program.

#### Scenario: A running loop reaches new code at its next call
- **WHEN** `fun step(i: Int): Int = i` and `fun main() { var i = 0; while (i < 4) { println(step(i)); i += 1 } }` run under the script `after line 2: edit main.prism main.prism.v2`, where `main.prism.v2` declares `fun step(i: Int): Int = i * 10` and the same `main`
- **THEN** stdout holds `0`, `1`, `20` and `30`

### Requirement: Running Frames Keep Their Code
A frame that is already running when a reload is applied MUST continue in the code it entered. Every later call of a slot that the reload repointed MUST run the new code, whether it comes from new code, from a frame of old code, or through a function value made before the reload.

#### Scenario: A running main keeps its own body
- **WHEN** `fun main() { println(1); println(2) }` runs under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `fun main() { println(10); println(20) }`
- **THEN** stdout holds `1` and `2`, and stderr holds `reload: applied main.prism`

#### Scenario: A function value made before a reload runs the new body
- **WHEN** `fun main() { val f: (Int) -> Int = { n -> n + 1 }; println(f(1)); println(f(1)) }` runs under `after line 1: edit main.prism main.prism.v2`, where the closure becomes `{ n -> n + 2 }`
- **THEN** stdout holds `2` and `3`

### Requirement: Slots Record the Contract of Their Code
Each slot of the function table MUST record the contract of the code it holds: the type fingerprints of its parameter and result types and, for a closure's code, its environment's captures with their modes and type fingerprints. A type fingerprint MUST be a structural hash of the type. It covers the names, markers, type arguments, fields, variants and payloads of every struct and enum the type reaches, with type arguments substituted.

#### Scenario: A changed field type keeps old values with old code
- **WHEN** `struct P { val x: Int }`, `fun show(p: &P) { println(p.x) }` and `fun main() { val p = P(1); show(p); show(p) }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `struct P { val x: Double }`, `fun show(p: &P) { println(p.x + 0.5) }` and `fun main() { val p = P(1.0); show(p); show(p) }`
- **THEN** stdout holds `1` and `1`, and stderr notes that the signature of `main::show` changed

#### Scenario: A changed state type keeps old handles with old code
- **WHEN** `val Count: MutableState<Int> = mutableStateOf(1)` and `fun main() { var i = 0; while (i < 3) { Count.value += 1; println(Count.value); i += 1 } }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `val Count: MutableState<String> = mutableStateOf("a")` and a `main` that writes it
- **THEN** stdout holds `2`, `3` and `4`, stderr notes that the signatures of `main::Count#init` and `main::Count#get` changed, and the session exits with status 0: the running `main` keeps the handle of its `MutableState<Int>` cell

### Requirement: Type Fingerprints Depend on the Type Alone
A type's fingerprint MUST be the same in every unit and in every version of a unit that uses the type: it MUST not depend on the order in which a unit declares or uses its types.

#### Scenario: Reordering declarations keeps their contracts
- **WHEN** `struct P { val x: Int }`, `fun show(p: &P) { println(p.x) }`, `fun take(p: P): Int = p.x` and `fun main() { val p = P(1); var i = 0; while (i < 3) { show(p); i += 1 } }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `take` before `show` and `show` prints `p.x * 10`
- **THEN** stdout holds `1`, `10` and `10`, and stderr is exactly `reload: applied main.prism` and `watch: main returned with status 0; waiting for changes`

### Requirement: A Slot Is Repointed Only to Code With the Same Contract
A reload MUST repoint a symbol's slot only to code with the same contract. Code with a different contract MUST get a new slot for the symbol, which code compiled from then on calls, while code compiled before keeps calling the old slot and its old code. No reload MUST be refused, and no live value MUST be converted, because a type's layout changed.

#### Scenario: A changed signature keeps old callers on the old function
- **WHEN** `fun step(i: Int): Int = i` and `fun main() { var i = 0; while (i < 3) { println(step(i)); i += 1 } }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `fun step(i: Int, k: Int): Int = i * k` and calls `step(i, 10)`
- **THEN** stdout holds `0`, `1` and `2`

#### Scenario: A closure whose captures changed keeps its old code for old values
- **WHEN** `fun main() { val k = 1; val f: (Int) -> Int = { n -> n + k }; println(f(1)); println(f(1)) }` runs under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `val j = 5` before `f` and the closure `{ n -> n + k + j }`
- **THEN** stdout holds `2` and `2`

### Requirement: Contract Changes Are Reported
When a reload gives a symbol a new slot while the program runs, it MUST say so on stderr after its `reload: applied` line. For a function the line is `reload: the signature of <symbol> changed; code already running keeps calling its previous version`. For a closure it is `reload: the captures of <symbol> changed; values made before keep calling its previous version`.

#### Scenario: Reporting a changed signature
- **WHEN** the program of "A changed signature keeps old callers on the old function" runs under its script
- **THEN** stderr holds `reload: applied main.prism` and then `reload: the signature of main::step changed; code already running keeps calling its previous version`

#### Scenario: Reporting changed captures
- **WHEN** the program of "A closure whose captures changed keeps its old code for old values" runs under its script
- **THEN** stderr holds `reload: the captures of main::main#closure0 changed; values made before keep calling its previous version`

### Requirement: Old Code Stays Mapped
The code and data of every JIT module MUST stay mapped for as long as the program runs: its string literals' data, its closures' descriptors and its glue included. So running frames, function values, drop entries, slots and strings made by old code stay valid. A module MUST be freed only with its program, when the process exits or `main` runs again in a fresh program.

#### Scenario: A string literal of old code outlives its reload
- **WHEN** `fun greeting(): String = "hello"` and `fun main() { val g = greeting(); println(g); println(g); println(greeting()) }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where `greeting` returns `"bye"`
- **THEN** stdout holds `hello`, `hello` and `bye`, and the session exits with status 0

### Requirement: Top-Level Values Are Kept Unless Their Declaration Changed
A top-level value's storage MUST belong to the program, keyed by three things: the value's symbol, its type fingerprint, and a hash of its declared type and initializer that ignores positions. A reload that keeps all three MUST keep the value, so an initialized value is not initialized again. A reload that changes any of them MUST give the value new storage, which is initialized at the next read, as on a first read.

#### Scenario: An unchanged value keeps its value
- **WHEN** `fun noisy(): Int { println(0); return 5 }`, `val V: Int = noisy()`, `fun helper(): Int = 1` and `fun main() { println(V); println(V + helper()) }` run under `after line 2: edit main.prism main.prism.v2`, where `helper` returns `2`, and `fun main()` is unchanged
- **THEN** stdout holds `0`, `5` and `7`

#### Scenario: An edited initializer runs again at the next read
- **WHEN** the same program runs under `after line 2: edit main.prism main.prism.v2`, where `main.prism.v2` declares `val V: Int = noisy() + 1`
- **THEN** stdout holds `0`, `5`, `0` and `7`

### Requirement: Replaced Top-Level Storage Stays Valid
Storage that a reload replaced MUST stay valid for the code and borrows that still refer to it. The allocation check MUST release what it owns, as it releases every top-level value.

#### Scenario: A replaced string value is released
- **WHEN** `val NAME: String = "a" + "b"` and `fun main() { println(NAME); println(NAME) }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where `NAME` is `"c" + "d"`
- **THEN** stdout holds `ab` and `cd`, and the session exits with status 0

### Requirement: Removed Declarations Keep Their Code
A reload that removes a declaration MUST leave the slots of its functions holding their last code, so that old frames and old function values calling them keep working. A declaration added again later with the same contract MUST get its slot back.

#### Scenario: A removed function still serves the running main
- **WHEN** `fun old(): Int = 1` and `fun main() { println(old()); println(old()) }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` removes `old` and declares `fun main() { println(2) }`
- **THEN** stdout holds `1` and `1`, and stderr holds `reload: applied main.prism`

### Requirement: A Reload With Errors Keeps the Previous Code
When a reload's files report errors, the session MUST print the diagnostics as `prism run` does and then `reload: not applied; the program keeps running its previous code`. It MUST leave the program and its own state as they were. This holds for errors of every stage: resolution, parsing, type and borrow checking, `E6001`, and `E6002` or `E6003` for the entry's `main`.

#### Scenario: A type error keeps the previous code
- **WHEN** `fun step(i: Int): Int = i` and `fun main() { println(step(1)); println(step(2)) }` run under `after line 1: edit main.prism main.prism.v2`, where `step`'s body becomes `true`
- **THEN** stdout holds `1` and `2`, stderr holds an `E4005` diagnostic and `reload: not applied; the program keeps running its previous code`, and the session exits with status 0

### Requirement: A Program That Does Not Compile at Start Waits
When the program does not compile as the session starts, the session MUST print its diagnostics and `watch: waiting for changes`, and wait for a change instead of exiting.

The session MUST watch every file it reached while trying, the file in error included, so that fixing any of them is seen.

#### Scenario: A program that does not compile at start waits for a change
- **WHEN** `fun main() { val x: Int = true }` runs under `after exit: edit main.prism main.prism.v2`, where `main.prism.v2` declares `fun main() { println(1) }`
- **THEN** stderr holds an `E4002` diagnostic and `watch: waiting for changes`, stdout holds `1`, and the session exits with status 0

#### Scenario: A syntax error in an imported file at start
- **WHEN** `prism run --watch app/main.prism` starts on `app/main.prism`, which imports `/lib/math`, while `lib/math.prism` declares `public fun twice(n: Int): Int = n *`, and `lib/math.prism` is then saved on disk with `n * 2`
- **THEN** stderr holds an `E2001` diagnostic and `watch: waiting for changes`, and then the program runs and prints `4`

### Requirement: A Newer Change Replaces a Pending Reload
When a file changes while a compiled reload has not been applied yet, the session MUST discard that reload and prepare a new one from the latest sources. The new reload is computed against the code the program is running.

#### Scenario: Two saves before the program calls anything
- **WHEN** a Rust test of the watch session holds the program before its next call, changes `value` to return `2` and then to return `3`, and then lets the program call `value`
- **THEN** the program reads `3`, and only one reload is applied

### Requirement: A Change After main Returns Runs It Again
When `main` returns, the session MUST run the end-of-run steps as `prism run` does, the allocation check included, print `watch: main returned with status <status>; waiting for changes`, and keep watching. The next change that compiles MUST print `watch: running main again` and run `main` in a fresh program. Every unit is linked again into new modules, top-level values are initialized again on their first read, and the old program's modules are freed.

#### Scenario: Editing after main returns
- **WHEN** `fun noisy(): Int { println(0); return 1 }`, `val V: Int = noisy()` and `fun main() { println(V) }` run under `after exit: edit main.prism main.prism.v2`, where `main` prints `V + 1`
- **THEN** stdout holds `0`, `1`, `0` and `2`, and stderr holds, in order, `watch: main returned with status 0; waiting for changes`, `watch: running main again` and the same `main returned` line

### Requirement: A Runtime Error Stops the Program, Not the Session
Under `--watch`, a runtime error MUST be reported as the `execution` spec requires, and MUST stop the program: no further code of it runs and no drop runs. The session MUST print `watch: the program stopped with a runtime error; waiting for changes` and keep watching. The next change that compiles MUST run `main` again in a fresh program.

#### Scenario: Fixing a division by zero
- **WHEN** `fun main() { val zero = 0; println(1 / zero) }` runs under `after exit: edit main.prism main.prism.v2`, where `main.prism.v2` prints `1 / 1`
- **THEN** stderr holds `panic: division by zero` with its location, `watch: the program stopped with a runtime error; waiting for changes` and `watch: running main again`, stdout holds `1`, and the session exits with status 0

### Requirement: Changes Are Detected From File-System Events
Without an edit script, the session MUST detect changes from the platform's file-system events on the directories that hold the program's files and its unresolved imports' targets, registered again after each reload. A file saved between a build's read and the registration MUST reload once it is done. After the first event it MUST wait for 10 ms without events, and at most 100 ms, before reloading. If events are unavailable, it MUST poll the files every 100 ms and say so on stderr.

#### Scenario: An atomic save is seen
- **WHEN** an editor saves `main.prism` by writing `main.prism.tmp` and renaming it to `main.prism` while the program runs
- **THEN** the session reloads the program once

### Requirement: A Removed Workspace Root Ends the Session
Without an edit script, when the workspace root of the program, or the entry file's directory before any graph was built, no longer exists, removed or renamed away, the session MUST print `watch: the workspace root <path> was removed; the session ends`, with the absolute path, and exit with status 1, ending a program that still runs. It MUST not report the program's files as missing first.

#### Scenario: Removing the workspace root ends the session
- **WHEN** `prism run --watch main.prism` runs `fun value(): Int = 1` and `fun main() { var n = 0; while (value() == 1) { n += 1 }; println(value()) }`, and the directory holding `main.prism` is then removed
- **THEN** stderr holds `watch: the workspace root <path> was removed; the session ends` and no `reload:` line, stdout is empty, and the process exits with status 1

### Requirement: Unchanged Content Does Not Reload
A file-system event for a file whose content is the same as when it was last read MUST not cause a reload. A file whose last content did not parse MUST count as changed at its next event, whatever it then holds.

#### Scenario: Saving without changes does nothing
- **WHEN** `prism run --watch main.prism` runs a program whose `main` loops calling a function, and `main.prism` is written again with the same content
- **THEN** stderr holds no `reload:` line

#### Scenario: Changing a file back after a syntax error
- **WHEN** a Rust test of the watch session runs `fun main() { println(1) }` until `main` returns, then edits `main.prism` to `fun main() { println(1 }`, and then edits it back to `fun main() { println(1) }`
- **THEN** the first edit does not compile, the second runs `main` again, and it prints `1`

### Requirement: Edit Scripts
`prism run --watch --edits <script>` MUST replace file watching with the script's steps, one per line, ignoring blank lines and lines starting with `#`. A step is `<trigger>: edit <target> <replacement>`, both paths relative to the script's directory. It makes the session read `<target>` as the content of `<replacement>` without changing any file on disk, and a missing target becomes a new file. Another line, or `--edits` without `--watch`, MUST be a usage error with status 1.

#### Scenario: Edits at an output line
- **WHEN** `fun step(i: Int): Int = i` and `fun main() { var i = 0; while (i < 4) { println(step(i)); i += 1 } }` run under `after line 2: edit main.prism main.prism.v2`, where `step` returns `i * 10`
- **THEN** stdout holds exactly `0`, `1`, `20` and `30`, every time the session runs, and `main.prism` on disk is unchanged

### Requirement: Edit Triggers
The trigger `after line <n>` MUST stop the program when it writes its `n`-th newline in the current run. The session then applies the step's edits and runs the reload, and the program applies the reload at that point, if it compiled, and continues. The trigger `after exit` MUST apply its edits once no program is running. Consecutive steps with the same trigger MUST form one reload.

#### Scenario: Two files edited in one reload
- **WHEN** the program of "A body edit compiles one file" runs under `after line 1: edit lib/math.prism lib/math.prism.v2` and `after line 1: edit app/main.prism app/main.prism.v2`
- **THEN** stderr holds one `reload: applied lib/math.prism, app/main.prism` line

### Requirement: The End of an Edit Script
The session MUST end when no step is left and no program is running. Its exit status MUST be that of the last run of `main`, or 1 when the last reload came after that run and did not compile. A step whose line is never written MUST end the session with `watch: the edit script's step "after line <n>" was never reached` and status 1.

#### Scenario: A step that is never reached
- **WHEN** `fun main() { println(1) }` runs under `after line 5: edit main.prism main.prism.v2`
- **THEN** stderr ends with `watch: the edit script's step "after line 5" was never reached`, and the session exits with status 1

### Requirement: Frame Loops Can Defer Reloads to Reload Points
The runtime MUST let a frame loop defer reloads to reload points it calls itself. Once a loop asks for this, reloads MUST not be applied at function entries. The session MUST wake the loop instead, and the reload MUST be applied when the loop next calls the reload point, which MUST report which modules the applied reload compiled.

#### Scenario: A deferred reload is applied at the reload point
- **WHEN** a Rust test runs a program whose loop calls the runtime's reload point between iterations, defers reloads to it, and requests a reload while an iteration runs
- **THEN** the iteration finishes on the old code, the reload is applied at the next reload point, which reports the reloaded module, and the next iteration runs the new code

### Requirement: Reload Latency
Reloading a one-line body edit of one file in a 100-file program SHOULD take under 50 ms on a release build, as a median from the write on disk to the program running the new code, debounce included. A benchmark MUST measure and report it. With `--watch`, `--timings` MUST print, after each reload, the time of its stages and the number of live modules.

#### Scenario: Timing a reload
- **WHEN** `prism run --watch --timings --edits edits main.prism` applies one reload
- **THEN** stderr holds one line starting with `timing: reload` giving the reload's total and per-stage times in milliseconds, and one line giving the number of live modules

### Requirement: Edit Steps Wait for the Reload Point of a Frame Loop
While a frame loop defers reloads to its reload points, an `after line <n>` step MUST still stop the program at its newline and let the session prepare the reload, but the reload MUST be applied at the loop's next reload point rather than at that output call.

#### Scenario: A line printed during a pass
- **WHEN** a headless counter app whose `Counter` prints `composing` runs under `--watch` with the edit script `after line 1: edit main.prism main.prism.v2`, which changes the label, and the event script `click "+"` and `dump`
- **THEN** the pass that printed `composing` finishes on the old code, and the dump holds the new label with the count `1`

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

## Design Rationale

Decisions from `hot-reload` (planned autonomously on 2026-10-17, implemented and reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`). How slots, stack checks and top-level storage work is in the Design Rationale of `code-generation`.

- **Plain programs first, compose next** (D1). This capability reloads ordinary programs, so the mechanism is tested with `println` programs; `compose-hot-reload` adds the policy for composables (recomposing reloaded scopes, resetting slot-table state whose type fingerprint changed) on top of the same reload points, fingerprints and closure contracts. No language change: no syntax, built-in or error code.
- **One process, the program on its own thread** (D2). The session runs in the `prism` process and the program on the thread `prism run` gives it, talking through channels (`LineWritten`, `MainReturned`, `Stopped`). A child process would need shared executable memory and an IPC protocol to patch; restarting on every change keeps no state. `desktop-window` moves the session off the main thread; nothing depends on which thread it is.
- **Applied at the next call, on the program thread** (D3). The session compiles and finalizes a reload while the program runs, then interrupts it through the stack limit; the program thread applies the whole reload at its next function entry, under the pending reload's lock, so a reload is never half applied and `withdraw` returning nothing means it was applied completely. A frame already running keeps its code (no on-stack replacement), so a running `main` loop keeps its own body; the call that takes the interrupt still runs the code it loaded, and the next one runs the new code. A loop without calls is not patched until it calls something, which it could not observe anyway.
- **Reload points for frame loops** (D4). `reload::defer_to_reload_points(wake)` makes requests wake the loop instead of interrupting it, and `reload::reload_point()` applies a pending reload between frames and reports the units it defined; tested from Rust, for `compose-hot-reload`.
- **Contracts instead of refusals or migration** (D5, D7). A changed signature, type or capture list gives the symbol a new slot; values of an old layout stay with the old code and old glue that understand them, and new code never receives them. The split is noted on stderr, since old code may keep running for a while. A closure's slot is keyed by its index within its parent, so inserting a closure before another sends an old value to the body now at its position when the contracts match: memory-safe, and the same positional matching as compose's group keys. Rejected: refusing layout changes as JVM HotSwap does (the most common edits), migrating values field by field as Dart does (needs layout metadata and surprises borrows).
- **Top-level values are kept unless their declaration changed** (D8). The value key covers the symbol, the type fingerprint and a span-free hash of the declared type and initializer, so editing a function the initializer calls, moving or reformatting the declaration keeps the value, and editing the initializer resets it at its next read. Rejected: always keeping values (an edited initializer would have no effect until a restart); resetting a changed file's values; re-running initializers eagerly in the middle of the program's call.
- **A reload does work proportional to the change** (D9, D14). The session keeps each file's content hash, syntax tree, check key, summary and MIR in memory, reads changed files through its store, runs Phase 1 over the program, Phase 2 for files whose check key changed, and compiles the units whose code key is not in the running plan, reading and writing the build cache as `prism run` does, so a reload's code is reused by the next run. Per-function granularity was rejected: a file is the unit of hermetic compilation and compiles in about a millisecond.
- **Errors keep the previous code** (D12). A reload with errors of any stage prints them and leaves the program and the session's state as the last applied reload left them; a newer change withdraws a prepared reload the program has not applied and frees its module. Preparing a reload allocates fresh slots and storage but maps nothing; the program thread commits the mappings with the slot writes (D21), so a withdrawn reload leaves both tables exactly as the running code left them. Applying the files that compile was rejected: importers were checked against their imports' new interfaces.
- **After `main` returns, a fresh program** (D13). The next change that compiles links every unit into new modules, mostly from the cache, so a re-run behaves like a new `prism run` and its allocation check covers one run. Re-running `main` in the patched program was rejected: values would carry over between runs.
- **File watching** (D15). `notify` 8.2.0 in non-recursive mode on the directories of the program's files and its unresolved imports' targets, registered again after each reload, with a 10 ms quiet period bounded at 100 ms and content hashes, falling back to 100 ms polling. Directories rather than files, so editors that save by renaming are seen. The files a failed resolution reached, the one in error included, are watched too, and a file that leaves the program is read again when it comes back. Rejected: polling first; `notify-debouncer-full` for a 30-line debounce; a recursive watch of the workspace.
- **Edit scripts make reloads deterministic** (D16). `--edits` replaces the watcher with `after line <n>` and `after exit` steps over a virtual overlay, so fixtures stay read-only; the program blocks in the output layer at the step's line, which is set before the program can pass it. Rejected: writing files and sleeping in tests; a test-only built-in. Watch fixtures run cold and warm, which also checks that reloads write and reuse cache entries.
- **A latency target** (D17). A median under 50 ms from the write on disk to `reload: applied` for a one-line body edit in a generated 100-file program, release build, measured by the ignored `reload_timing.rs`. Measured on 2026-10-18 (Apple silicon): a median of 31.0 ms, of which 12.5 ms detection with the debounce, 6.1 ms front end, 0.8 ms code generation, 0.03 ms linking, 0.002 ms application, and the rest FSEvents' delivery.
- **The status line names what a reload defines** (D20). `reload: applied <files>` lists the files whose units a reload defines, compiled or loaded, so it reads the same on a warm cache as on a cold one.
- **A removed workspace root ends the session** (plain fix, 2026-10-19). Orphaned `prism run --watch` processes of failed tests kept running their looping programs for hours in directories nobody watched any more. With its root gone, removed or renamed away, no file of the program can change and the watcher's directories are gone, so the session says so and exits with status 1 instead of waiting forever. It checks the root before each wait, since a root removed before the watcher registered has no event, and matches an event on a watched directory itself to the files in it. A session ending while its program runs keeps the program's code mapped until the process exits. Rejected: waiting for the root to come back (the watches are on directories that no longer exist); reporting the files as missing and waiting (the reported cause).
- **A separate capability** (D18). The session gets this spec; `code-generation` changes where slots and storage live and what a stack check does, and `execution` lists the options. Reload failures keep their existing error codes.

Decisions from `compose-hot-reload` (planned autonomously on 2026-10-18, implemented on 2026-10-19, and logged in `docs/decisions-for-review.md`), on the mechanism this capability gives compositions; what they recompose is in the `compose` and `ui` Design Rationales.

- **A restart function's contract covers its composable's** (D8). `slot_contract` hashes a `BodyKind::Restart` body with its composable's contract, so changing the composable's parameter types splits `F#restart` with `F` and no old restart function is repointed to new code that would read the old scope's kept slots by its new parameter list. Restart and root-content splits get no note of their own, and `PreparedReload::retired_slots` reports the old slots of every split for the runtime. Cost: one more hash per restart body.
- **Reload points and the next call** (D2, D12). `apply_at_next_call` raises the stack-limit interrupt when a reload is pending, so a reload requested during the last frame of an app is applied at the program's next call rather than at the next `runApp`. Under reload points, `apply_now` leaves the reload pending, so an `after line` step during a pass never applies it mid-pass, and `reload_point` reads an atomic requested flag before taking the pending reload's lock. A scripted-edit hook, `set_edit_hook`, lets an app's event script send edits to the session as `SessionEvent::ScriptedEdit`.
- **`AppliedReload` names what compositions need** (D4, D5). Besides its units and message, it carries the body keys of the composable bodies the reload affects, computed by the session from the import graph reversed and the kept summaries (and the generic of each instance unit), and the retired slots.
