# MIR Specification Delta

## ADDED Requirements

### Requirement: Drops Are Elaborated After Borrow Checking
When a file passes the borrow checker, each drop of its MIR MUST be elaborated: removed where the value was moved on every path reaching it, kept where it holds a value on every path, and otherwise guarded by a drop flag, a `Bool` local that is true exactly when the value is present. A drop flag MUST be set where its value is assigned and cleared where the value moves and where its storage starts or ends.

#### Scenario: A moved value is not dropped
- **WHEN** `fun take(s: String) {}` is declared and `fun f(s: String) { take(s) }` passes the borrow checker
- **THEN** the elaborated MIR of `f` calls `take` with a move of `s` and has no drop

#### Scenario: A conditionally moved value is dropped under a flag
- **WHEN** `fun take(s: String) {}` is declared and `fun f(c: Bool, s: String) { if (c) take(s) }` passes the borrow checker
- **THEN** the elaborated MIR of `f` has a drop flag for `s`, set on entry and cleared where `s` moves into `take`, and the drop of `s` before the return runs only when the flag is set

#### Scenario: Assigning over a possibly moved variable
- **WHEN** `fun take(s: String) {}` is declared and `fun f(c: Bool) { var s = "a"; if (c) take(s); s = "b" }` passes the borrow checker
- **THEN** in the elaborated MIR the drop of the old value of `s` before `"b"` is written runs only when the flag of `s` is set, and the drop of `s` at the end of the block has no flag

#### Scenario: Reinitialization after a move drops nothing
- **WHEN** `fun take(s: String) {}` is declared and `fun f() { var s = "a"; take(s); s = "b" }` passes the borrow checker
- **THEN** in the elaborated MIR the assignment of `"b"` drops nothing, and `s` is dropped at the end of the block without a flag

#### Scenario: Deterministic scope exit cleanup
- **WHEN** `fun g() {}` is declared and a body of `fun f(c: Bool)` declares `val a = "a"`, then runs `if (c) { val b = "b" }`, then `g()`
- **THEN** in the elaborated MIR `b` is dropped at the end of the `if` branch, before `g` is called, and `a` after `g` returns, at the end of the body, each without a flag

#### Scenario: Drop of a declaration that a continue skipped
- **WHEN** `fun f(c: Bool, d: Bool) { do { val s = "a"; if (c) continue; val t = "b" } while (d) }` passes the borrow checker
- **THEN** in the elaborated MIR `s` is dropped without a flag on both edges of the condition, and the drop of `t` is guarded by a flag that the `continue` path leaves cleared

## MODIFIED Requirements

### Requirement: Values Are Dropped Where Their Scope Ends
A local or temporary whose type needs a drop (any type except `Copy` types, mutable borrows and nullables of these) MUST be dropped where its scope ends, in reverse order of creation, on every path leaving the scope; arguments MUST be dropped when the body returns, after its locals. In the MIR as lowered, a drop MUST NOT depend on whether the value was moved: drop elaboration refines it once the file passes the borrow checker.

#### Scenario: Locals dropped in reverse order
- **WHEN** `fun f() { val a = "a"; val b = "b" }` is lowered
- **THEN** at the end of the block `b` is dropped, then `a`

#### Scenario: A moved value keeps its drop
- **WHEN** `fun take(s: String) {}` is declared and `fun f(s: String) { take(s) }` is lowered
- **THEN** `take` is called with a move of the argument `s`, and `s` is still dropped when `f` returns in the MIR as lowered

#### Scenario: Early return drops the live locals
- **WHEN** `fun f(c: Bool): Int { val s = "x"; if (c) return 1; return 2 }` is lowered
- **THEN** both paths to the return drop `s` before returning

#### Scenario: Copy values are not dropped
- **WHEN** `fun f(n: Int, r: &mut Int) { val m = n }` is lowered
- **THEN** the body has no drop

### Requirement: Call Arguments Are Evaluated in Order
A call MUST evaluate its receiver first: a receiver that the call moves or copies (the receiver of a `consuming fun`, or a `Copy` receiver copied out of a reference) is moved or copied then, and a receiver that the call borrows has its place evaluated then and is borrowed after the written arguments. The written arguments MUST be evaluated in source order, then the omitted arguments in parameter order, and then the call made.

