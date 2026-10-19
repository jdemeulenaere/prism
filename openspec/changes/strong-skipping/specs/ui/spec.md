## ADDED Requirements

### Requirement: Dumping Recompositions
An event script's `dump recompositions` MUST print `recompositions`, then one line per composable function of the program's own files, not of the bundled library, whose body ran or was skipped since the run started or the previous `dump recompositions`: two spaces, its declaration path as `prism dump-mir` names its body, ` ran <n> skipped <m>`, sorted by path. Composable lambdas MUST NOT be counted. Without this command in the script, nothing MUST be counted.

#### Scenario: Counting a click's recompositions
- **WHEN** a headless app composes `compose fun Badge(n: Int) { Text("$n") }` and `compose fun Screen() { val c = remember { mutableStateOf(0) }; Column { Text("${c.value}"); Badge(1); Button(onClick = { c.value += 1 }) { Text("+") } } }` through `runApp { Screen() }`, and its script is `dump recompositions`, `click "+"` and `dump recompositions`
- **THEN** stdout is the lines `recompositions`, `  main::Badge ran 1 skipped 0`, `  main::Screen ran 1 skipped 0`, `recompositions`, `  main::Badge ran 0 skipped 1` and `  main::Screen ran 1 skipped 0`

#### Scenario: A misspelled dump kind
- **WHEN** `prism run --headless --events script main.prism` runs an app whose `script` holds `dump recomposition`
- **THEN** stderr is `Error: script:1: expected 'tree', 'draw' or 'recompositions' after 'dump'`, the exit status is 1, and the program does not run

### Requirement: Pausing an Event Script
An event script's `pause <milliseconds>` MUST sleep the program thread for that many milliseconds before the next command. A pause MUST count as waiting: its time MUST be part of no frame.

#### Scenario: Paced clicks
- **WHEN** a test of `run_cli_tests.rs` runs the counter screen headless with `--frame-stats` and the script `click "+"`, `frame`, `pause 50`, `click "+"` and `frame`
- **THEN** the run takes at least 50 ms longer than the same script without the pause, the report's first line starts with `frame stats: frames 2, events 4, passes 3, `, and its frame maximum is under 50 ms

#### Scenario: A pause without a duration
- **WHEN** `prism run --headless --events script main.prism` runs an app whose `script` holds `pause`
- **THEN** stderr is `Error: script:1: expected a number of milliseconds after 'pause'`, the exit status is 1, and the program does not run

## MODIFIED Requirements

### Requirement: Headless Runs Follow an Event Script
A headless run MUST read its event script's commands in order and end the app after the last one: `click`, `press` and `release` at a text target, at a kind target ("Script Targets by Kind") or at `<x> <y>`, `type "<text>"`, `key backspace`, `key enter`, `resize <width> <height>`, `frame`, `pause <milliseconds>`, `dump`, `dump tree`, `dump draw` and `dump recompositions`, and, under `--watch`, `edit <target> <replacement>` and `wait reload`. Blank lines and `#` comments MUST be ignored.

#### Scenario: Scripted counter session
- **WHEN** the counter program runs headless with the script `# two clicks`, `click "+"`, `click "+"` and `dump`
- **THEN** stdout holds a tree whose text node reads `Count: 2`, and the run passes the allocation check

#### Scenario: Resizing the window
- **WHEN** a headless app composes `Box(modifier = Modifier.fillMaxSize()) { }` and its script is `resize 200 100` then `dump`
- **THEN** stdout is the lines `window 200x100` and `  box #1 0,0 200x100 fillMaxSize`
