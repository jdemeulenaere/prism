# Typed Tree Specification Delta

## Purpose

The typed tree is the type checker's output for one file: every name resolved, every node typed, every call normalized and every implicit operation explicit, so that MIR lowering, borrow checking, code generation and the `compose fun` transform never re-derive a type-checking decision.

## ADDED Requirements

### Requirement: A Typed Tree for Every Checked File
Checking a file MUST produce a typed tree holding the file's declarations and one body per function and top-level `val`; a function's body also holds its parameters' default values. Every expression node MUST carry a type and the span of the source it stands for. The tree MUST be produced even when the file has type errors.

#### Scenario: Every node has a type and a span
- **WHEN** `fun f(a: Int): Int = a + 1` is checked
- **THEN** its typed tree holds a body for `f` whose value is an addition of type `Int` over the parameter `a` and the literal `1`, each node with type `Int` and the line and column where its source starts

#### Scenario: Tree for a file with type errors
- **WHEN** `fun f(): Int = nope + 1` is checked
- **THEN** the type checker emits `E4001`, and the typed tree still holds the body of `f`, with an error node of the error type for `nope`

#### Scenario: Default values belong to their function's body
- **WHEN** `fun f(a: Int, b: Int = a + 1): Int = b` is checked
- **THEN** the body of `f` holds the default value of `b`, an addition that reads the parameter binding `a`

### Requirement: No Error Nodes Without Type Errors
Where checking fails, the typed tree MUST hold an error node of the error type that keeps the nodes of the subexpressions that were checked. A file that typechecks successfully MUST have a typed tree with no error node and no node of the error type.

#### Scenario: Error node keeps what was checked
- **WHEN** `fun g(a: Int): Int = a` is declared and `fun f(): Int = g(1, 2)` is checked
- **THEN** the type checker emits `E4004`, and the call is an error node of the error type

#### Scenario: Error-free trees
- **WHEN** a file typechecks successfully
- **THEN** its typed tree contains no error node and no node of the error type

### Requirement: Locals Are Resolved to Bindings
Every parameter, local `val` or `var`, `for` variable, closure parameter and extension receiver (`this`) MUST be a binding of its body, with its name, type and mutability, and every use of a name that resolves to one MUST refer to that binding. Two declarations with the same name MUST be distinct bindings.

#### Scenario: Shadowing declarations are distinct bindings
- **WHEN** `fun f(a: Int): Int { val a = 2; return a }` is checked
- **THEN** the body of `f` has two bindings named `a`, the parameter and the local, and `return` reads the local

### Requirement: Declarations Are Resolved to Module-Qualified Paths
Every use of a top-level function, extension or `val`, of the file or of a direct import, MUST name the declaration by its path: the defining module, the receiver type for an extension, and the name. Every field MUST be named by its index in its struct, and every enum variant by its index in its enum.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public struct User { val name: String }` and `public fun greet(u: &User): String`, and a file importing `/models/user` declares `fun hello(u: &User): String = greet(u)`
- **THEN** the call in `hello` names its callee `models/user::greet`

#### Scenario: Extension named by its receiver type
- **WHEN** `struct User { val name: String }` and `fun User.shout(): String = name + "!"` are declared and `fun f(u: User): String = u.shout()` is checked
- **THEN** the call names its callee by the file's module, the receiver type `User` and the name `shout`, and the body of `shout` reads field 0 (`name`) of the dereferenced `this` binding

### Requirement: Implicit Conversions Are Explicit
Where a value fits its position only through assignability, as a `T` given for a `T?`, a `Nothing` value, or a `&mut T` given for a `&T`, the typed tree MUST wrap it in a coercion node of the type the position expects. In a tree without type errors, every node's type MUST equal the type its position expects.

#### Scenario: Value wrapped into a nullable
- **WHEN** a body runs `val x: Int? = 5`
- **THEN** the initializer is a coercion to `Int?` around the literal `5` of type `Int`

#### Scenario: Branches coerced to their join
- **WHEN** a body of `fun f(c: Bool): Int` runs `val y = if (c) 1 else null` and `val z: Int = if (c) 1 else return 0`
- **THEN** the binding `y` has type `Int?`, both branches of its `if` are coercions to `Int?`, and the `else` branch of the second `if` is a coercion of the `Nothing`-typed `return` to `Int`

### Requirement: Consumed Places Are Explicit Moves or Copies
Bindings, top-level values, field paths, dereferences, indices, payloads of nullables and temporaries MUST be place nodes. A place whose value is consumed (the `ownership` spec, "Consuming Positions") MUST be wrapped in a move, or in a copy when its type is `Copy`. A place used anywhere else MUST appear unwrapped, which means it is read in place.

#### Scenario: Move and copy of locals
- **WHEN** `fun f(s: String, n: Int) { val a = s; val b = n }` is checked
- **THEN** the initializer of `a` is a move of the binding `s`, and the initializer of `b` is a copy of the binding `n`

#### Scenario: Operands are read in place
- **WHEN** `fun f(s: String): Bool = s == "a"` is checked
- **THEN** the left operand of `==` is the binding `s` itself, neither moved nor copied

#### Scenario: Copy-out is a copy of a dereference
- **WHEN** `fun f(a: &Int): Int = a` is checked
- **THEN** the body's value is a copy of the dereference of the binding `a`

#### Scenario: Field access through references dereferences explicitly
- **WHEN** `struct User { val id: Int }` is declared and `fun f(u: &&User): Int = u.id` is checked
- **THEN** the body's value is a copy of field 0 of two dereferences of the binding `u`

### Requirement: Borrows Are Explicit
Every borrow MUST be a node over a place, recording whether it is shared or mutable and whether it was written (`&x`), implied by a reference parameter, implied by a method receiver, or made by a nullable conversion. A borrowed value that is not a place MUST be held in a temporary place, which lives until the end of the enclosing statement. Passing a `&mut` binding MUST be a mutable borrow of its dereference (a reborrow).

#### Scenario: Implicit argument borrow of a place
- **WHEN** `fun greet(n: &String): Int = 1` is declared and `fun f(name: String): Int = greet(name)` is checked
- **THEN** the argument is a shared argument borrow of the binding `name`

#### Scenario: Implicit argument borrow of a temporary
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and `fun f(): Int = greet(make())` is checked
- **THEN** the argument is a shared argument borrow of a temporary holding the call of `make`

#### Scenario: Passing a mutable borrow reborrows it
- **WHEN** `struct User { var name: String }` and `fun touch(u: &mut User) {}` are declared and `fun f(m: &mut User) { touch(m); touch(m) }` is checked
- **THEN** each argument is a mutable argument borrow of the dereference of the binding `m`, and neither is a move

#### Scenario: Written borrow
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(u: User)` runs `val r = &u.name`
- **THEN** the initializer of `r` is a written shared borrow of field 0 of the binding `u`

