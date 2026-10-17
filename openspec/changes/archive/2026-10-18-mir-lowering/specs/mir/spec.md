# MIR Specification Delta

## Purpose

The MIR is the control-flow graph of each body of a file that typechecks: typed locals, places, explicit storage, drops, branches and calls, from which the borrow checker and the code generator work without re-deriving any decision of the type checker.

## ADDED Requirements

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
A local or temporary whose type needs a drop (any type except `Copy` types, mutable borrows and nullables of these) MUST be dropped where its scope ends, in reverse order of creation, on every path leaving the scope; arguments MUST be dropped when the body returns, after its locals. A drop MUST NOT yet depend on whether the value was moved.

#### Scenario: Locals dropped in reverse order
- **WHEN** `fun f() { val a = "a"; val b = "b" }` is lowered
- **THEN** at the end of the block `b` is dropped, then `a`

#### Scenario: A moved value keeps its drop
- **WHEN** `fun take(s: String) {}` is declared and `fun f(s: String) { take(s) }` is lowered
- **THEN** `take` is called with a move of the argument `s`, and `s` is still dropped when `f` returns

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

### Requirement: Borrowed Initializer Temporaries Live as Long as Their Binding
When the initializer of a `val` or `var` is a written borrow of a place rooted at a temporary, that temporary MUST belong to the binding's block and be dropped just after the binding, instead of at the end of the declaration.

#### Scenario: Borrowed temporary in a val initializer
- **WHEN** `fun make(): String = "x"` and `fun greet(n: &String): Int = 1` are declared and `fun f(): Int { val r = &make(); return greet(r) }` is lowered
- **THEN** the temporary holding the result of `make` is still live when `greet` is called, and is dropped when the block ends

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
- **THEN** the path of the `continue` keeps `s`, marks `x` live without assigning it, and jumps to the condition, after which `x` and `s` end on both edges; reading the unassigned `x` there is left to the borrow checker's initialization analysis

### Requirement: Diverging Code Ends Its Block
Code after an expression of type `Nothing` MUST NOT be lowered, and every block of a body MUST be reachable from its entry. A call whose callee returns `Nothing` MUST have no successor block.

#### Scenario: Code after return is not lowered
- **WHEN** `fun g() {}` is declared and `fun f() { return; g() }` is lowered
- **THEN** the body of `f` has no call of `g`

### Requirement: Calls Name Their Callee
A call MUST name its callee (a declaration path for an own or imported function or extension, the built-in `clone`, a function-typed place, or a default value's generator), pass one operand per parameter in parameter order, the receiver first, and be marked composable when its callee is composable.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public fun greet(n: Int): Int = n`, and a file importing `/models/user` declares `fun hello(): Int = greet(1)`
- **THEN** the MIR of `hello` calls `models/user::greet` with the constant `1`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the body of `Screen` is composable, its call of `Label` is marked composable, and its call of `log` is not

### Requirement: Call Arguments Are Evaluated in Order
A call MUST evaluate the receiver's place first, then the written arguments in source order, then borrow or move the receiver, then evaluate the omitted arguments in parameter order, and then call.

#### Scenario: A mutable receiver is borrowed after the arguments
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}` and `fun Stack.size(): Int = count` are declared and a body runs `var v = Stack(0); v.push(v.size())`
- **THEN** `size` is called before the mutable receiver borrow of `v` is taken, and `push` is called with that borrow and the result of `size`

#### Scenario: Named arguments are evaluated as written
- **WHEN** `fun g(): Int = 1`, `fun h(): Int = 2` and `fun f(a: Int, b: Int = 2, c: Int = 3): Int = a` are declared and `fun k(): Int = f(c = g(), a = h())` is lowered
- **THEN** `g` is called before `h`, then the generator of `b`, and `f` is called with the results of `h`, the generator and `g`, in that order

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

### Requirement: Closures Are Separate Bodies
Each closure MUST be lowered to its own body whose captured bindings are capture locals following its arguments, and the enclosing body MUST build the closure value from the places of those bindings, in capture order. How each binding is captured MUST NOT be decided by the MIR.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int, b: Int): Int = run { b + a + b }` is lowered
- **THEN** the closure's body has the capture locals `b` and `a` and adds them, and the body of `g` builds the closure value from its arguments `b` and `a` and passes it to `run`

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
The MIR of a file without type errors MUST satisfy its structural invariants: every block ends in a terminator and is reachable, every local used is declared, every assignment's value has its place's type except for an explicit subtyping conversion, projections and call operands are well typed, only values whose type needs a drop are dropped, and every local is storage-live wherever it is used.

#### Scenario: Error-free MIR
- **WHEN** a file typechecks successfully
- **THEN** its MIR satisfies every structural invariant

### Requirement: MIR Text Dump
`prism dump-mir <file.prism>` MUST print the MIR of the file, checked against its direct imports' ABIs, as deterministic text: each body with its locals and its blocks, one statement or terminator per line with its line and column. With type errors it MUST report them, print no MIR, and exit with a failure status.

#### Scenario: Dumping a file
- **WHEN** `prism dump-mir main.prism` runs twice on a file that typechecks successfully
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file with type errors
- **WHEN** `prism dump-mir main.prism` runs on `fun f(): Int = nope + 1`
- **THEN** it reports `E4001`, prints no MIR, and exits with a failure status
