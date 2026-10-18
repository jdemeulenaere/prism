# Design: codegen-core

## Context

See `proposal.md` for motivation. This is step 14 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17. The orchestrator fixed the scope (scalars, functions and calls across files, control flow, loops, ranges, top-level `val`s, a minimal output built-in, an executable-fixture runner) and the later changes that own the rest. Decisions marked **(autonomous)** were made by the proposer and are logged in `docs/decisions-for-review.md`.

Current state:

- **The pipeline ends at elaborated MIR.** `crates/prism-cli/src/workspace.rs` runs Phase 1 over the import graph (`extract_abis`), then Phase 2 per file in topological SCC order (`check_phase_two`: `typecheck_file`, `prism_mir::lower_file`, `prism_borrowck::check_file`), and uses the elaborated MIR only for instance requests. With the build cache, a file whose verdict is cached skips Phase 2 and has no MIR at all.
- **MIR is self-describing** (the `mir` spec, D10 of `mir-lowering`). Calls name their callee by `DeclarationPath` and repeat the parameter types; the destination place gives the result type; each `MirFile` carries its type table. A backend needs neither the ABI nor the type checker. Bodies are owned by a function, a top-level `val`, a default generator (`BodyOwner::Default`) or a closure; `Body::type_parameters` marks generic bodies.
- **Scalar programs already use references and a struct in MIR.** A top-level `val` read is `_t = const &main::LIMIT; copy (*_t)` (D5 of `mir-lowering`). A default generator takes `&T` borrows of the earlier arguments, which the caller makes with `_k = &_j (default)`. A `for` loop builds the built-in `Range` (`struct <builtin>::Range(start, end, endInclusive)`) into a hidden local and reads `_2.start` and `_2.end`; its latch compares with the last element before incrementing, so it never overflows.
- **There is no output built-in.** `BuiltinFunction` (`crates/prism-typed-tree/src/tree.rs`) has only `Clone`; many fixtures declare their own `fun print(n: &Int) {}`. `Callee::has_receiver` (`crates/prism-mir/src/mir.rs`) answers true for every built-in. `Double` and `Float` are both `Type::Float`, a 64-bit float; `Int` is `i64` (`Constant::Int`); `Char` is a Unicode scalar.
- **No Cranelift dependency, no runtime crate.** The workspace (`Cargo.toml`) has ten crates. `cranelift-jit` dropped its "hotswap" mode years ago, so patchable calls are ours to build.
- **The fixture runner** (`crates/prism-cli/tests/fixture_tests.rs`) runs `prism check` in process and compares codes and `*.snap` dumps, blessed with `PRISM_BLESS=1`. Other CLI tests spawn `env!("CARGO_BIN_EXE_prism")`.

## Goals / Non-Goals

**Goals:**
- Run scalar programs end to end, with every runtime decision the later backend changes inherit made once: representation, calling convention, symbols, the function table, runtime errors, output and program start.
- A structure `hot-reload` can patch without redesign: per-file code units, symbolic cross-file references, every call through a slot.
- Reject every unsupported program with a diagnostic, never a compiler panic or a miscompilation.
- Every new scenario has an executable fixture.

**Non-Goals:**
- Optimization beyond what Cranelift does by itself; parallel compilation of files (the per-file units allow it later).
- Caching generated code, Tier 2, debug information, backtraces, stack-overflow reporting.
- Layouts of aggregates and of types from transitively imported modules (D24 records the decision path).

## Decisions

### D1. Two new crates: `prism-codegen` and `prism-runtime`

`prism-codegen` holds everything that knows Cranelift: the support and entry checks (`supported.rs`, `entry.rs`), symbol names (`symbols.rs`), representation and signatures (`representation.rs`), lowering of one body (`lower.rs`), one file's code unit (`unit.rs`), the function table and linking (`link.rs`), program execution (`program.rs`), the IR dump (`dump.rs`) and `CodegenError` (`error.rs`). `prism-runtime` holds the `extern "C"` functions that generated code calls and the process state behind them; it depends on nothing, so Tier 2 can later link it as a static library without Cranelift. `prism-cli` gains a `run` module driving the pipeline. Rejected: the runtime inside `prism-codegen` (Tier 2 would drag in Cranelift); a separate `prism-jit` crate now (one consumer, split when Tier 2 arrives).

