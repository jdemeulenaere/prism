# MIR Specification

## Purpose

The MIR is the control-flow graph of each body of a file that typechecks: typed locals, places, explicit storage, drops, branches and calls, from which the borrow checker and the code generator work without re-deriving any decision of the type checker.

## Requirements

### Requirement: MIR for Every File Without Type Errors
A file that typechecks MUST be lowered to MIR holding one body per function with a body, per top-level `val`, per parameter default value and per closure. A file with type errors MUST NOT be lowered.

#### Scenario: Bodies of a file
- **WHEN** a file declares `fun run(g: () -> Int): Int = g()`, `fun f(a: Int, b: Int = a + 1): Int = run { a + b }` and `val LIMIT = 3`
- **THEN** its MIR holds a body for `run`, for `f`, for the default value of `b`, for the closure in `f`, and for `LIMIT`

### Requirement: Locals Are Typed and Ordered
Every body MUST declare each of its locals with a type: the return place first, then the arguments in parameter order (the receiver first), then a closure's captures, then bindings and temporaries in order of creation. A local that stands for a binding of the typed tree MUST record the binding's name.

#### Scenario: Locals of a function
- **WHEN** `fun f(a: Int, s: String): Int { val n = a; return n }` is lowered
- **THEN** its body declares the return place of type `Int`, the arguments `a: Int` and `s: String`, and a local `n: Int`

### Requirement: Statements Assign Rvalues to Places
Every place MUST be a local followed by projections: dereferences, fields by index, nullable payloads and indices. A consumed place MUST be a move or copy operand, and every borrow MUST be an rvalue over a place recording whether it is shared or mutable and why it exists.

#### Scenario: Field read through references
- **WHEN** `struct User { val id: Int }` is declared and `fun f(u: &&User): Int = u.id` is lowered
- **THEN** the return place is assigned a copy of field `id` of the place reached by two dereferences of the argument `u`

#### Scenario: Moves, copies and borrows
- **WHEN** `fun greet(n: &String): Int = 1` is declared and `fun f(s: String, n: Int): Int { val a = n; val b = s; return greet(b) }` is lowered
- **THEN** `a` is assigned a copy of `n`, `b` a move of `s`, and `greet` is called with a shared argument borrow of `b`

### Requirement: Storage Is Marked
Every local other than the return place, the arguments and a closure's captures MUST be marked live where its scope starts and dead where its scope ends, on every path that leaves the scope.

#### Scenario: Storage of a block local
- **WHEN** `fun f(c: Bool) { if (c) { val x = 1 } }` is lowered
- **THEN** `x` is marked live before it is assigned and dead at the end of the `if` branch, and is not live on the path where `c` is false

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

### Requirement: Assignments Drop the Old Value
An assignment to a place whose type needs a drop MUST evaluate the new value first, then drop the place's old value, then write the new value.

#### Scenario: Writing through a reference drops the old referent
- **WHEN** `fun f(r: &mut String) { *r = "b" }` is lowered
- **THEN** the dereference of `r` is dropped and then assigned the string `"b"`

#### Scenario: Assigning a variable to itself
- **WHEN** `fun f() { var s = "a"; s = s }` is lowered
- **THEN** `s` is moved into a temporary before its old value is dropped, and the temporary is then moved into `s`; in the elaborated MIR that drop is removed, because `s` has moved

### Requirement: Scope Exits Are Located Where Control Leaves the Scope
The drops and storage ends that a braced block runs when control reaches its end MUST carry the span of the block's closing brace, and those that a `return`, `break` or `continue` runs MUST carry the span of that jump. Other scopes MUST keep their own span.

#### Scenario: Block-end drops at the closing brace
- **WHEN** `fun f(c: Bool) { if (c) { val s = "a" } }` is lowered, and `fun g(c: Bool) { while (c) { val s = "a"; if (c) break } }`
- **THEN** in `f` the drop and the storage end of `s` are at the `}` that closes the `if` branch, and in `g` the drop of `s` on the path of the `break` is at the `break`, and on the other path at the `}` that closes the loop body

### Requirement: Drops Are Elaborated After Borrow Checking
When a file passes the borrow checker, each drop of its MIR MUST be elaborated: removed where the value was moved on every path reaching it, kept where it holds a value on every path, and otherwise guarded by a drop flag: a `Bool` local without storage markers, found only in elaborated MIR, that is true exactly when the value is present. A drop flag MUST be set where its value is assigned and cleared where the value moves and where its storage starts or ends.

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

