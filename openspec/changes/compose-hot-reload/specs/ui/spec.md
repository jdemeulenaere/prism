# UI Specification Delta

## Purpose
The UI toolkit turns compositions into user interfaces. This change adds hot reload to the capability that `ui-toolkit` creates: where `runApp`'s frame loop applies reloads, what becomes of handlers, layout and drawing after a reload, errors in reloaded code, the `edit` and `wait reload` script commands, and the reload latency target.

## ADDED Requirements

### Requirement: The Frame Loop Applies Reloads Between Events
While `runApp` runs, a reload MUST be applied only at the top of an iteration of its frame loop: after the handlers of the previous event have returned and before the loop recomposes, never during a pass, a handler, layout or drawing. The pass that follows MUST recompose the scopes the reload invalidated.

#### Scenario: A reload waits for the running handler
- **WHEN** a Rust test of `prism-ui` requests a reload from inside a click handler of a running app
- **THEN** the handler finishes in its old code, the reload is applied before the next pass, and that pass runs the new code

### Requirement: A Pending Reload Wakes an Idle App
A reload that becomes pending while the app waits for events MUST wake the frame loop, which then applies it, recomposes and presents a frame without any input. When `runApp` returns, reloads MUST again be applied at the program's next call, a reload still pending included.

#### Scenario: Saving a file under a waiting headless app
- **WHEN** a headless counter app runs under `prism run --watch --events` with the script `wait reload` then `dump`, and a test saves `main.prism` on disk with the label `Clicks:`
- **THEN** stdout holds a dump with a text node reading `Clicks: 0`

### Requirement: Handlers Written by Reloaded Code Are Replaced Before the Next Event
A handler whose closure's captures did not change MUST run its new body at its next call. A handler whose captures changed MUST keep its old code until its node is given a new handler; the pass after the reload MUST give a new handler to every node whose handler an affected composable body writes, before the next event is dispatched.

#### Scenario: A handler whose captures changed
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"` three times, an edit that declares `val step = 2` in `Counter` and makes the handler `{ c.value += step }`, then `click "+"` and `dump`
- **THEN** the dump holds the text node reading `Count: 5`, and stderr notes that the handler's captures changed

#### Scenario: A handler body edit reaches the next click
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"`, an edit of the handler to `{ c.value += 10 }`, `click "+"` and `dump`
- **THEN** the dump holds the text node reading `Count: 11`

### Requirement: Layout and Drawing After a Reload
After the pass that follows a reload, the frame loop MUST lay out and build the display list as after any pass: only nodes whose properties changed are marked dirty, matched nodes keep their numbers, focus and measurements, and no frame is presented when nothing visible changed.

#### Scenario: A focused field keeps its focus and text across a reload
- **WHEN** the typing app, run headless under `--watch`, gets the script `click 100 18`, `type "ab"`, an edit of `Text("Hello ${name.value}")` to `Text("Hi ${name.value}")`, then `type "c"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "abc" focused` and `text #3 0,36 48x20 "Hi abc"`: both nodes kept their numbers, and the field its focus

### Requirement: A Reload With Errors Keeps the Running UI
When a reload does not compile, the app MUST keep running its previous code, its composition and its nodes unchanged, and keep handling events, while the session prints the diagnostics and `reload: not applied; the program keeps running its previous code` on stderr.

#### Scenario: A type error in a composable
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"`, an edit whose `Text` call passes an `Int`, `click "+"` and `dump`
- **THEN** stderr holds a type error and `reload: not applied; the program keeps running its previous code`, and the dump holds the text node reading `Count: 2`

### Requirement: A Runtime Error in Reloaded Code Ends the Program
A runtime error raised by reloaded code, during the pass after the reload or later, MUST end the program as every runtime error does. Under `--watch`, that stops the program and not the session.

#### Scenario: Reloaded code divides by zero
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"`, then an edit whose `Counter` body prints `10 / (c.value - 1)`, then `dump`
- **THEN** stderr holds `panic: division by zero` with its location and `watch: the program stopped with a runtime error; waiting for changes`, no dump is printed, and the session exits with status 101

### Requirement: Event Scripts Can Edit Files Under Watch
Under `prism run --watch`, the event script command `edit <target> <replacement>` MUST make the session read `<target>` as the content of `<replacement>`, both relative to the script's directory, as an edit script's `edit` does. Consecutive `edit` commands MUST form one reload, applied before the next command runs. Without `--watch`, `edit` MUST be a script error with status 1.

#### Scenario: Edit without watch
- **WHEN** `prism run --headless --events events main.prism` runs a script whose first line is `edit main.prism main.prism.v2`
- **THEN** stderr is `Error: events:1: the edit command needs --watch` and a newline, nothing runs, and the exit status is 1

#### Scenario: Two edits form one reload
- **WHEN** a headless app run under `--watch` gets the script `edit lib/labels.prism lib/labels.prism.v2`, `edit app/main.prism app/main.prism.v2` and `dump`
- **THEN** stderr holds one line `reload: applied lib/labels.prism, app/main.prism`

### Requirement: Event Scripts Can Wait for a Reload
Under `prism run --watch`, the event script command `wait reload` MUST block the frame loop until a reload from the session's file watcher is applied, and then continue. When none is applied within 10 seconds, it MUST end the program with `event script line <n>: no reload arrived within 10 seconds`. Without `--watch`, it MUST be a script error with status 1.

#### Scenario: Waiting for a reload that never comes
- **WHEN** a Rust test of `prism-ui` runs a headless app whose script is `wait reload`, with the wait shortened to 10 milliseconds and no reload requested
- **THEN** the runtime error `event script line 1: no reload arrived within 10 seconds` ends the test's child process with status 101

### Requirement: UI Reload Latency
On a release build, saving a one-line edit of a composable body of the counter app SHOULD reach the frame that shows it in a median under 50 ms, debounce included, measured from the write on disk to the display list of that frame. A benchmark MUST measure and report it. With `--watch`, `--timings` MUST print `timing: reload pass <ms> (<n> scopes)` after the pass of each reload.

#### Scenario: Timing a UI reload
- **WHEN** `prism run --watch --timings --headless --events events main.prism` runs the counter app with an `edit` command
- **THEN** stderr holds a `timing: reload` line and then a line starting with `timing: reload pass` that gives the pass's time in milliseconds and the number of scopes it recomposed