### D2. Cranelift 0.136.2, pinned, with fixed settings (autonomous)

`cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-jit` and `cranelift-native`, each `=0.136.2` (released 2026-10-02; it needs Rust 1.96, the toolchain is 1.99). The exact pin keeps generated IR, and so `prism dump-clif` output and the timing baseline, stable until a deliberate upgrade. The ISA is the host's (`cranelift_native::builder()`), with `opt_level=speed`, the IR verifier on in debug builds only, frame pointers preserved (cheap, and profilers and later backtraces need them), inline stack probes so a large frame always touches the guard page, and `cranelift-jit`'s own requirements: `use_colocated_libcalls=false`, and `is_pic=true` on x86-64 only (where 0.136 resolves symbols through per-blob GOT entries and accepts nothing else) and `false` elsewhere. *Implementation note:* the pin resolves and builds with Rust 1.99 on macOS arm64 (Apple silicon), where the default `SystemMemoryProvider` maps pages writable and `mprotect`s them executable after finalization, which works without `MAP_JIT` because `prism` is not built with the hardened runtime; a smoke test of slot loads, indirect calls and runtime calls ran there. Cargo resolves a few of Cranelift's internal crates (`cranelift-codegen-meta`, `cranelift-codegen-shared`, `cranelift-assembler-x64`) to `0.136.3`, which `cranelift-codegen 0.136.2` itself allows; `Cargo.lock` pins them. Rejected: `opt_level=none`, whose compile-time saving is small on Tier 1 programs while dev runs of loops benefit from the optimizer; the timing test reports both stages, so this can be revisited with numbers. A version range (`0.136`) was rejected for the reproducibility above.

### D3. One code unit per file, one JIT module per load (autonomous)

Each file's elaborated MIR is lowered into its own code unit (`CodeUnit`: its functions, its data objects, the symbols it defines and the ones it imports), from that MIR alone. The code units of one program load are defined into one `cranelift_jit::JITModule` and finalized together; `hot-reload` will define a changed file's new unit into a new module and repoint slots (D4). This is the "Single File as Compilation Unit" requirement of `docs/architecture.md` §1, and what makes per-file object code cacheable later: a unit's code depends on its file, its direct imports' ABIs (through its own MIR), the compiler and the target, never on how another unit was compiled. Rejected: a `JITModule` per file, which costs separate executable pages and a finalization per file; one Cranelift module with direct cross-file calls, which ties every file's code to its callees' addresses and breaks both hot reload and hermetic per-file artifacts.

### D4. Every call goes through the function table (autonomous)

The linker owns a process-wide **function table**: one pointer-sized slot per symbol (D5) of a function, default generator or `val` accessor, allocated in chunks that never move. A code unit refers to a slot only symbolically, as an imported data symbol `<symbol>#slot`, which the JIT's symbol lookup resolves to the slot's address at link time; a call loads the slot and calls the loaded pointer with `call_indirect`. After finalizing a load, the linker writes each defined function's address into its slot. A slot nothing defines holds the address of `prism_rt_unlinked_function`, which raises a runtime error (in a checked program this is an internal error). Calls within a file go through slots too, and nothing is inlined, so after `hot-reload` repoints a slot every later call of that function, even from a frame of old code such as a running `main` loop, reaches the new code. Calls of runtime functions (D17) are direct: the runtime is never reloaded.

Rejected: direct calls within a file, which leave running old frames calling old code and give two call paths to test; embedding slot addresses as constants in the code, which makes units unrelocatable (no cache, no Tier 2 reuse); `cranelift-jit`'s removed hotswap mode. The cost, a load and an indirect call per call, is acceptable in a development tier; Tier 2 calls directly.