### Requirement: Temporaries Live Until the End of Their Scope
A temporary MUST be dropped at the end of the innermost enclosing statement, block tail or body value, or of the innermost part that is evaluated conditionally or repeatedly: a condition, a `when` pattern test, a `for` range, a branch of `if` or `when`, the right operand of `&&` or `||`, the fallback of `?:`, or the rest of a `?.` chain after its first test.

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and a body runs `greet(make())` as a statement
- **THEN** the `String` returned by `make` is held in a temporary that `greet` borrows, and the temporary is dropped after `greet` returns, at the end of the statement

#### Scenario: Condition temporaries dropped before branching
- **WHEN** `fun check(s: &String): Bool = true`, `fun make(): String = "x"` and `fun g() {}` are declared and a body runs `if (check(make())) g()`
- **THEN** the temporary holding the result of `make` is dropped before the branch on the condition

#### Scenario: Branch temporaries dropped in their branch
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and `fun f(c: Bool): Int = if (c) greet(make()) else 0` is lowered
- **THEN** the temporary holding the result of `make` is created and dropped in the `then` branch, and the `else` branch drops nothing

#### Scenario: Tail temporaries dropped before the block's locals
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and `fun f(): Int { val s = "a"; greet(make()) }` is lowered
- **THEN** the temporary holding the result of `make` is dropped before `s` is dropped

### Requirement: Borrowed Branch Temporaries Live as Long as the Whole Value
A temporary whose written borrow is the value of a branch of `if` or `when`, of the fallback of `?:` or of a block tail, through any nesting of these, MUST belong to the scope of the whole expression whose value that borrow becomes, instead of the branch's: its storage MUST start before that expression is evaluated, and it MUST be dropped at the end of that scope, on the paths where it was created.

#### Scenario: A borrowed fallback temporary lives until the end of the statement
- **WHEN** `fun greet(n: &String): Int = 1` is declared and `fun f(n: String?): Int = greet(&n ?: &"anon")` is lowered
- **THEN** the storage of the temporary holding `"anon"` starts before the tag of `n` is read, the temporary is assigned only on the edge where `n` is `null`, and it is dropped after `greet` returns; in the elaborated MIR that drop is guarded by a drop flag

### Requirement: Borrowed Initializer Temporaries Live as Long as Their Binding
When the initializer of a `val` or `var` is a written borrow of a place rooted at a temporary, or an expression whose value is such a borrow in a branch of `if` or `when`, the fallback of `?:` or a block tail (the temporaries of the previous requirement), each such temporary MUST belong to the binding's block and be dropped just after the binding, instead of at the end of the declaration.

#### Scenario: Borrowed temporary in a val initializer
- **WHEN** `fun make(): String = "x"` and `fun greet(n: &String): Int = 1` are declared and `fun f(): Int { val r = &make(); return greet(r) }` is lowered
- **THEN** the temporary holding the result of `make` is still live when `greet` is called, and is dropped when the block ends

#### Scenario: Borrowed temporaries in the branches of a val initializer
- **WHEN** `fun make(): String = "x"` and `fun greet(n: &String): Int = 1` are declared and `fun f(c: Bool, s: String): Int { val r = if (c) &make() else &s; return greet(r) }` is lowered
- **THEN** the storage of the temporary holding the result of `make` starts before `c` is tested, it is assigned only in the `then` branch, and it is still live when `greet` is called and dropped when the block ends, under a drop flag in the elaborated MIR

### Requirement: Conditional Operators Become Branches
`if`, `&&`, `||` and `?:` MUST become branches that evaluate only the operands and branches their values need. `?:` MUST read the tag of its nullable operand and, when it is present, move or copy its payload out.

#### Scenario: Logical and evaluates its right operand only when needed
- **WHEN** `fun g(): Bool = true` is declared and `fun f(a: Bool): Bool = a && g()` is lowered
- **THEN** the body branches on `a`, calls `g` only on the edge where `a` is true, and gives `false` on the other edge

#### Scenario: Elvis branches on the tag
- **WHEN** `fun f(n: String?): String = n ?: "anon"` is lowered
- **THEN** `n` is moved into a temporary whose discriminant is read; where it is present its payload is moved into the return place, and where it is `null` the string `"anon"` is

### Requirement: Safe Navigation Tests Each Payload
A `?.` chain MUST read the tag of each nullable it looks into, give `null` as soon as one is `null`, dropping the temporaries the chain created so far, and otherwise read the payload where it lives.

#### Scenario: Safe-navigation chain over a place
- **WHEN** `struct Team { val lead: User? }`, `struct User { val name: String }` and `fun User.greet(): Int = 1` are declared and `fun f(t: Team?): Int? = t?.lead?.greet()` is lowered
- **THEN** the tag of `t` and then the tag of the `lead` field of its payload are read, either `null` gives `null`, and `greet` is called with a shared receiver borrow of the payload of that `lead` field, whose result is wrapped as non-null

