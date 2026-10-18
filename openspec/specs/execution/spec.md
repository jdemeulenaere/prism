# Execution Specification

## Purpose

The execution specification defines what happens when a Prism program runs: how `prism run` starts it and reports its end, the program's entry point, what the output built-ins print, the runtime meaning of scalar arithmetic and evaluation order, how runtime errors end a program, and when top-level values are initialized.

## Requirements

### Requirement: Running a Program
`prism run <file.prism>` MUST check the entry file and every file it transitively imports as `prism check` does, compile them, and call the entry file's `main`. Diagnostics MUST go to stderr and the program's output to stdout. It MUST exit with status 0 when `main` returns, and with status 1, without running any of the program, when the program does not compile.

#### Scenario: Running a multi-file program
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, and `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(21)) }`
- **THEN** `prism run app/main.prism` prints `42` followed by a newline to stdout, nothing to stderr, and exits with status 0

#### Scenario: A program that does not compile
- **WHEN** `prism run main.prism` runs on `fun main() { println(1); val x: Int = true }`, or on `fun take(s: String) {}` and `fun main() { val s = "a"; take(s); take(s) }`
- **THEN** it reports `E4002`, or `E5001`, on stderr, prints nothing to stdout and exits with status 1

### Requirement: Program Arguments and Options
Arguments after `--` MUST be passed to the program and never interpreted by `prism`; no built-in reads them yet. The `--timings` option, before the file, MUST print to stderr the time spent in each stage (front end, code generation, linking) and the total time from the start of `prism` until `main` is called, before `main` runs.

#### Scenario: Arguments after the separator
- **WHEN** `prism run main.prism -- --timings x` runs on `fun main() { println(1) }`
- **THEN** it prints `1` to stdout, nothing to stderr, and exits with status 0

#### Scenario: Reporting timings
- **WHEN** `prism run --timings main.prism` runs on `fun main() { println(1) }`
- **THEN** stderr holds one line per stage and one line with the total time to `main`, each in milliseconds, and stdout holds `1`

### Requirement: Program Entry Point
The entry file MUST declare a top-level function `main` that has no type parameters, receiver or parameters, returns `Unit` (written or omitted) and is not `compose`. Without one, `prism run` MUST report `E6002` (`MissingMain`); a `main` breaking these rules, or a top-level `val` named `main`, MUST be reported as `E6003` (`InvalidMain`). A `main` of any other file is an ordinary function.

#### Scenario: Missing main
- **WHEN** `prism run main.prism` runs on a file declaring only `fun helper() {}`
- **THEN** it reports `E6002` and exits with status 1

#### Scenario: Invalid main
- **WHEN** the entry file declares `fun main(n: Int) {}`, or `fun main(): Int = 0`, or `fun <T> main() {}`, or `compose fun main() {}`, or `val main: Int = 1`
- **THEN** `prism run` reports `E6003` at `main` and exits with status 1

#### Scenario: A main of an imported file is ordinary
- **WHEN** `lib/tool.prism` declares `public fun main(n: Int): Int = n`, and `app/main.prism` imports `/lib/tool` and declares `fun main() { println(2) }`
- **THEN** `prism run app/main.prism` prints `2` and exits with status 0

### Requirement: Output Built-Ins Print Their Argument
`print(x)` MUST write the text of `x` to stdout, `println(x)` the text of `x` followed by a newline, and `println()` a newline. An `Int` MUST be written in decimal with a leading `-` when negative, a `Bool` as `true` or `false`, and a `Char` as its UTF-8 encoding.

#### Scenario: Printing scalars
- **WHEN** `fun main() { print(1); print(true); println('x'); println(-5); println() }` runs
- **THEN** stdout holds `1truex`, a newline, `-5`, a newline, and a newline