### Requirement: Nullable Conversions Are Safe-Navigation Chains
A safe navigation MUST be a chain node whose body reads the payload of a nullable and which gives `null` when the nullable is `null`. A `?.` whose target is itself a safe navigation MUST join its chain, so that the payload it reads stays a place. Reading `&(T?)` as `(&T)?`, lending a `T?` place as `(&T)?`, and reading `(&T)?` as `T?` MUST each be such a chain, marked with the conversion it makes.

#### Scenario: Safe-navigation chain over a place
- **WHEN** `struct Team { val lead: User? }`, `struct User { val name: String }` and `fun User.greet(): Int = 1` are declared and `fun f(t: Team?): Int? = t?.lead?.greet()` is checked
- **THEN** the body's value is one chain whose body is the call of `greet` with a receiver borrow of the payload of field 0 of the payload of the binding `t`

#### Scenario: Lending a nullable place as an optional borrow
- **WHEN** `struct User { val nick: String? }` and `fun show(n: (&String)?): Int = 1` are declared and `fun f(u: User): Int = show(u.nick)` is checked
- **THEN** the argument is a chain marked as lending its contents, whose body is a shared borrow of the payload of field 0 of the binding `u`

#### Scenario: Copying out of an optional borrow
- **WHEN** `fun f(r: (&Int)?): Int? = r` is checked
- **THEN** the body's value is a chain marked as a copy-out, whose body is a copy of the dereference of the payload of the binding `r`

### Requirement: Calls Are Normalized
Every call MUST name its callee: a declaration path, a built-in function, or a function-typed value. Its arguments MUST be in parameter order, one per parameter, with an explicit default slot for an omitted parameter, and the call MUST record the source order in which its written arguments are evaluated and whether its callee is composable. Struct and variant constructions MUST be construction nodes with their fields in declaration order.

#### Scenario: Named arguments in parameter order
- **WHEN** `fun g(): Int = 1`, `fun h(): Int = 2` and `fun f(a: Int, b: Int = 2, c: Int = 3): Int = a` are declared and `fun k(): Int = f(c = g(), a = h())` is checked
- **THEN** the call's arguments are, in order, the call of `h`, a default slot, and the call of `g`, and its evaluation order is `c` then `a`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the call of `Label` in `Screen` is marked composable, and the call of `log` is not

#### Scenario: Constructor fields in declaration order
- **WHEN** `struct User { val id: Int, val name: String }` is declared and `fun f(n: String): User = User(name = n, id = 1)` is checked
- **THEN** the body's value is a construction of `User` whose fields are the literal `1` and a move of `n`, in that order, with the evaluation order `name` then `id`

### Requirement: Receivers Are Explicit First Arguments
An extension call MUST be a call whose first argument is the receiver, passed as its mode decides: a shared or mutable receiver borrow of the place reached through every looked-through `&`, `&mut` and `Heap<T>` layer, or of a temporary; a move of a whole receiver; or a copy of the dereferenced receiver for a `Copy` receiver of a `consuming fun` reached through borrows.

#### Scenario: Mutable receiver borrow
- **WHEN** `struct Stack { var size: Int }` and `mut fun Stack.push(n: Int) { size = size + n }` are declared and a body runs `var v = Stack(0); v.push(1)`
- **THEN** the call's first argument is a mutable receiver borrow of the binding `v`, and its second the literal `1`