#### Scenario: A safe call on null evaluates no argument
- **WHEN** `struct User { val name: String }`, `fun next(): Int = 1` and `fun User.greet(n: Int): Int = n` are declared and `fun f(u: User?): Int? = u?.greet(next())` is lowered
- **THEN** `next` is called only on the edge where `u` is present

#### Scenario: A wrapped chain value is moved, not dropped
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User?): String? = u?.name?.clone()` is lowered
- **THEN** the result of `clone` is held in a temporary that is moved, wrapped, into the return place, and that temporary is dropped on neither path

### Requirement: When Branches Test Patterns in Order
A `when` MUST evaluate its subject once. A `when` whose patterns are all payload-free variants or `else` MUST switch on the subject's discriminant; any other `when` MUST test its patterns in order. When no pattern matches, an exhaustive `when` without `else` MUST reach an unreachable terminator.

#### Scenario: When over variants switches on the discriminant
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color): Int = when (c) { Red -> 1; Green -> 2 }` is lowered
- **THEN** the discriminant of `c` is read once and switched on, `0` leading to the branch giving `1`, `1` to the branch giving `2`, and any other value to an unreachable terminator

#### Scenario: When patterns tested in order
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 1; in 1..10 -> 2; else -> 3 }` is lowered
- **THEN** `x` is first compared with `0`, then tested against `1` and `10` only when it is not `0`, and the `else` branch is reached when both tests fail

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

### Requirement: Diverging Code Ends Its Block
Code after an expression of type `Nothing` MUST NOT be lowered, and every block of a body MUST be reachable from its entry. A call whose callee returns `Nothing` MUST have no successor block.

#### Scenario: Code after return is not lowered
- **WHEN** `fun g() {}` is declared and `fun f() { return; g() }` is lowered
- **THEN** the body of `f` has no call of `g`

### Requirement: Calls Name Their Callee
A call MUST name its callee (a declaration path for an own or imported function or extension, the built-in `clone`, a function-typed place, or a default value's generator), pass one operand per parameter in parameter order, the receiver first, and be marked composable when its callee is composable. An extension's declaration path names its receiver type, so the first operand of a call of an extension, and the first argument of an extension's body, MUST be its receiver; no other call has one.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public fun greet(n: Int): Int = n`, and a file importing `/models/user` declares `fun hello(): Int = greet(1)`
- **THEN** the MIR of `hello` calls `models/user::greet` with the constant `1`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the body of `Screen` is composable, its call of `Label` is marked composable, and its call of `log` is not

#### Scenario: An extension call names its receiver
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }` and `public fun Team.captain(o: &String): &String = &lead`, and a file importing `/lib/team` declares `fun f(t: Team, s: String): Int { val c = t.captain(&s); return 1 }`
- **THEN** the MIR of `f` calls `lib/team::Team.captain`, whose path names the receiver type `Team`, with the receiver borrow of `t` first and the borrow of `s` second

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

### Requirement: Safe-Call Receivers Are Borrowed at Their Test
A borrowed receiver that a `?.` test reaches (the payload of a nullable) MUST be borrowed right after its test, before the written arguments, so that no argument can change the tested value while the receiver is reached through it.

#### Scenario: A safe-call receiver is borrowed after its test
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}` and `fun next(): Int = 1` are declared and `fun f(s: Stack?) { var t = s; t?.push(next()) }` is lowered
- **THEN** on the edge where `t` is present, the mutable receiver borrow of the payload of `t` is taken before `next` is called, and `push` is called with that borrow and the result of `next`

### Requirement: Operands Are Evaluated in Order
The operands of an operator, a range and a construction MUST be evaluated in source order, and the subject of a `when` once. An operand that reads a place which a later operand, or a later pattern of the `when`, may write MUST hold the value the place had when the operand was evaluated.

#### Scenario: An operand read before a later write
- **WHEN** `fun inc(r: &mut Int): Int { *r = r + 1; return r }` is declared and `fun f(): Int { var n = 1; return n + inc(&mut n) }` is lowered, or `struct Pair { val a: Int; val b: Int }` is declared and `fun g(): Pair { var n = 1; return Pair(n, inc(&mut n)) }`, or `fun k(): Int { var n = 0; return when (n) { inc(&mut n) -> 1; else -> 2 } }`
- **THEN** `n` is copied into a temporary before `inc` is called, and the sum, the construction or the comparison reads that temporary

### Requirement: Default Values Are Generator Bodies
Each parameter's default value MUST be lowered to its own body, which takes a shared borrow of the receiver and of each earlier argument and returns the parameter's value. A call omitting that argument MUST call the generator with borrows of its own earlier arguments.