### D5. Symbols from declaration paths (autonomous)

A symbol is derived from the body's identity alone (`symbols.rs`, `symbol_of(&BodyOwner, &MirFile)` and `callee_symbol(&Callee)`):

| Body | Symbol |
|---|---|
| function `name` of module `m` | `m::name` |
| extension `name` on receiver `R` of module `m` | `m::R.name`, `R` written as `prism_abi::receiver_constructor` writes it (`lib/team.Team`, `Int`, `Range`) |
| generator of the default of parameter `p` | `<function symbol>#default(p)` |
| initializer and accessor of top-level `val` `name` | `m::name#init`, `m::name#get` |
| slot of any of these (D4) | `<symbol>#slot` |
| storage of top-level `val` `name` (D18) | `m::name#storage`, `m::name#state` |

`closures` will add `<parent symbol>#closure<i>`, as `prism dump-mir` names closures, and `monomorphization` `<symbol><<type arguments>>`. The receiver uses the module-qualified constructor, as instance records do, so two `Team` types never collide. Within a module a parameter name is unique, so `#default(p)` is too, and it stays stable when parameters are added before `p`. These names contain `/`, `:`, `.`, `#`, `(` and `)`, which the JIT accepts as is; Tier 2 will escape them for object files. Rejected: a C++-style mangling now (unreadable in `dump-clif`, and Tier 2 can escape later); the receiver's bare name (relies on the one-type-per-name-per-module rule staying true).

### D6. Scalar representation and the calling convention (autonomous)