#### Scenario: Receivers are dereferenced to the extension's type
- **WHEN** `struct User { val name: String }` and `fun User.greet(): Int = 1` are declared and `fun f(r: &&User): Int = r.greet()` is checked
- **THEN** the call's first argument is a shared receiver borrow of two dereferences of the binding `r`

#### Scenario: Consuming receiver moves it
- **WHEN** `struct User { val name: String }`, `struct Account { val owner: User }` and `consuming fun User.toAccount(): Account = Account(this)` are declared and `fun f(u: User): Account = u.toAccount()` is checked
- **THEN** the call's first argument is a move of the binding `u`, and the construction in `toAccount` holds a move of `this`

#### Scenario: A safe call evaluates its arguments inside the chain
- **WHEN** `struct User { val name: String }`, `fun next(): Int = 1` and `fun User.greet(n: Int): Int = n` are declared and `fun f(u: User?): Int? = u?.greet(next())` is checked
- **THEN** the body's value is a chain whose body is the call of `greet`, with a receiver borrow of the payload of `u` and the call of `next` as arguments

### Requirement: Built-In Operations Are Identified
The built-in `clone()` MUST be a call of the built-in clone function with a shared receiver borrow, never a call of a declaration. A range `a..b` or `a..<b` MUST be a range node of the built-in `Range` type that records whether its end is included.

#### Scenario: Built-in clone
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the body's value is a call of the built-in clone function whose receiver is a shared receiver borrow of field 0 of the dereferenced binding `u`

#### Scenario: Half-open range
- **WHEN** a body of `fun f(n: Int)` runs `for (i in 0..<n) {}`
- **THEN** the loop binds `i` and iterates a range node of type `Range` from the literal `0` to the binding `n`, with its end excluded

### Requirement: Control Flow Stays Structured
Blocks MUST keep their statements and tail value, and a block whose value is discarded MUST hold its last expression as a statement. `if`, `when`, `&&`, `||` and `?:` MUST stay structured nodes; a `when` MUST read its subject in place, resolve each pattern to a variant, a value, a range or a condition, and record whether it is exhaustive.

#### Scenario: A discarded tail becomes a statement
- **WHEN** `fun g(): Int = 1` is declared and `fun f() { g() }` is checked
- **THEN** the block of `f` has the call of `g` as an expression statement and no tail, and has type `Unit`

#### Scenario: When patterns resolved to variants
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color): Int = when (c) { Red -> 1; Green -> 2 }` is checked
- **THEN** the `when` reads the binding `c` in place, its patterns are variants 0 and 1 of `Color`, and it is exhaustive

### Requirement: Loops and Jumps Are Resolved
`while`, `do`/`while` and `for` loops MUST be statements; a `for` loop MUST bind its variable as a binding of the body and hold its range. Every `break` and `continue` MUST name the loop it leaves or continues.

#### Scenario: Break names the innermost loop
- **WHEN** a body runs `for (i in 0..<3) { while (true) { break } }`
- **THEN** the `break` names the `while` statement, not the `for` statement

### Requirement: Assignments Are Statements Over Places
An assignment MUST be a statement whose target is a place: a binding (re-pointing it when it is a reference), a field path, or a dereference for a write through a reference. Its value is consumed.

#### Scenario: Write through a reference
- **WHEN** `fun f(r: &mut Int) { *r = 5 }` is checked
- **THEN** the assignment's target is the dereference of the binding `r` and its value the literal `5`

#### Scenario: Re-pointing a reference variable
- **WHEN** a body of `fun f(a: Int, b: Int)` runs `var r = &a; r = &b`
- **THEN** the assignment's target is the binding `r` and its value a written shared borrow of the binding `b`

### Requirement: Closures Record Their Captures
A closure MUST record its parameters as bindings, whether it is composable, and its captures: the bindings of enclosing scopes that its body uses, each once, in order of first use. How each capture is taken is not recorded.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and a body of `fun g(a: Int, b: Int): Int` runs `return run { b + a + b }`
- **THEN** the closure captures `b` then `a`, each once

#### Scenario: Nested closures capture through their parent
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int): Int = run { run { a } }` is checked
- **THEN** both closures capture `a`

### Requirement: Typed Tree Text Dump
`prism dump-typed-tree <file.prism>` MUST print the file's typed tree, checked against its direct imports' ABIs, as deterministic text: one node per line, children indented under their parent, each line giving the node's kind, its resolved names, its type and its line and column. With type errors it MUST print the tree, report the diagnostics, and exit with a failure status.

#### Scenario: Dumping a file
- **WHEN** `prism dump-typed-tree main.prism` runs twice on a file that typechecks successfully
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file with type errors
- **WHEN** `prism dump-typed-tree main.prism` runs on `fun f(): Int = nope + 1`
- **THEN** it prints the typed tree, reports `E4001`, and exits with a failure status

#### Scenario: Dumping a file with a syntax error
- **WHEN** `prism dump-typed-tree main.prism` runs on a file that does not parse
- **THEN** it reports the `E2xxx` error, prints no tree, and exits with a failure status