#### Scenario: Default reading an earlier parameter
- **WHEN** `fun f(a: Int, b: Int = a + 1): Int = b` is declared and `fun g(): Int = f(1)` is lowered
- **THEN** the generator of `b` takes a `&Int` and returns its dereference plus `1`, and `g` holds `1` in a temporary, calls the generator with a shared borrow of it, and calls `f` with both values

### Requirement: Borrowed Default Temporaries Live in the Caller
The generator of a parameter of reference type MUST also take a mutable borrow of a slot of the referent's nullable type, `null` on entry, created by the caller and dropped at the end of the caller's statement. A default that borrows a temporary MUST store the temporary in the slot and return a borrow of it there.

#### Scenario: Default borrowing a temporary
- **WHEN** `fun greet(n: &String = "anon"): Int = 1` is declared and `fun f(): Int = greet()` is lowered
- **THEN** the generator of `n` stores `"anon"` in its slot and returns a borrow of the slot's payload, and `f` creates a `null` slot of type `String?`, passes a mutable borrow of it to the generator, calls `greet` with the result, and drops the slot afterwards

### Requirement: Optional Mutable Borrows Are Reborrowed
Where a `(&mut T)?` place is lent as a `(&T)?`, the MIR MUST read its tag and pass `null`, or a shared borrow of the dereferenced payload wrapped as non-null, without moving the place.

#### Scenario: Lending an optional mutable borrow twice
- **WHEN** `struct User { var name: String }` and `fun show(n: (&User)?): Int = 1` are declared and `fun f(m: (&mut User)?) { show(m); show(m) }` is lowered
- **THEN** before each call the tag of `m` is read, and `show` receives either `null` or a wrapped shared argument borrow of the dereference of `m`'s payload; `m` is never moved

### Requirement: Optional Mutable Borrows Are Reborrowed as Mutable
Where a `(&mut T)?` place is lent as a `(&mut T)?`, the MIR MUST read its tag and pass `null`, or a mutable borrow of the dereferenced payload wrapped as non-null, without moving the place.

#### Scenario: Lending an optional mutable borrow as mutable twice
- **WHEN** `struct User { var name: String }` and `fun edit(n: (&mut User)?) {}` are declared and `fun f(m: (&mut User)?) { edit(m); edit(m) }` is lowered
- **THEN** before each call the tag of `m` is read, and `edit` receives either `null` or a wrapped mutable argument borrow of the dereference of `m`'s payload; `m` is never moved

### Requirement: Closures Are Separate Bodies
Each closure MUST be lowered to its own body whose captured bindings are capture locals following its arguments, and the enclosing body MUST build the closure value from the places of those bindings, in capture order. How each binding is captured MUST NOT be decided by the MIR as lowered: the borrow checker infers it, and the elaborated MIR MUST record exactly one mode per capture local, shared or mutable (see `references`), and MIR as lowered none.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int, b: Int): Int = run { b + a + b }` is lowered
- **THEN** the closure's body has the capture locals `b` and `a` and adds them, and the body of `g` builds the closure value from its arguments `b` and `a` and passes it to `run`

#### Scenario: Capture modes in elaborated MIR
- **WHEN** `struct User { var name: String }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(n: Int) { var u = User("a"); run { u.name = "z"; n } }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure value from a mutable borrow of `u` and a shared borrow of `n`, while the MIR as lowered records no mode

### Requirement: Comparisons Borrow Non-Copy Operands
`==` and `!=` over a type that is not `Copy` MUST compare shared borrows of their operands, nullable operands as optional borrows, without moving or copying them. Any other operator over a type that is not `Copy` (string concatenation) MUST likewise read shared borrows of its operands. A comparison with `null` MUST read the tag.

#### Scenario: String equality borrows its operands
- **WHEN** `fun f(s: String): Bool = s == "a"` is lowered
- **THEN** the comparison is between a shared borrow of `s` and a shared borrow of a temporary holding `"a"`, and `s` is not moved

#### Scenario: Comparing a value with a nullable one
- **WHEN** `fun f(a: String, b: String?): Bool = a == b` is lowered, or `fun g(x: String?, y: String): Int = when (x) { y -> 1; else -> 2 }`
- **THEN** the nullable operand is viewed through a test of its tag as an optional shared borrow, the other is borrowed where it lives and wrapped as non-null without a tag test, and neither is moved

#### Scenario: String concatenation borrows its operands
- **WHEN** `fun f(a: String, b: String): String = a + b` is lowered
- **THEN** the return place is assigned the sum of a shared borrow of `a` and a shared borrow of `b`, and neither is moved

#### Scenario: Comparison with null reads the tag
- **WHEN** `fun f(s: String?): Bool = s == null` is lowered
- **THEN** the discriminant of `s` is read and compared with the discriminant of `null`