#### Scenario: A mutable receiver is borrowed after the arguments
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}` and `fun Stack.size(): Int = count` are declared and a body runs `var v = Stack(0); v.push(v.size())`
- **THEN** `size` is called before the mutable receiver borrow of `v` is taken, and `push` is called with that borrow and the result of `size`

#### Scenario: A consuming receiver is moved before the arguments
- **WHEN** `struct Stack { var count: Int }`, `struct Point : Copy { val x: Int }`, `consuming fun Stack.into(n: Int): Int = n`, `consuming fun Point.moved(n: Int): Point = this` and `fun next(): Int = 1` are declared, and `fun f(s: Stack): Int = s.into(next())` and `fun g(p: &Point): Point = p.moved(next())` are lowered
- **THEN** `s` is moved into a temporary, and the dereference of `p` copied into one, before `next` is called, and each call receives that temporary and the result of `next`

#### Scenario: Named arguments are evaluated as written
- **WHEN** `fun g(): Int = 1`, `fun h(): Int = 2` and `fun f(a: Int, b: Int = 2, c: Int = 3): Int = a` are declared and `fun k(): Int = f(c = g(), a = h())` is lowered
- **THEN** `g` is called before `h`, then the generator of `b`, and `f` is called with the results of `h`, the generator and `g`, in that order

### Requirement: Loops Become Cycles of Blocks
`while`, `do`/`while` and `for` MUST become cycles of blocks. A `for` loop MUST evaluate its range once and step its variable from the start to the last element without overflowing. `break` and `continue` MUST jump to their loop's exit or next iteration, dropping every scope they leave.

#### Scenario: For loop over a closed range
- **WHEN** `fun g(i: Int) {}` is declared and `fun f(n: Int) { for (i in 0..n) g(i) }` is lowered
- **THEN** the range is evaluated once into a local, the loop is skipped when the range is empty, and after each call of `g` the variable is compared with the last element before it is incremented

#### Scenario: Break drops the scopes it leaves
- **WHEN** `fun f(c: Bool, d: Bool) { while (c) { val s = "x"; if (d) break } }` is lowered
- **THEN** the path of the `break` drops `s` and then jumps to the block after the loop

#### Scenario: Continue in a do-while reaches the condition
- **WHEN** `fun next(): Int = 0` is declared and `fun f(c: Bool) { do { val s = "a"; if (c) continue; val x = next() } while (x > 0) }` is lowered
- **THEN** the path of the `continue` keeps `s`, marks `x` live without assigning it, and jumps to the condition, after which `x` and `s` end on both edges; reading the unassigned `x` there is a read of an uninitialized value, which the borrow checker rejects with `E5002` (see `ownership`)

### Requirement: MIR Is Well Formed
The MIR of a file without type errors MUST satisfy its structural invariants: every block ends in a terminator and is reachable, every local used is declared, every assignment's value has its place's type except for an explicit subtyping conversion, projections and call operands are well typed, no operand moves a place through a dereference, only values whose type needs a drop are dropped, and every local is storage-live wherever it is used. Drop flags MUST appear only in elaborated MIR, as `Bool` locals without storage markers.

#### Scenario: Error-free MIR
- **WHEN** a file typechecks successfully
- **THEN** its MIR satisfies every structural invariant, and so does its elaborated MIR when it passes the borrow checker

### Requirement: MIR Text Dump
`prism dump-mir <file.prism>` MUST print the MIR of the file, checked against its direct imports' ABIs, as deterministic text: each body with its locals and its blocks, one statement or terminator per line with its line and column. With type errors it MUST report them, print no MIR, and exit with a failure status. `prism dump-mir --elaborated <file.prism>` MUST print the elaborated MIR instead, and with borrow-checker errors MUST report them, print no MIR, and exit with a failure status.

#### Scenario: Dumping a file
- **WHEN** `prism dump-mir main.prism` runs twice on a file that typechecks successfully
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file with type errors
- **WHEN** `prism dump-mir main.prism` runs on `fun f(): Int = nope + 1`
- **THEN** it reports `E4001`, prints no MIR, and exits with a failure status

#### Scenario: Dumping elaborated MIR
- **WHEN** `fun take(s: String) {}` is declared and `prism dump-mir --elaborated main.prism` runs on a file holding `fun f(s: String) { take(s) }`, and then on one holding `fun g(s: String) { take(s); take(s) }`
- **THEN** the first prints the elaborated MIR, marked as elaborated, and exits with status 0; the second reports `E5001`, prints no MIR and exits with a failure status, while `prism dump-mir main.prism` still prints its MIR as lowered