### Requirement: Double Text Form
A `Double` MUST be written as Kotlin writes it (Java's `Double.toString`): as the shortest decimal that reads back as the same value, or, when that decimal has a single significant digit, as the closest decimal of one or two significant digits that reads back as the same value; with at least one digit after the point; in scientific notation (`1.0E7`, `1.0E-4`) when its magnitude is at least 10^7 or below 10^-3; and as `NaN`, `Infinity`, `-Infinity` and `-0.0` for those values.

#### Scenario: Printing doubles
- **WHEN** a program prints `1.0`, `0.1 + 0.2`, `10000000.0`, `0.0001`, `1.0 / 0.0`, `0.0 / 0.0` and `-0.0`, each with `println`
- **THEN** stdout holds the lines `1.0`, `0.30000000000000004`, `1.0E7`, `1.0E-4`, `Infinity`, `NaN` and `-0.0`

#### Scenario: Printing the smallest double
- **WHEN** a program prints `4.9E-324`, the smallest positive `Double`, and `1.0E-323`, its double, each with `println`
- **THEN** stdout holds the lines `4.9E-324` and `9.9E-324`, the closest two-digit decimals, rather than the shortest `5.0E-324` and `1.0E-323`

### Requirement: Output Is Buffered and Flushed
Output MUST appear in the order the program writes it. It MUST be written to stdout at the latest when the program ends, before a runtime error is reported, and at each newline when stdout is a terminal.

#### Scenario: Output before a runtime error
- **WHEN** `fun zero(): Int = 0` is declared and `fun main() { println(1); println(1 / zero()) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds the runtime error, and the exit status is 101

### Requirement: Integer Arithmetic
`Int` MUST be a 64-bit two's complement integer. `+`, `-`, `*`, unary `-` and `/` MUST raise the runtime error "integer overflow" when the exact result does not fit; `/` and `%` by zero MUST raise "division by zero". `/` MUST truncate toward zero and `%` MUST take the sign of the dividend, so the smallest `Int` `% -1` is `0`. Compound assignments MUST behave as their operators.

#### Scenario: Overflowing addition
- **WHEN** `fun main() { val big = 9223372036854775807; println(big + 1); println(2) }` runs
- **THEN** stdout is empty, stderr holds `panic: integer overflow in '+'` and the location of `big + 1`, and the exit status is 101

#### Scenario: Overflow in every form
- **WHEN** a program evaluates `9223372036854775807 * 2`, or `-9223372036854775807 - 2`, or `-m` or `m / -1` for `val m = -9223372036854775807 - 1`, or runs `var n = 9223372036854775807; n += 1`
- **THEN** it raises "integer overflow", naming `*`, `-`, `-`, `/` and `+` respectively, and exits with status 101

#### Scenario: Division by zero
- **WHEN** `fun zero(): Int = 0` is declared and a program evaluates `7 / zero()`, or `7 % zero()`
- **THEN** it raises "division by zero" and exits with status 101

#### Scenario: Truncating division and remainder
- **WHEN** a program prints `-7 / 2`, `-7 % 2`, `7 % -2` and `m % -1` for `val m = -9223372036854775807 - 1`
- **THEN** it prints `-3`, `-1`, `1` and `0`

### Requirement: Double Arithmetic
`Double` arithmetic MUST follow IEEE 754 binary64 with rounding to nearest and MUST never raise a runtime error: division by zero gives an infinity or NaN. `%` MUST be the remainder of truncated division, with the sign of the dividend. Every comparison with NaN MUST be false, except `!=`, which is true.

#### Scenario: IEEE results
- **WHEN** a program prints `-7.5 % 2.0`, `1.0 / 0.0 > 1.0`, and, for `val n = 0.0 / 0.0`, `n == n` and `n != n`
- **THEN** it prints `-1.5`, `true`, `false` and `true`

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. No further code of the program MUST run, and no drop MUST run.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

### Requirement: Stack Overflow Is a Runtime Error
A program whose calls nest deeper than its stack allows MUST raise the runtime error "stack overflow" at the declaration of the function being entered, which ends the program as any runtime error does. A program MUST get a stack of the same size on every platform.

#### Scenario: Unbounded recursion
- **WHEN** `fun deep(n: Int): Int = 1 + deep(n + 1)` is declared and `fun main() { println(1); println(deep(0)) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds `panic: stack overflow` and the location of the declaration of `deep`, and the exit status is 101

### Requirement: Top-Level Values Initialize on First Read
A top-level `val`'s initializer MUST run when the program first reads the value, and only then; later reads MUST give the same value without running it again. A value the program never reads MUST never be initialized. Reading a value while its own initializer runs MUST raise the runtime error "top-level value '<name>' read during its own initialization".

#### Scenario: Initialized once on first read
- **WHEN** `fun noisy(): Int { println(0); return 5 }`, `val V: Int = noisy()` and `val W: Int = noisy()` are declared and `fun main() { println(1); println(V); println(V) }` runs
- **THEN** stdout holds the lines `1`, `0`, `5` and `5`

#### Scenario: Reading a value during its own initialization
- **WHEN** `val A: Int = B + 1` and `val B: Int = A + 1` are declared and `fun main() { println(A) }` runs
- **THEN** the program raises "top-level value 'A' read during its own initialization" at the read of `A` in `B`'s initializer and exits with status 101

### Requirement: When Branches Evaluate in Order
A `when` MUST evaluate its subject once, then test its patterns in order, evaluating each pattern only when the earlier ones did not match, and evaluate only the branch of the first pattern that matches, or the `else` branch when none does.

#### Scenario: When pattern matching
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 10; in 1..10 -> 20; else -> 30 }` is declared and a program prints `f(0)`, `f(5)` and `f(11)`
- **THEN** it prints `10`, `20` and `30`

#### Scenario: Later patterns are not evaluated
- **WHEN** `fun loud(n: Int): Int { println(n); return n }` is declared and a program runs `when (loud(1)) { loud(1) -> println(7); loud(2) -> println(8); else -> println(9) }`
- **THEN** it prints `1`, `1` and `7`, and `loud(2)` is never called

#### Scenario: Exhaustive when over a Bool
- **WHEN** `fun f(b: Bool): Int = when (b) { true -> 1; false -> 0 }` is declared and a program prints `f(true)` and `f(false)`
- **THEN** it prints `1` and `0`, with no `else` branch

### Requirement: Logical Operators Short-Circuit
`&&` MUST evaluate its right operand only when its left operand is `true`, and `||` only when its left operand is `false`.

#### Scenario: Right operands skipped
- **WHEN** `fun loud(b: Bool): Bool { println(b); return b }` is declared and a program runs `println(loud(false) && loud(true))` and `println(loud(true) || loud(false))`
- **THEN** it prints `false`, `false`, `true` and `true`

### Requirement: Tier 1 Instant Execution
On a release build, `prism run` MUST reach the program's `main` in under 15 ms for the reference program `examples/run/main.prism` on a developer machine, without invoking any external program. A timing benchmark MUST report this time against the target; it MUST NOT fail a test run on the time alone.

#### Scenario: Tier 1 instant script execution
- **WHEN** `prism run --timings examples/run/main.prism` runs on a release build of `prism`
- **THEN** no other process is started, and the reported total time to `main` is under 15 ms

## Design Rationale

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`Int` overflow traps in every tier** (D10). `+`, `-` and `*` use Cranelift's overflow-reporting forms and branch to a cold block calling `prism_rt_panic`; unary `-` and `/` check the smallest `Int`; `/` and `%` check for zero first; `%` by `-1` gives `0` without dividing. Each check is generated by the backend at the operator's span, and the `for` latch never overflows. This is Swift's rule, and Rust's in debug builds: a silent wrap turns an arithmetic bug into wrong data, and an `Int` that differed between Tier 1 and Tier 2 would make the release build a different program. Explicit wrapping operations can come later. No MIR `Assert` terminator: a Prism panic neither unwinds nor is evaluated at compile time. Rejected: wrapping (Kotlin, Java), Rust's tier-dependent split, trapping on `% -1`, hardware traps from `sdiv`.
- **`Double` is IEEE 754** (D11), with ordered comparisons except `!=`, and `%` as a runtime call (`prism_rt_double_remainder`, C's `fmod`), since Cranelift has no remainder instruction.
- **Runtime errors abort with status 101** (D12). `prism_rt_panic(kind, detail, file, line, column)` flushes the output, writes `panic: <message>` and `  at <file>:<line>:<column>` (the module path plus `.prism`, so workspace-relative and deterministic) and exits with 101, Rust's panic status, distinct from 1 (did not compile) and from signal statuses. There is no unwinding, so no landing pads, cleanup blocks or unwind tables; a `prism test` runner can run tests in subprocesses. No backtrace yet. Rejected: unwinding now, signal handlers, status 1 or 134.
- **The entry point is `fun main()` of the entry file** (D13): no type parameters, receiver or parameters, a `Unit` result, not `compose`. Missing is `E6002`, invalid (including a `val main`) `E6003`, both code-generation errors because `prism check` checks libraries. Rejected: `main(): Int` as the exit status (wait for a process library), `main(args)` (needs strings and lists), a `main` in any file.
- **`prism run [--timings] <file> [-- <argument>...]`** (D14). Arguments after `--` are recorded by the runtime for a future built-in, so scripts written now keep working; `--timings` prints the front end, code generation, linking and total time to `main`. Rejected: rejecting arguments until readable, arguments without `--`, an environment variable for timings.
- **Output built-ins print as Kotlin does** (D15, review). `print` and `println` call one runtime function per representation. A `Double` uses Rust's shortest round-trip digits laid out with Java's plain and scientific thresholds, and, where those digits are a single one, the closest decimal of one or two digits, as Java's `Double.toString` does (`4.9E-324`), which only differs for subnormals. Rejected: Rust's `1e10` and `inf`, `{}` (prints `1` for `1.0`).
- **Output is buffered by the runtime** (D16): written at 8 KiB, at the end, before a runtime error, and at each newline on a terminal. Rejected: a write per `print`, Rust's `LineWriter` (line-buffered into pipes too), C's `stdio`.
- **The runtime library** (D17) exports `RUNTIME_FUNCTIONS` (name, address, signature over a small `RuntimeType`), from which the codegen declares its imports and the JIT resolves them; its state is process-wide behind a `Mutex`, bracketed by `start` and `finish`. Rejected: formatting in generated code, libc's `printf`.
- **Top-level `val`s initialize lazily, through an accessor** (D18). Each has `#storage`, a `#state` byte (uninitialized, initializing, initialized), `#init` from its MIR body, and a generated `#get` returning the storage's address, or null when called during its own initialization, which the reader turns into the runtime error at the read's location. Rejected: eager initialization in dependency order (per-file units cannot see every initializer, and it runs initializers the program never needs), constant evaluation only (rejects initializers the type checker accepts), Kotlin's per-file initialization (a hidden coupling). A thread-safe once comes with threads.
- **The program runs on its own thread with an 8 MiB stack** (D19), so its stack does not depend on how `prism` was started or on the platform's main-thread size.
- **Stack overflow is a runtime error, detected on entry** (D27, from the review). Every compiled body compares the stack pointer with the runtime static `prism_rt_stack_limit`, which `Program::run` sets 256 KiB (`STACK_RESERVE`) above the end of the program thread's stack, and raises "stack overflow" at the body's declaration below it, so the output is flushed and the status is 101 rather than an abort from the guard page that loses buffered output. The limit is zero, disabling the check, outside a run. Rejected: a guard-page signal handler (platform-specific `sigaction`, a `libc` dependency, flushing from a signal handler), Cranelift's trapping `stack_limit` check (no message).
- **`prism run` reuses Phase 2 and bypasses the build cache** (D20): a cached verdict carries no MIR, and caching generated code is a follow-up. Errors stop at the first file with errors, as `prism check` does.
- **Executable fixtures** (D21). A fixture with `// run: <status>` is run as a `prism run` subprocess from its directory and compared with `stdout.snap` and `stderr.snap`, blessed with `PRISM_BLESS=1`; `E6xxx` codes in run stderr count for the catalog check, and every `execution` case is executable. Rejected: inline `// stdout:` lines, running in process (a panic exits the process).
- **The 15 ms target is reported, not asserted** (D22). An ignored release benchmark (`crates/prism-cli/tests/run_timing.rs`) runs `prism run --timings examples/run/main.prism` and prints the median and 90th percentile per stage, failing only with `PRISM_ASSERT_TIMING=1`; shared CI runners and debug builds would make an assertion flaky. Measured at implementation: a 1.8 ms median to `main` on Apple silicon.