### Requirement: MIR Is Well Formed
The MIR of a file without type errors MUST satisfy its structural invariants: every block ends in a terminator and is reachable, every local used is declared, every assignment's value has its place's type except for an explicit subtyping conversion, projections and call operands are well typed, no operand moves a place through a dereference or a place rooted at a capture local, only values whose type needs a drop are dropped, and every local is storage-live wherever it is used.

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

### Requirement: Generic Bodies Are Lowered Once
A generic declaration's body, and its default values and closures, MUST be lowered once, recording the declaration's type parameters, with locals typed by them. A call of a generic callee MUST record its type arguments and the callee's declared parameter types. A value of a type parameter that is not `Copy` MUST be dropped like any value that needs a drop, and one bounded by `Copy` MUST NOT be dropped.

#### Scenario: Dropping a type-parameter value
- **WHEN** `fun <T> discard(x: T) {}` and `fun <T : Copy> keep(x: T): Int = 1` are lowered
- **THEN** the body of `discard` records the type parameter `T` and drops its argument when it returns, and the body of `keep` has no drop

#### Scenario: A call with type arguments
- **WHEN** `fun <T> id(x: T): T = x` is declared and `fun f(s: String): String = id(s)` is lowered
- **THEN** the MIR of `f` calls `main::id` with the type argument `String`, the declared parameter type `T` and a move of `s`, and `prism dump-mir` prints the callee as `main::id<String>`

#### Scenario: A default value of a generic function
- **WHEN** `fun <T : Copy> f(x: T, y: T = x): T = y` is declared and `fun g(): Int = f(1)` is lowered
- **THEN** the generator of `y` records the type parameter `T : Copy`, and the MIR of `g` calls it with the type argument `Int`, which `prism dump-mir` prints as `default main::f<Int> y`

#### Scenario: Elaborated drops of a generic body
- **WHEN** `fun <T> pass(x: T): T = x` and `fun <T> pick(a: T, b: T, c: Bool): T = if (c) a else b` are lowered and elaborated
- **THEN** `pass` drops nothing, since `x` is moved into the return place, and `pick` drops the argument it did not return, under a drop flag or on each branch

## Design Rationale
Decisions from `mir-lowering` (the shape agreed with the user in the borrow-checker roadmap exploration, 2026-10-17: a `prism-mir` crate lowering the typed tree to a place-based control-flow graph, with `prism dump-mir` for snapshot tests; the rest decided while proposing, implementing and reviewing the change and logged in `docs/decisions-for-review.md`). The consumers are the borrow checker (`borrowck-moves`, `borrowck-exclusivity`, `borrowck-elision`), drop elaboration, closure conversion, the Cranelift backend, `hot-reload` and `compose-transform`.

