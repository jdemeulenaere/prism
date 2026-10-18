# Execution Specification Delta

## ADDED Requirements

### Requirement: Watch Options
`--watch`, before the file, MUST run the program in a watch session that reloads it when its files change (`hot-reload`), passing the same arguments to every run of `main`. `--edits <script>` MUST drive that session's reloads from a script, and without `--watch` is a usage error with status 1. Under `--watch`, `--timings` MUST also print the stages of each reload, `--no-cache` keeps the whole session out of the cache, and `--explain-cache` explains its first load.

#### Scenario: Edits without watching
- **WHEN** `prism run --edits edits main.prism` runs
- **THEN** it prints the usage of `prism run` to stderr, runs nothing and exits with status 1

### Requirement: A Runtime Error Under Watch Ends Only the Run
Under `prism run --watch`, a runtime error MUST be reported as "Runtime Errors End the Program" states, and MUST end the program's run instead of the process: no further code of the run and no drop runs, and the watch session continues (`hot-reload`).

#### Scenario: A runtime error under watch
- **WHEN** `prism run --watch --edits edits app/main.prism` runs the program of "Runtime error location", with an edit script whose only step is `after exit` and fixes `zero` to return `1`
- **THEN** stderr holds `panic: division by zero` and `  at app/main.prism:3:33`, then `watch: the program stopped with a runtime error; waiting for changes`, and the program runs again