`representation.rs` maps a MIR type to Cranelift types: `Int` to `i64`, `Double` (`Type::Float`) to `f64`, `Bool` to `i8` holding 0 or 1 (what `icmp` produces and Rust's `bool` uses), `Char` to `i32`, a reference whose referent is supported to the pointer type, and `Unit` and `Nothing` to no value. A function's signature is its body's argument types and return type mapped this way, `Unit` parameters and results left out; the caller derives the same signature from the call's `parameters` and destination type, so no ABI lookup is needed. A `Nothing` result has no return value, and the code after a call with no successor is a `trap` (it cannot be reached). Every Prism function uses the host's C calling convention (`isa.default_call_conv()`), so runtime calls, later FFI and Tier 2 share one convention. Aggregate parameters and results (passed by pointer, results through a caller-owned slot) are `codegen-aggregates`'. Rejected: Cranelift's `tail` convention (Prism promises no tail calls, and mixing conventions complicates runtime calls), `i32` booleans, materializing `Unit` as a zero byte.

### D7. Locals: SSA variables unless borrowed

Lowering one body (`lower.rs`, `BodyLowering`) first scans the body for locals that are borrowed (`Rvalue::Borrow` of a place rooted at them). A borrowed local lives in a Cranelift stack slot sized and aligned for its type, and its reads and writes are loads and stores; every other local is a `cranelift_frontend::Variable` per scalar, so Cranelift builds SSA itself (the reason MIR is not SSA, D2 of `mir-lowering`). A local of the built-in `Range` type is three variables (`start`, `end`, `endInclusive`), which `Aggregate` writes and `Field` projections read: scalar replacement, which `codegen-aggregates` can generalize to any struct that is never borrowed. `Unit` and `Nothing` locals have no storage. Arguments are block parameters of the entry block, copied into a stack slot when borrowed. Storage markers and `Read` statements generate nothing, and `Unreachable` calls `prism_rt_panic` with the kind "unreachable" (D12). `SwitchInt` becomes `brif` for a `Bool` and `cranelift_frontend::Switch` for an `Int`. Rejected: every local in a stack slot (Cranelift does little memory-to-register promotion), and a general struct layout for `Range` now.

### D8. References to scalars are in this change (autonomous)

`ROADMAP.md` gave references to `codegen-aggregates`, but MIR already uses them in scalar programs: every top-level `val` read dereferences a `&T`, and every default generator takes `&T` arguments. Supporting `&T` and `&mut T` of supported referents (`Rvalue::Borrow` of a local or of a dereference, the `Deref` projection, `&mut T` to `&T` `Subtype`, built-in `clone()` of a scalar) costs a stack-slot rule (D7) and loads and stores, and it lets defaults and `&mut Int` out-parameters run. `codegen-aggregates` keeps references to aggregates, borrows of fields and payloads, and `(&T)?`. Rejected: special-casing the `const &VAL` pattern (a second, narrower representation for the same MIR), rejecting top-level `val`s and defaults.

### D9. What the backend accepts, and `E6001` (autonomous)

`supported.rs` (`check_file(&MirFile) -> Result<(), Vec<CodegenError>>`) runs on each file's elaborated MIR before any code is generated. It skips generic bodies (non-empty `type_parameters`) and closure bodies, which are never compiled; their uses are what it rejects. For every other body it reports the first unsupported construct, in this order: an unsupported argument or return type at the body's span (the declaration), a composable body at the body's span, then the first statement or terminator, in block order, that uses an unsupported type or operation, at its span. One error per body keeps the output short while still listing every function that needs work.

| Rejected construct | Named as | Supported by |
|---|---|---|
| a local or operand of a struct type other than a `Range` local, `Rvalue::Aggregate` of a struct, a `Field` projection off a `Range` local | structs | `codegen-aggregates` |
| enums, `Discriminant`, variant constants and aggregates | enums | `codegen-aggregates` |
| nullable types, `null`, `WrapNullable`, `Payload` | nullable values | `codegen-aggregates` |
| references to unsupported referents, `Subtype` other than `&mut T` to `&T` | references to `<type>` | `codegen-aggregates` |
| a `Range` parameter, result, borrow or top-level `val` | `Range` values outside locals | `codegen-aggregates` |
| `String` values, string constants, `Index`, `print` of a `String` | strings | `runtime-strings-and-drops` |
| `Heap<T>`, any `Drop` terminator | `Heap` values, drops | `runtime-strings-and-drops` |
| function types, `Constant::Function`, `Rvalue::Closure`, `Callee::Value` | function values and closures | `closures` |
| a call with type arguments | calls of generic functions | `monomorphization` |
| a composable body or call | composable functions | `compose-transform` |
| opaque types | values of unknown type `<name>` | (a type error once unknown names are rejected) |

The diagnostic says "the Tier 1 backend does not support <construct> yet" with a note naming the change that will. It is checked over every compiled body of every file of the program, not only those reachable from `main`: reachability would need a whole-program call graph for a check that disappears as the backend grows, and it would make a program fail or run depending on which functions an edit calls. Rejected: compiling unsupported bodies into stubs that raise a runtime error (a compile-time fact reported at run time), and a compiler panic or `todo!()`.

### D10. `Int` overflow traps, in every tier (autonomous)

`Int` is a 64-bit two's complement integer. `+`, `-` and `*` use Cranelift's overflow-reporting forms (`sadd_overflow`, `ssub_overflow`, `smul_overflow`) and branch to a cold block calling `prism_rt_panic` on overflow; unary `-` checks for the smallest `Int`. `/` and `%` first compare the divisor with zero ("division by zero"); `/` then checks the smallest `Int` divided by `-1` ("integer overflow"); `%` by `-1` gives `0` without dividing, since the result is representable and the hardware would trap. Each check is generated by the backend from the operator and its operand type, and runs at the operator's MIR span, which is also where the panic points. The `for` latch's increments and `end - 1` never overflow (they are guarded by the comparison with the last element), so their checks never fire. Compound assignments lower to the same `Binary` rvalues and behave alike.

This answers the `mir-lowering` follow-up "trap, wrap or check": trap, like Swift in every build and Rust in debug builds. A silent wrap turns an arithmetic bug into wrong data, which is worse for a language that promises memory safety; and an `Int` whose meaning differed between Tier 1 and Tier 2 (Rust's split) would make the release build a different program. Explicit wrapping arithmetic can be added later as separate operators or functions (`wrappingAdd`), as Swift's `&+`. No MIR `Assert` terminator is added: Rust needs one because panics unwind and constant evaluation reads MIR, and a Prism panic does neither (D12), so the borrow checker never needs to see the panic edge. Rejected: wrapping (Kotlin, Java), Rust's tier-dependent split, trapping on `%` by `-1` as Rust does, and hardware traps from `sdiv` (a signal with no message or location).

### D11. `Double` is IEEE 754, `%` is a runtime call

`+`, `-`, `*`, `/`, unary `-` and comparisons are Cranelift's `fadd`, `fsub`, `fmul`, `fdiv`, `fneg` and `fcmp` (`==` ordered-equal, `!=` unordered-or-not-equal, the orderings ordered). Cranelift has no floating-point remainder, so `%` calls `prism_rt_double_remainder`, which is Rust's `%` on `f64` (C's `fmod`: truncated, with the dividend's sign). Equality of `Bool`, `Char` and `Int` is `icmp`; orderings of `Char` and `Int` are signed (a `Char` is below 2^21, so signed and unsigned agree); `!` is `bxor` with 1; `Unit == Unit` is `true`.

### D12. Runtime errors abort with status 101 (autonomous)

A runtime error ("panic") calls `prism_rt_panic(kind: u32, detail: *const u8, detail_length: usize, file: *const u8, file_length: usize, line: u32, column: u32) -> !`. The kinds are integer overflow (the detail is the operator), division by zero, a top-level value read during its own initialization (the detail is the value's name), unreachable code, and an unlinked function. The file is the module path plus `.prism` (`app/main.prism`); it and the details are read-only data in the file's code unit, so locations are workspace-relative and deterministic and a unit needs no shared table. Line and column are immediates from the MIR span. The runtime flushes the program's output, writes `panic: <message>` and `  at <file>:<line>:<column>` to stderr, and exits the process with status 101.

A panic does not unwind: no drop runs and no caller can catch it, as with Rust's `panic=abort`. Process exit releases memory, and with no unwinding there are no landing pads in MIR, no cleanup blocks for drop elaboration to build, and no unwind tables to register with the JIT. A `prism test` runner may later want recoverable failures; it can run each test in its own process, or unwinding can be designed then. Status 101 is Rust's panic status, distinct from 1 (the program did not compile) and from signal statuses. Rejected: unwinding now (cost above, for an error that ends the program), Cranelift traps and signal handlers (no message, platform-specific), status 1 or 134. No backtrace is printed yet.

### D13. The entry point: `fun main()` in the entry file (autonomous)

`entry.rs` (`find_main(&MirFile) -> Result<DeclarationPath, CodegenError>`) looks in the entry file's elaborated MIR for the body owned by `Function(m::main)` with no receiver. None is `E6002` `MissingMain`, at the start of the file, with help showing `fun main() {}`. A `main` with type parameters, arguments, a return type other than `Unit`, or `is_compose`, and a top-level `val main`, are `E6003` `InvalidMain`, at the declaration, saying which rule it breaks. A generic `main` has a polymorphic body and a `val main` a `Value` body, so both are found. The check runs after every file passed Phase 2 and before the support check (D9), so a program with type errors reports those first. Only the entry file's `main` counts; others are ordinary functions. These are code-generation errors (a new `E6xxx` range, D25), not type errors: `prism check` checks libraries, which have no `main`. Rejected: `fun main(): Int` returning the exit status (an exit-status API can come with a process library), `fun main(args: List<String>)` (needs strings and lists), a `main` found in any file.

### D14. `prism run` command line (autonomous)

`prism run [--timings] <file.prism> [-- <argument>...]`. Options come before the file; everything after `--` is the program's and is stored by the runtime (`prism_runtime::start(arguments)`) for a future built-in that reads it once strings and lists exist. Exit status: the program's (0 when `main` returns, 101 after a panic), or 1 for usage errors and for any diagnostic. `--timings` prints, before `main` runs, lines `timing: front end <ms>`, `timing: code generation <ms>`, `timing: linking <ms>` and `timing: total to main <ms>` to stderr, measured with `std::time::Instant` from the start of the `prism` process's `main`. Rejected: rejecting program arguments until they are readable (scripts written now would break later), arguments without `--` (ambiguous with future options), a `PRISM_TIMINGS` environment variable (less discoverable than a flag).

### D15. The output built-ins (autonomous)

`print(x)` and `println(x)`, and `println()`, are compiler built-ins, because Prism has no overloading, traits or prelude module through which a library function could accept several types. In `TypeChecker::infer_call_expr`, an `Ident` callee `print` or `println` that `self.env.lookup` and `self.env.lookup_import` do not find is checked as the built-in: zero or one positional argument as `type-system` says, a `Copy` argument copied out of references, a `String` argument borrowed like a `&String` parameter's (`BorrowOrigin::Argument`, so the borrow checker sees it), and anything else `E4004` with help "`print` accepts an `Int`, `Double`, `Bool`, `Char` or `String`". The typed tree's call is `Callee::Builtin(BuiltinFunction::Print | BuiltinFunction::PrintLine)`, with its parameter type the adjusted argument's type. Naming one without a call stays `E4001`, with help "`println` is a built-in function and can only be called". Being found last, like the built-in `Range` type, keeps every fixture that declares its own `fun print(n: &Int)` meaning what it meant.

The backend calls one runtime function per argument representation: `prism_rt_print_int(i64)`, `prism_rt_print_double(f64)`, `prism_rt_print_bool(u8)`, `prism_rt_print_char(u32)`, then `prism_rt_print_newline()` for `println`. `String` arguments typecheck now and are `E6001` until `runtime-strings-and-drops` adds `prism_rt_print_string`.

A `Double` is written as Kotlin writes it (the `execution` spec): Rust's shortest round-trip digits (`format!("{:e}")`), laid out in plain or scientific notation with Java's thresholds, and `NaN`, `Infinity`, `-Infinity`. Kotlin users expect `1.0E10` and `Infinity`; Rust's `1e10` and `inf` were rejected, as was `{}` (which prints `1` for `1.0`). Rejected also: `print` in a Prism-written prelude (needs overloading or traits), string templates (need strings), printing nullables as `null` now (needs nullables in the backend).

### D16. Output is buffered by the runtime (autonomous)

The runtime appends output to a buffer and writes it to file descriptor 1 when it reaches 8 KiB, at program end, before a panic message, and after each newline when stdout is a terminal (`std::io::IsTerminal`). Rejected: a write per `print` (a system call each), Rust's `LineWriter` (line-buffered even into pipes and files, slow for large outputs), C's `stdio` (format and Windows differences).

### D17. The runtime library and how JIT code finds it

`prism-runtime` exposes `extern "C"` functions (`prism_rt_*`) and `pub static RUNTIME_FUNCTIONS: &[RuntimeFunction]`, each entry a name, an address and a signature over a small `RuntimeType` enum (`I8`, `I32`, `I64`, `F64`, `Pointer`), so the codegen declares each import with the right Cranelift signature from one table and registers all addresses with `JITBuilder::symbols`. Its state (output buffer, arguments) is process-wide, behind a `Mutex`: Prism programs are single-threaded today, and the cost is negligible next to formatting. `prism_runtime::start(arguments)` and `prism_runtime::finish() -> i32` (which flushes) bracket a run. Rejected: generating formatting code in Cranelift, and calling libc's `printf`.

### D18. Top-level `val`s initialize lazily, through their accessor (autonomous)

Each top-level `val` of a supported type gets, in its file's unit, a writable data object for its value (`#storage`) and a state byte (`#state`: 0 uninitialized, 1 initializing, 2 initialized), its MIR body compiled as `#init` (returning the value), and a generated accessor `#get` returning a pointer to the storage: state 2 returns it at once; 0 sets 1, calls `#init` through its slot, stores the result, sets 2 and returns it; 1 returns a null pointer. `Constant::TopLevelValue(path)` becomes a call of `path#get` through its slot, in the same file or another, followed by a null test whose cold branch panics with "read during its own initialization" at the read's own location, so the accessor needs no location arguments. Values are immutable, so a pointer to initialized storage stays valid. There are no threads yet; a thread-safe once comes with them.

Rejected: eager initialization before `main` in dependency order (a cross-file order needs every initializer's body, which per-file units do not see, and it would run initializers the program never needs); constant evaluation only (Rust's statics: would reject initializers the type checker accepts today); Kotlin's per-file initialization (reading one value initializes all of its file's, a hidden coupling).

### D19. The program runs on its own thread (autonomous)

`Program::run` spawns one thread with an 8 MiB stack, calls `prism_runtime::start`, calls `main` through its slot, calls `prism_runtime::finish`, and joins, so the stack a program gets does not depend on how `prism` was started or on the platform's main-thread size (1 MiB on Windows). A stack overflow hits the thread's guard page and aborts through Rust's handler ("has overflowed its stack", signal status); reporting it as a Prism panic is a follow-up.

### D20. `prism run` reuses Phase 2 and does not use the build cache (autonomous)

`check_phase_two` is split: `elaborate_file(graph, abis, path) -> Result<MirFile, WorkspaceError>` returns the elaborated MIR, and `check_phase_two` maps it to instance requests. `run::compile_program(entry)` validates the entry, builds the graph, extracts ABIs, then elaborates every file in topological SCC order, stopping at the first file with errors as `prism check` does; then it finds `main` (D13), checks every file's MIR (D9, stopping at the first file with `E6001`), lowers each file into a unit, and links. Errors become `WorkspaceError::Codegen { path, source, errors }`, rendered by `diagnostics::format_codegen_errors`. The build cache is neither read nor written: a cached verdict carries no MIR, and caching generated code is a follow-up (D24). Rejected: writing check verdicts from `prism run` (two commands with different cache behaviour to keep in step for no gain yet).

### D21. Executable fixtures run as subprocesses (autonomous)

A fixture whose header has `// run: <status>` beside its `// expect:` line (usually `ok`; a case showing that `prism run` reports type errors expects their codes) is also run: the runner spawns `env!("CARGO_BIN_EXE_prism") run <entry>` with the case directory as working directory and the entry relative to it, so rendered paths are relative, and compares the exit status, and stdout and stderr with the case's `stdout.snap` and `stderr.snap`, an absent file meaning empty. `PRISM_BLESS=1` writes both. The `E6xxx` codes in a run case's stderr count as exercised for the catalog check, so `E6001` to `E6003` need no exemption. Every case of the `execution` spec must have a `// run:` line. Rejected: inline `// stdout:` lines (a second way to state output, and unable to express output without a final newline); running in process (a panic exits the process, and stdout is process-wide).

### D22. The 15 ms target is reported, not asserted (autonomous)

`crates/prism-cli/tests/run_timing.rs` has an `#[ignore]` test, run as `cargo test --release -p prism-cli --test run_timing -- --ignored --nocapture`, that runs `prism run --timings examples/run/main.prism` 20 times, parses the `total to main` lines and prints the median and 90th percentile per stage against the 15 ms target; it fails on the time only when `PRISM_ASSERT_TIMING=1` is set, for local use on a quiet machine. A non-ignored test checks that `--timings` prints its four lines. `examples/run/` is a small multi-file scalar program (`main.prism` importing `/math/numbers`: primes, gcd, a sum over a range, a top-level `val`), which `prism check` on `examples/` also covers. Rejected: a hard assertion in CI (shared runners and debug builds make it flaky), and timing inside the debug test binary only.

### D23. `prism dump-clif` (autonomous)

`prism dump-clif <file.prism>` builds the entry file's elaborated MIR as `prism dump-mir --elaborated` does (against its direct imports' ABIs, without their bodies), runs the support check, lowers the file into a unit without linking it, and prints each function's `cranelift_codegen::ir::Function` text in body order, each headed by `function <symbol>` and with external names shown by symbol. It exists for debugging and for the scenarios that look at generated code (indirect calls, signatures, independence from dependency bodies); executable fixtures are the main tests, so there are no `clif.snap` snapshots, which every Cranelift upgrade would churn.

### D24. Deferred, with the decision path

- **Layouts of types from transitively imported modules** (from `mir-lowering`). Scalars need none, so nothing is decided here. Recommended for `codegen-aggregates`: the ABI of a file carries the definitions (fields, variants, markers) of every type reachable from its exported signatures and from the fields and payloads of its exported types, transitively, collected in Phase 1 from the other files' Phase 1a headers. An importer then lays out every type it handles from its direct imports' ABIs alone, and since those definitions are in the ABI hash, a field added to a transitively used type rebuilds exactly the files whose layouts change, which keeps deterministic hermetic builds. Alternatives to weigh then: per-program layout computation at link time (breaks per-file artifacts), and Swift-style resilient layouts queried at run time (not zero-cost).
- **Caching generated code.** A unit is relocatable (D3, D4), so its code bytes and relocations can be cached keyed like check entries plus the target triple, which is when the target triple joins the keys (the `abi-cache` follow-up). A follow-up item, not tied to a numbered change.
- **Hot reload** (`hot-reload`): defining a changed file's new unit into a new `JITModule`, repointing its slots, and what happens to the values of its top-level `val`s.

### D25. Two new capabilities and the `E6xxx` range (autonomous)

`code-generation` (the backend's contract: what it compiles and rejects, units, symbols, the table, representation, `dump-clif`) and `execution` (what a running program does) separate what later backend changes extend (`codegen-aggregates` adds layouts to the first) from language runtime semantics that every tier must share (Tier 2 adds nothing to the second). Loop evaluation goes to `loops`, beside its static rules. The new stage gets `E6xxx`: `E6001` `UnsupportedByBackend`, `E6002` `MissingMain`, `E6003` `InvalidMain`, one per fix (wait for the backend or avoid the construct; add a `main`; fix its signature). Rejected: one `execution` spec for both (mixes a backend's temporary limits with permanent semantics), and type-checker codes for the entry point (D13).

## Risks / Trade-offs

- [Cranelift's API changes between versions] → the exact pin (D2); upgrades are deliberate commits that re-run the fixtures.
- [Every call is indirect, so Tier 1 is slower than it could be] → a load and a predicted indirect branch per call; Tier 2 calls directly. The timing test measures compile time, not run time, so a regression would show in programs, not CI.
- [`E6001` on any unsupported body rejects programs whose `main` never reaches it] → temporary: the backend changes that follow remove each row of D9's table, and the diagnostic names which one.
- [Trapping on overflow costs a branch per arithmetic operation, and Kotlin programmers may expect wrapping] → the branch is predicted and cold; the `execution` spec and the panic message make the behaviour explicit.
- [Panics do not unwind, so a future in-process test runner cannot recover from one] → run tests in subprocesses, or design unwinding with `prism test`; nothing here prevents it.
- [Lazy top-level values make the order of side effects in initializers depend on first use] → initializers with side effects are unusual, the order is deterministic and specified, and a cycle is a runtime error, not a hang.
- [The JIT needs writable then executable memory, which hardened macOS processes restrict] → `cranelift-jit`'s default memory provider handles `MAP_JIT` and write protection on Apple silicon; `prism` is not built with the hardened runtime.
- [Stack overflow aborts with a signal status rather than 101] → listed as a follow-up; the 8 MiB stack makes it consistent across platforms.

## Migration Plan

No external users. In-repo, fixtures that declare their own `fun print` keep working because the built-ins are found last (D15). The new crates and the `run` and `dump-clif` commands are additions. Rollback is a revert.

## Open Questions

- The exact wording of the `E6001` messages and their notes can be tuned during implementation, provided each names the construct and the change that will support it.
- Whether `opt_level=speed` or `none` gives the better Tier 1 trade-off can be settled from the timing test's numbers without changing the specs (D2).
