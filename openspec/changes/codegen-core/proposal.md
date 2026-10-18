# Proposal: codegen-core

## Why

Prism checks programs but cannot run one: the elaborated MIR that the borrow checker produces has no consumer. This change, step 14 of "Then: the road to Milestone 2" in `ROADMAP.md` and the first of Phase 3, adds the Tier 1 backend (`docs/architecture.md` §2): a Cranelift JIT that compiles a workspace's files in memory and runs `main`, with no external toolchain. It covers scalar programs only. Its structure is what `hot-reload`, `codegen-aggregates`, `runtime-strings-and-drops`, `monomorphization`, `closures` and the compose runtime build on: per-file code units, symbols derived from declaration paths, calls through a patchable function table, a Rust runtime library and an executable-fixture runner. It also settles the runtime questions the earlier changes left to the backend: integer overflow and division by zero, how a runtime error is reported, and how a program starts and exits.

## What Changes

- **A `prism-codegen` crate** lowers each file's elaborated MIR to Cranelift IR, one code unit per file, from that file's MIR alone. Supported: `Int` (64-bit), `Double` (`Float`), `Bool`, `Char` and `Unit` values; arithmetic, comparison and logical operators; locals and assignments; functions, extensions and default values on scalars, called within a file and across files; `if`, `when` over scalars, `while`, `do`/`while`, `for` over ranges, `break` and `continue`; top-level `val`s; references to scalars (`&Int`, `&mut Int`), which top-level `val`s and default values already use in MIR; and the built-in `clone()` of a scalar.
- **Everything else is rejected before code generation** with a new diagnostic, `E6001` (`UnsupportedByBackend`), at the first unsupported construct of each body, naming the construct and the change that will add it. Structs, enums, nullables, `Heap<T>`, strings, drops, closures and function values, generic calls and composable functions are rejected; generic and closure bodies are not compiled at all.
- **Calls go through a function table.** Every call of a top-level function, extension, default generator or top-level `val` accessor loads its target from a slot of a process-wide table that the linker fills, including calls within one file. Nothing is inlined across functions. `hot-reload` will patch these slots.
- **A `prism-runtime` crate**, linked into the `prism` binary, provides the `extern "C"` functions that generated code calls: output, the `Double` remainder, and panics. Generated code finds them by name through the JIT's symbol table.
- **`prism run <file.prism> [-- <arguments>...]`** compiles the entry file and every file it transitively imports, then calls the entry file's `fun main()`. The exit status is `0` when `main` returns, `101` after a runtime error, and `1` when the program does not compile. `--timings` prints the time spent in each stage before `main` runs. `main` must be a non-generic, non-composable top-level function of the entry file with no parameters returning `Unit`: a missing `main` is `E6002` (`MissingMain`) and an invalid one `E6003` (`InvalidMain`).
- **Output built-ins `print(x)` and `println(x)`, and `println()`.** They accept an `Int`, `Double`, `Bool`, `Char` or `String` (a reference to one also), resolve only when no local, top-level declaration or import of that name exists, and cannot be used as values. `String` arguments typecheck now and are rejected by the backend (`E6001`) until `runtime-strings-and-drops`. Output is buffered and flushed when the program ends, before a runtime error is reported, and at each newline when stdout is a terminal.
- **Runtime semantics.** `Int` is 64-bit two's complement, and `+`, `-`, `*`, unary `-` and `/` stop the program with a runtime error when the result does not fit (overflow traps, as in Swift and in Rust debug builds); `/` and `%` by zero are runtime errors; `Int.MIN % -1` is `0`. `Double` follows IEEE 754 and never traps. A runtime error prints `panic: <message>` and the source location to stderr and exits with status `101`, without running drops. A top-level `val` is initialized on its first read, once; reading it during its own initialization is a runtime error. Loops, ranges and `when` evaluate as `docs/architecture.md` §4 planned.
- **Executable fixtures.** A fixture may add `// run: <status>` to its header: the runner then runs `prism run` on its entry and compares the exit status, and stdout and stderr with `stdout.snap` and `stderr.snap` (empty when absent), blessed with `PRISM_BLESS=1`. `prism dump-clif <file.prism>` prints a file's Cranelift IR for debugging.
- **Startup target.** A timing test reports how long `prism run` takes to reach `main` on a reference program (`examples/run/`), against the 15 ms target, without failing CI on it.
- Not in scope: aggregates, nullables, references to non-scalars, strings, drop glue, closures, generics, composable functions (their changes are named above); caching object code, so `prism run` does not use the build cache; Tier 2 (`prism build --release`); stack-overflow reporting; reading program arguments, which needs strings and lists; adapters for representation-changing subtyping (`ROADMAP.md`, from `typed-tree`), which only arise for function, nullable and `Heap<T>` types.

