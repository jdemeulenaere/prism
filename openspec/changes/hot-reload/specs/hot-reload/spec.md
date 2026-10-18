# Hot Reload Specification Delta

## Purpose

Defines `prism run --watch`: a watch session that keeps a program running while its source files change. It covers how changes are detected, what a reload compiles, where and when a reload is applied, which code old frames and old function values call after it (slot contracts), what happens to top-level values, how long old code stays mapped, what errors and runtime errors do to the session, how `main` runs again after it returns, how scripted edits drive reloads deterministically, and the reload latency target. What a single run of a program does is in `execution`; the function table and the code it holds are in `code-generation`.

## ADDED Requirements

### Requirement: Watching a Running Program
`prism run --watch <file.prism>` MUST compile and run the program as `prism run` does, and keep the process running after `main` returns. It MUST watch every file of the program and the target path of every unresolved import. When one of them changes, it MUST reload the program, patching the running program without restarting it. The session MUST end only when interrupted, or when its edit script ends.

#### Scenario: Saving an edited body while the program runs
- **WHEN** `prism run --watch main.prism` runs `fun value(): Int = 1` and `fun main() { var n = 0; while (value() == 1) { n += 1 }; println(value()) }`, and `main.prism` is then saved on disk with `fun value(): Int = 2`
- **THEN** the program prints `2` without being restarted, and stderr holds `reload: applied main.prism` and then `watch: main returned with status 0; waiting for changes`

### Requirement: Session Status Lines
The program's output MUST go to stdout, and the session's status lines to stderr, each starting with `reload:` or `watch:`. An applied reload MUST print `reload: applied <files>`. `<files>` lists the files whose code was compiled, relative to the workspace root, in topological order, separated by `, `. When instances were compiled, the list MUST be followed by ` (<n> instance)` or ` (<n> instances)`.

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
Each slot of the function table MUST record the contract of the code it holds: the type fingerprints of its parameter and result types and, for a closure's code, its environment's captures with their modes and type fingerprints. A type fingerprint MUST be a structural hash of the type. It covers the names, markers, fields, variants and payloads of every struct and enum the type reaches, with type arguments substituted.

#### Scenario: A changed field type keeps old values with old code
- **WHEN** `struct P { val x: Int }`, `fun show(p: &P) { println(p.x) }` and `fun main() { val p = P(1); show(p); show(p) }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `struct P { val x: Double }`, `fun show(p: &P) { println(p.x + 0.5) }` and `fun main() { val p = P(1.0); show(p); show(p) }`
- **THEN** stdout holds `1` and `1`, and stderr notes that the signature of `main::show` changed

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
- **THEN** stdout holds `1` and `2`, stderr holds an `E4002` diagnostic and `reload: not applied; the program keeps running its previous code`, and the session exits with status 0

### Requirement: A Program That Does Not Compile at Start Waits
When the program does not compile as the session starts, the session MUST print its diagnostics and `watch: waiting for changes`, and wait for a change instead of exiting.

#### Scenario: A program that does not compile at start waits for a change
- **WHEN** `fun main() { val x: Int = true }` runs under `after exit: edit main.prism main.prism.v2`, where `main.prism.v2` declares `fun main() { println(1) }`
- **THEN** stderr holds an `E4002` diagnostic and `watch: waiting for changes`, stdout holds `1`, and the session exits with status 0

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
Without an edit script, the session MUST detect changes from the platform's file-system events on the directories that hold the program's files and its unresolved imports' targets, registered again after each reload. After the first event it MUST wait for 10 ms without events, and at most 100 ms, before reloading. If events are unavailable, it MUST poll the files every 100 ms and say so on stderr.

#### Scenario: An atomic save is seen
- **WHEN** an editor saves `main.prism` by writing `main.prism.tmp` and renaming it to `main.prism` while the program runs
- **THEN** the session reloads the program once

### Requirement: Unchanged Content Does Not Reload
A file-system event for a file whose content is the same as when it was last read MUST not cause a reload.

#### Scenario: Saving without changes does nothing
- **WHEN** `prism run --watch main.prism` runs a program whose `main` loops calling a function, and `main.prism` is written again with the same content
- **THEN** stderr holds no `reload:` line

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