- **Only error-free trees are lowered** (`prism_mir::lower_file`). The tree's error-free invariants are the lowering's preconditions, and a violation is an internal compiler error, not a diagnostic: no consumer needs MIR for a broken file (the LSP reads the typed tree), and lowering error nodes would need a recovery story. `prism check` lowers every file that typechecks and discards the MIR, so every checked program exercises lowering and the borrow checker only has to be added to the pipeline.
- **A small, Rust-like MIR.** Places are a local plus projections (`Deref` of `&`, `&mut` or `Heap<T>`, `Field` by index, nullable `Payload`, `Index` by an `Int` local); operands move, copy or are constants; borrows are rvalues that keep the typed tree's origin (written, argument, receiver, conversion, operator, default); `Drop` and `Call` are terminators, so drop elaboration can turn a drop into a flag test or a call to drop glue without splitting blocks; `SwitchInt` branches on `Bool` and `Int`. `WrapNullable` is its own rvalue because it always changes the representation, and `Subtype` marks every other subtyping step, where the backend may need an adapter. Left out because Prism does not need them yet: unwinding edges (no unwinding panics), `Assert` (overflow semantics are undecided), casts, debuginfo scopes and promoted constants. Rejected: SSA, which the place-based borrow analyses would have to undo and Cranelift builds itself; a lowered tree with explicit drops, which leaves every dataflow analysis to rebuild the CFG.
- **Bodies and locals have symbolic identities.** One body per function, top-level `val`, parameter default and closure, ordered by item: a function, its closures, then each default generator and its closures. A closure is named by its parent body and its index there (`main::g#closure0`), so `hot-reload` and `compose-transform` can key bodies without global numbering. Locals are numbered as in Rust (`_0` the return place, then the arguments with `this` first, then captures, then bindings and temporaries in order of creation), and a local records the typed-tree binding and name it stands for, so diagnostics can say "use of moved value `s`".
- **Places come from typed-tree places.** A binding is its local (a generator's `*_i`, a closure's capture local); `Index` evaluates its index into a temporary first; a payload is reached through a tag test; a `Temporary` node is a temporary of the current temporary scope; a top-level `val` is `(*_t)` where `_t` holds a constant `&T`, as Rust lowers statics, so places stay local-rooted and `borrowck-elision` can recognize borrows of top-level values. A discarded place, or the subject of a `when` with only `else`, is a `Read` statement so the move checker sees the use. A string literal is a constant that evaluates to a fresh owned `String`.
- **Storage markers and drops are inserted at lowering time, unrefined by moves.** Lowering keeps a stack of scopes; where one ends, by falling through or by any jump out of it, each owned local is dropped (if its type needs a drop) and marked dead, in reverse creation order; arguments are dropped on return, after the locals. A type needs a drop unless it is `Copy`, a `&mut T`, or a nullable of these; whether a drop does anything is a layout question. A local moved into a call is still dropped at its scope's end: `borrowck-moves` computes maybe-moved places on this MIR, and drop elaboration turns each drop into nothing, a static drop or a flag-guarded one, as Rust's `ElaborateDrops` does. The one refinement is for temporaries lowering moves itself (call arguments, aggregate operands, assigned values, a wrapped `?.` value), which get no drop. An assignment to a place that needs a drop evaluates the value, drops the old value, then writes. Doing this at lowering is the only point where the scope structure exists, and the borrow checker must see drops as uses. Rejected: inserting drops after borrow checking (hides conflicts with live borrows) and running move analysis inside lowering (duplicates `borrowck-moves`).
- **Temporary scopes, as Rust's terminating scopes (with Rust 2024's tail rules).** A temporary lives until the end of its statement, block tail or body value, or of the conditionally or repeatedly evaluated part that creates it, so it is always created unconditionally within its scope and never needs a drop flag; a condition's temporaries are dropped before the branch, a tail's before the block's locals. A `when` subject and a chain's first nullable belong to the enclosing scope. A condition's `Bool` result lives in a scope of its own that the branch dissolves, marking it dead at the start of both successors. **Lifetime extension:** a `val` or `var` whose initializer is a written borrow of a place rooted at a temporary (`val r = &make()`, `val n = &make().name`) owns that temporary, created just before the binding; without this the code would always be a dangling borrow. Extension through branches and assignments is not done (possible relaxations). Rejected: one scope per statement with drop flags, and Rust 2021's tail rule that Rust itself reversed.
- **Control flow is destination-passing**, as rustc's `expr_into_dest`. `if`, `&&`, `||` and `?:` evaluate only what their value needs; `?:` moves or copies the payload of its consumed nullable out of a temporary that then needs no drop. A `?.` chain tests each payload, its null edge leaving the chain's scope like a `break`, and reads payloads where they live (`t?.lead?.greet()` borrows `((_1 as payload).lead as payload)`). A `when` over payload-free variants is one discriminant switch; any other `when` tests its patterns in order; an exhaustive `when` without `else` reaches `Unreachable`. A `for` evaluates its range once into a hidden local, exits at once when it is empty, computes the last element and compares with it in the latch before incrementing, so no addition overflows even for `0..Int.MAX`. Jumps exit every scope between them and their target. `Unit` destinations are written (`const ()`), so `borrowck-moves`' initialization analysis sees every `Unit` binding and return place initialized.
- **A `continue` in a `do`/`while` keeps the body's locals in scope.** The condition sees the body's top-level locals (the `loops` spec), so the `continue` path drops nothing of the body's scope and marks live, without a value, each body local declared after the `continue`. Reading one in the condition is a read of an uninitialized value, which the borrow checker rejects (`E5002`), as Kotlin does. Rejected: dropping the body's locals on that path (the condition still reads them) and a type error for every `continue` before a declaration.
- **Code after a diverging expression is not lowered**, so every block is reachable from the entry and code after `return` is not borrow-checked, as in Rust; a call returning `Nothing` has no successor. Blocks are created on demand, and a block that a `goto` is the only way into is merged into its predecessor.
- **Calls name their callee symbolically and evaluate in source order.** A callee is a declaration path (own or imported function or extension: the same path the ABI uses, so callers need only the ABI and `hot-reload` can key its tables), the built-in `clone`, a function-typed place, or a default generator. Operands are in parameter order with the parameter types repeated, so neither the validator nor the backend needs a signature. Order: a borrowed receiver's place (or a moved or copied receiver itself, see `borrowck-moves` below), the written arguments in source order (each in a temporary unless it is a constant no generator borrows), the borrowed receiver's borrow, the defaults in parameter order, the call. So in `v.push(v.size())` the `&mut v` is taken after `size` returns, and `borrowck-exclusivity` accepts it by liveness alone, with no special two-phase rule; a safe call's receiver is borrowed at its test instead (`borrowck-exclusivity` below). Composable calls and bodies stay marked for `compose-transform`.
- **Operands hold the value they had when evaluated.** An operator, range or construction collects its operands before its rvalue; an operand that reads a place which a later operand (or, for a `when`'s `Copy` subject, a later pattern) may write, because it calls a function or runs a statement, is first copied or moved into a temporary, as rustc's operands are, so `n + inc(&mut n)` adds the old `n`. Operands followed only by reads stay in place.
- **Default values are generator bodies** (Swift's default argument generators), owned by `Default { function, parameter }` in the callee's module, so a default may call the callee's private declarations. A generator takes a shared borrow of the receiver and of every earlier argument, written or defaulted, and returns the parameter's value; the signature depends only on the callee's signature, so a caller in another file derives it from the ABI and a change to a default's expression never changes a caller. A reference-typed parameter's generator (`&R`, `&mut R`, `(&R)?`, `(&mut R)?`) also takes a `&mut R?` slot that the caller creates as `null` in its statement's temporary scope: a default that borrows a temporary (`n: &String = "anon"`) stores it in the slot and returns a borrow of the slot's payload, so it lives until the end of the caller's statement, as an argument's temporary does. Rejected: recording in the ABI whether a default borrows a temporary (Phase 1 reads only the AST), Kotlin's `$default` wrapper with a bitmask (needs placeholder values for omitted reference parameters), inlining defaults at call sites (impossible across files), rejecting such defaults.
- **An optional mutable borrow lent as an optional shared borrow is reborrowed**: its tag is read, and `null` or a wrapped shared borrow of the dereferenced payload is passed, so `show(m); show(m)` leaves `m` usable, as the reborrow of a plain `&mut T` does.
- **Closures are separate bodies with capture locals.** A capture local stands for the captured binding itself (never marked or dropped by the closure), and the enclosing body builds the closure value from the captured places, so the MIR says what each closure does with each capture without deciding how it is captured: `borrowck-exclusivity` infers the modes and records them in elaborated MIR, and `closures` converts each closure into a function plus an environment. Rejected: an environment argument now, whose field types depend on the undecided modes.
- **Operators over non-`Copy` operands read borrows.** `==`, `!=` and string `+` over a type that is not `Copy` read shared borrows of their operands (`Operator` origin), never moving or copying them; a nullable operand is an optional borrow `(&T)?` (a place through a tag test, a non-nullable side wrapped without one, a `?.` chain ending in a place borrowed inside the chain), and a comparison with `null` reads the tag. The backend implements these operators by operand type.
- **The type table travels with the tree and the MIR** (see `typed-tree`), so lowering, the borrow checker and the backend know `Copy`-ness and field types without the checker. The MIR's copy gives the built-in `Range` a third field, `endInclusive`, which a construction sets and a `for` reads; the type system keeps exposing `start` and `end` only.
- **A stable text dump and a validator.** `prism dump-mir` prints each body with its locals and blocks, one statement or terminator per line with `@line:column`, and drives `mir.snap` fixtures, re-blessed with `PRISM_BLESS=1`. `MirFile::validate` checks reachability, declared locals, typed projections and rvalues, call operands and destinations, drops of droppable places only, captures never marked or dropped, and, by forward dataflow, that every local is storage-live wherever it is used and dead on every path to a return; it runs over the MIR of every passing fixture and example.

Decisions from `borrowck-moves`:

- **By-value receivers are evaluated before the arguments** (orchestrator). A `consuming fun` receiver, or a `Copy` receiver copied out of a reference, is moved or copied into a temporary first, strict left-to-right as in Kotlin and Rust, so `s.consume(f(&mut s))` is a use after move rather than a move of the `s` that `f` mutated. A borrowed receiver keeps the two-phase shape above, which makes `v.push(v.size())` acceptable by liveness alone. This replaces the `mir-lowering` order for by-value receivers.
- **Drop elaboration runs on MIR after the borrow checker, with drop flags as locals.** It needs exactly the borrow checker's move paths and initialization analyses, as Rust's `ElaborateDrops` reuses its `MoveData`, so it runs when a file passes and the MIR becomes `elaborated`. A drop of a whole local stays where it holds a value on every path, goes where it was moved on every path, and is otherwise guarded by a `Bool` flag local (no storage markers), set on entry for an argument, after each initialization, and cleared before each move and at each storage marker, so it is true exactly when the local holds a value. A drop of a field or through a reference stays: the use check proved what holds it initialized. `runtime-strings-and-drops` keeps drop glue, what a drop does. Rejected: elaborating in the backend change, which would gather the move data again; flags as storage-marked temporaries; open drops of nullables, unneeded once payload moves count as moves of the nullable.
- **Elaboration is verified on every passing fixture and example.** A dataflow over the elaborated MIR alone tracks what each local holds (nothing, a value, or `null`, which owns nothing) and its flag, and checks that no value is dropped twice, used after its drop, or lost to an assignment, the end of its storage or a return, and that each flag mirrors its local. It is the only check of elaboration before a backend runs the MIR.
- **Operands carry their spans.** An operand that reads a place, and a function-typed callee, records the span of the source that reads it, so borrow errors point at the operand rather than at its statement (`Pair(s, s)`). A temporary per operand, as rustc does, was rejected: it would copy every `Copy` operand of an operator first. Writes are at their target's span, so a `Copy` write to a field or through a reference has a statement of its own.
- **An assignment moves a named value into a temporary before dropping the old one**, so `s = s` cannot drop the value it is about to move.

Decisions from `borrowck-exclusivity`:

- **Capture modes live on the closure body, in elaborated MIR.** `Body::capture_modes` holds one `CaptureMode` (`Shared` or `Mutable`) per capture local, empty as lowered and set by the borrow checker, and `prism dump-mir --elaborated` prints a closure value's captures with their modes (`closure main::f#closure0 [&mut _1, &_2]`), for `closures` to lay out environments. Rejected: recording the modes on `Rvalue::Closure`, the parent's rvalue, while a mode belongs to the body that defines it. The validator checks one mode per capture in elaborated MIR, none as lowered, and that no operand moves a place rooted at a capture local, which the type checker's rule on moving captures guarantees (`ownership`). A capture is never dropped as a whole, though a field of it may be when it is assigned.
- **Scope exits are located where control leaves.** The parser records a block's closing brace, the typed tree keeps it, and a block's drops and storage ends at its end carry the brace's span, those of a `return`, `break` or `continue` the jump's, so "dropped here while still borrowed" and the backend's debug information point where the drop runs. An unbraced block keeps its whole span, and a `do`/`while` body's locals, which its condition sees, drop at the body's brace on both edges. Rejected: the opening brace; recomputing positions from offsets in the CLI.
- **Borrowed temporaries in branch values live as long as the value.** A temporary whose written borrow is the value of a branch of `if` or `when`, a `?:` fallback or a block tail, through any nesting, belongs to the scope of the whole expression: the statement's, or the binding's block for a `val` or `var` initializer, extending `mir-lowering`'s lifetime extension through branches as Rust 1.79 does. Its storage starts before the expression, it is assigned in its branch, and it is dropped under a drop flag. Without it, `greet(&u.nick ?: &"anon")`, which `references` accepts, dropped `"anon"` while still borrowed. Rejected: rejecting the idiom; extending every branch temporary.
- **A straight-line write of a whole local is located at the target.** An assignment to a local that needs no drop carries the target's span, like other assignments, so `x = 2` while borrowed points at `x`.
- **A safe call borrows its receiver at its test** (review, autonomous). The receiver of a safe call is a payload reached through a tag test, so it is borrowed right after that test rather than after the arguments, which could otherwise make the tested nullable `null` before the call. Rejected: re-testing the tag after the arguments, which would evaluate the arguments of a call it then skips.

Decisions from `borrowck-elision`:

- **The receiver of a call comes from its callee's path.** A declaration path names an extension's receiver type, and an extension cannot be named as a value, so a call's first operand is a receiver exactly when its callee is an extension's path (or the built-in `clone`), and a body's first argument exactly when it is an extension's body; the borrow checker's rule 2 reads that. Rejected: a receiver flag on calls, which duplicates the path and needs a validator rule; the first parameter's type (a `&T` first parameter is not necessarily a receiver).
- **A `(&mut T)?` lent as a `(&mut T)?` is reborrowed** (orchestrator). Like the `(&T)?` case, the lowering tests the tag and passes `null` or a wrapped mutable reborrow of the payload's referent, so the binding stays usable, as a `&mut` binding passed as an argument does, and passing it twice to one call is a conflict. Rejected: moving it (unusable after one call); a typed-tree coercion for one case.

Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Generic bodies are lowered once, and drops of type parameters stay in.** A body records its owner's type parameters, its default generators and closures included, and a value of a non-`Copy` type parameter "maybe needs drop": it is elaborated as usual and monomorphization resolves it to the instance's drop glue or removes it. A `T : Copy` value is never dropped. Calls, default generator calls and aggregates (`struct m::Box<T>(...)`, `variant m::Opt.Some<T>(...)`) record their type arguments, so a body stays self-describing and the validator checks operands against substituted types. Rejected: reading aggregate arguments from the assigned place's type, which hides them from anything reading the rvalue alone.