## Capabilities

### New Capabilities
- `code-generation`: the Tier 1 backend: which MIR it compiles and how it rejects the rest (`E6001`), per-file code units, symbol names, calls through the function table, the representation of scalar values and the calling convention, top-level `val` storage, and `prism dump-clif`.
- `execution`: running a program: `prism run` and its exit statuses, the entry point (`E6002`, `E6003`), what the output built-ins print, runtime errors, `Int` and `Double` arithmetic, top-level `val` initialization, the order in which `when` branches run, and the startup time target.

### Modified Capabilities
- `loops`: adds the runtime meaning of loops, ranges, `break` and `continue` ("Loop and Range Evaluation Semantics", from `docs/architecture.md` §4).
- `type-system`: adds the typing and scoping of the output built-ins.
- `typed-tree`: "Built-In Operations Are Identified" adds calls of the output built-ins.
- `mir`: "Calls Name Their Callee" adds the output built-ins, which take no receiver.
- `compilation-and-abi`: adds the Phase 3 step: code generation from each file's elaborated MIR alone, independent of other files' bodies.
- `diagnostics`: "Stable Diagnostic Codes" adds the `E6xxx` range for code generation and execution.

## Impact

- New crates `crates/prism-codegen` (depending on `prism-mir`, `prism-typed-tree`, `prism-ast`, `prism-runtime` and `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-jit` and `cranelift-native`, all pinned to `=0.136.2`) and `crates/prism-runtime` (no dependencies), added to the workspace.
- `crates/prism-typechecker`: resolves `print` and `println` as built-ins in `infer_call_expr` and checks their arguments. `crates/prism-typed-tree`: `BuiltinFunction::Print` and `BuiltinFunction::PrintLine`. `crates/prism-mir`: `Callee::has_receiver` is true only for `clone`, and the dump prints the new built-ins. `crates/prism-borrowck`: the new built-ins in `elision.rs`.
- `crates/prism-cli`: a `run` module and the `run` and `dump-clif` commands, `WorkspaceError::Codegen`, rendering of `E6001` to `E6003`, a shared Phase 2 step returning elaborated MIR, the run mode of `tests/fixture_tests.rs`, a timing test, and fixtures under `tests/fixtures/execution/`, `tests/fixtures/code-generation/` and `tests/fixtures/loops/`.
- `examples/run/`: a small multi-file scalar program, also the timing test's reference program.
- `openspec/specs/diagnostics/spec.md`: a new `E6xxx` catalog section with `E6001` to `E6003`.
- `docs/architecture.md`: on archive, the "Single File as Compilation Unit" requirement (§1), the "Tier 1 instant script execution" scenario (§2), "Loop and Range Evaluation Semantics" and the `when` scenario of "Expression Evaluation Semantics" (§4) move into the specs. The hot-reload and Tier 2 scenarios, safe navigation and the other §4 requirements stay.
- `ROADMAP.md`: on archive, the follow-ups on overflow (from `mir-lowering`) and on runtime arguments, layouts of transitively imported types, object-code caching and the target triple in cache keys are updated as design.md says.
