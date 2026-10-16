# Spec Delta: type-system

## ADDED Requirements

### Requirement: Canonical Nominal Type Identity
Every struct and enum MUST have a canonical identity made of its defining module (the root-relative file path without extension, such as `a/lib`) and its name. Two nominal types MUST be equal only when their identities are equal, regardless of how each type was written or obtained (annotation, constructor call, enum variant, imported signature).

#### Scenario: Same-named structs from different modules stay distinct
- **WHEN** module `a/lib` declares `public struct User { val name: String }` and `public struct Order { val user: User }`, module `b/lib` declares `public struct User { val id: Int }`, and a file importing both evaluates `o.user.id` for `o: Order`
- **THEN** `o.user` has type `a/lib`'s `User` and the type checker emits `E4006` for `id`

#### Scenario: Built-in Range distinct from a user struct named Range
- **WHEN** a file declares `struct Range { val a: Int, val b: String }` and checks `val r: Range = 1..10` inside a function body
- **THEN** the type checker emits `E4002`

#### Scenario: Struct and enum never equal
- **WHEN** a value of an enum type initializes a local annotated with a struct type, or the reverse
- **THEN** the type checker emits `E4002`, even when the two types share a name across different modules

#### Scenario: Annotation and constructor denote the same type
- **WHEN** `val a: User = User("x")` is declared and `a` is passed to `fun read(u: User): String`
- **THEN** the call typechecks successfully

### Requirement: Imported Signatures Keep Their Declared Types
A type in an imported signature MUST denote the same type the exporting file declared. Nominal names resolve in the exporter's context, never the importer's, and nested function, nullable and reference types MUST keep their structure.

#### Scenario: Higher-order parameter type across files
- **WHEN** an imported module declares `public fun hof(f: ((Int) -> Int) -> Int): Int` and the importer calls `hof { k -> k(1) }`
- **THEN** `k` has type `(Int) -> Int` and the call typechecks successfully

#### Scenario: Function type returning a nullable across files
- **WHEN** an imported module declares `public fun pick(f: (Int) -> Int?): Int` and the importer calls `pick { x -> null }`
- **THEN** the closure is checked against `(Int) -> Int?` and the call typechecks successfully

### Requirement: Operators Read Through References
Operator operands (arithmetic, comparison, equality, logical, unary and range bounds), and `if` and subjectless-`when` conditions, MUST be typed after peeling every `&` and `&mut` layer, so a borrowed value can be read by operators. Positions that expect a value of type `T` (arguments, initializers, returns, assignment values) MUST NOT accept a `&T`.

#### Scenario: Arithmetic on a borrowed integer
- **WHEN** `fun f(a: &Int): Int = a + 1` is checked
- **THEN** the function typechecks successfully

#### Scenario: Equality between a borrow and a value
- **WHEN** `fun f(a: &Int, b: Int): Bool = a == b` is checked
- **THEN** the function typechecks successfully

#### Scenario: Borrowed boolean as a condition
- **WHEN** `fun f(c: &Bool): Int = if (c) 1 else 2` is checked
- **THEN** the function typechecks successfully

#### Scenario: Borrow passed where a value is expected
- **WHEN** `fun g(x: Int): Int` is declared and `fun f(a: &Int): Int = g(a)` is checked
- **THEN** the type checker rejects the argument with `E4004`

### Requirement: Return and Argument Mismatch Codes
A value whose type is incompatible with the return type of its enclosing function or closure MUST be reported as `E4005`. A value incompatible with a parameter type (positional, named or trailing-lambda argument, struct constructor field, or enum payload) MUST be reported as `E4004`. Both codes MUST also apply inside an `if`/`when` branch or block's final expression producing that value.

#### Scenario: Returned value of the wrong type
- **WHEN** `fun f(): Int { return "s" }` or `fun f(): Int = "s"` is checked
- **THEN** the type checker emits `E4005`

#### Scenario: Returning a value from a Unit function
- **WHEN** `fun f() { return 5 }` is checked
- **THEN** the type checker emits `E4005`

#### Scenario: Mismatch inside a returned branch
- **WHEN** `fun f(c: Bool): Int { return if (c) "a" else 1 }` is checked
- **THEN** the type checker emits `E4005` for `"a"`

#### Scenario: Argument of the wrong type
- **WHEN** `fun f(a: Int): Int` is declared and `f("s")`, `f(a = "s")` or the struct constructor `P("s")` for `struct P { val x: Int }` is checked
- **THEN** the type checker emits `E4004`

#### Scenario: Other expectations keep the generic mismatch code
- **WHEN** `val x: Int = "s"` is checked inside a function body, or a default value `b: Int = "s"` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Type name in return position is not a mismatch
- **WHEN** `struct User { val name: String }` is declared and `fun f(): User = User` is checked
- **THEN** the type checker emits `E4002` stating that `User` is a type, not a value

### Requirement: Closure Parameter Inference
A closure parameter MUST get its type from the expected function type, or from its own type annotation if the grammar allows one. A closure parameter with neither MUST be rejected with a cannot-infer error (`E4013`) naming the parameter.

#### Scenario: Unannotated closure parameter without an expected type
- **WHEN** a body declares `val g = { x -> x + 1 }`
- **THEN** the type checker emits `E4013` naming the parameter `x`

#### Scenario: Calls through an uninferred closure are not silently accepted
- **WHEN** a body declares `val g = { x -> x + 1 }` followed by `val s: String = g("str")`
- **THEN** the type checker emits `E4013` for `x` and does not accept the program

#### Scenario: Closure against an expected function type still inferred
- **WHEN** a closure `{ x -> x + 1 }` is passed to a parameter of type `(Int) -> Int`
- **THEN** `x` has type `Int` and the closure typechecks successfully

### Requirement: Order-Independent Inference of Private Declarations
The inferred type of an unannotated private top-level function or variable MUST be the same wherever it is used in the file, regardless of declaration order. A declaration whose inferred type depends on itself MUST be rejected with a cannot-infer error (`E4013`) that asks for an explicit type.

#### Scenario: Use before an inferred function declaration
- **WHEN** `fun g(): String { val s: String = h(); return s }` precedes `fun h() = 42`
- **THEN** the type checker emits `E4002`, exactly as when the declarations are swapped

#### Scenario: Use before an inferred top-level value
- **WHEN** `fun g(): String = top` precedes `val top = 42`
- **THEN** the type checker emits `E4005`

#### Scenario: Recursive inferred function
- **WHEN** `fun fact(n: Int) = if (n < 1) 1 else n * fact(n - 1)` is checked
- **THEN** the type checker emits `E4013` for `fact`

#### Scenario: Errors in an inferred body reported once
- **WHEN** `fun h() = undefinedThing` is used by a function declared before it
- **THEN** the type checker emits `E4001` exactly once

### Requirement: Ambiguous Imported Names Rejected at Use
When two imported modules export the same name and a file uses that name (as a value, callee or type), the type checker MUST reject the use with an ambiguity error (`E4012`) that names both modules. Importing both modules without using the name MUST be accepted. A top-level declaration that has the same name as an imported symbol MUST still be rejected with `E4010`.

#### Scenario: Using a name exported by two imports
- **WHEN** `a/lib` exports `public fun helper(x: Int): Int`, `b/lib` exports `public fun helper(x: String): Int`, and a file importing both calls `helper("s")`
- **THEN** the type checker emits `E4012` naming `a/lib` and `b/lib`, and no other error for the call

#### Scenario: Unused colliding names accepted
- **WHEN** the same two modules are imported and `helper` is never referenced
- **THEN** the file typechecks successfully

#### Scenario: Ambiguous type name in an annotation
- **WHEN** both imported modules export `public struct User` and the file declares `fun f(u: User): Int`
- **THEN** the type checker emits `E4012` for the annotation

#### Scenario: Local declaration colliding with an import
- **WHEN** a file imports `a/lib` and also declares its own `fun helper(x: String): Int`
- **THEN** the type checker emits `E4010`

### Requirement: Discarded Block Values
The final expression of a block body whose value is discarded MUST be typechecked, but its type MUST be ignored. A block body's value is discarded when it is the body of a block-bodied function returning `Unit` (declared or implied), or the body of a closure whose expected return type is `Unit`. Expression bodies are never discarded.

#### Scenario: Unit function ending in a non-Unit call
- **WHEN** `fun g(): Int = 1` is declared and `fun f() { g() }` is checked
- **THEN** the function typechecks successfully

#### Scenario: Unit closure ending in a non-Unit expression
- **WHEN** `fun run(f: (Int) -> Unit)` is declared and `run { x -> x + 1 }` is checked
- **THEN** the call typechecks successfully

#### Scenario: Errors in a discarded expression still reported
- **WHEN** `fun f() { undefinedThing }` is checked
- **THEN** the type checker emits `E4001`

#### Scenario: Expression body of a Unit function still checked
- **WHEN** `fun g(): Int = 1` is declared and `fun f(): Unit = g()` is checked
- **THEN** the type checker emits `E4005`

### Requirement: Members of a Type Require Importing Its Module
A file MUST be able to receive and pass along a value whose nominal type is defined in a module it does not import directly. Reading or writing that value's fields, matching on its enum variants, or constructing it MUST be rejected with `E4014`, naming the type, its defining module and the import to add. The type checker MUST read only the ABIs of the file's direct imports.

#### Scenario: Field access on a type from an unimported module
- **WHEN** `shop/api` exports `public fun first(): Order`, where `Order` is declared in `shop/models`, and a file that imports only `/shop/api` evaluates `first().price`
- **THEN** the type checker emits `E4014` naming `Order`, `shop/models` and `import /shop/models`

#### Scenario: Passing the value along without touching its members
- **WHEN** the same file declares `fun keep(): Unit { send(first()) }`, where `send(o: Order)` is also exported by `shop/api`
- **THEN** the file typechecks successfully

#### Scenario: Importing the defining module
- **WHEN** the file also imports `/shop/models` and evaluates `first().price`
- **THEN** the access typechecks with the declared type of `price`

## MODIFIED Requirements

### Requirement: Parameterless Closures Are Checked as Closures
A closure without parameters, including a trailing lambda or brace argument, MUST be typechecked as a function value, never as an eagerly evaluated block. When checked against an expected function type, its body MUST be checked in that type's composable context. When the expected type is not a function type, the closure MUST be rejected with the mismatch code of the site where it is checked: `E4004` for an argument, `E4005` for a returned value and `E4002` elsewhere.

#### Scenario: Trailing lambda passed to a non-function parameter rejected
- **WHEN** checking `fun takesInt(n: Int) { }` and `fun f() { takesInt { 42 } }`
- **THEN** the type checker reports an argument type mismatch (`E4004`) between `Int` and the closure's function type

#### Scenario: Non-composable trailing lambda inside a compose fun
- **WHEN** checking `compose fun Child() { }`, `fun later(f: () -> Unit) { }` and `compose fun P() { later { Child() } }`
- **THEN** the type checker reports an invalid compose context error (`E4008`) for the call to `Child`

#### Scenario: Parameterless trailing lambda checked against its parameter type
- **WHEN** checking `fun run(body: () -> Int): Int = body()` and `fun f(): Int = run { 1 }`
- **THEN** the closure checks against `() -> Int` and the program typechecks without errors

### Requirement: Composable Function Types Keep Their Compose Flag
A function type written with the `compose` prefix MUST denote a composable function type wherever it is used, and one written without it MUST denote a non-composable one. A closure checked against a composable function type MUST be checked in a composable context, and composable and non-composable function types MUST NOT be assignable to each other.

#### Scenario: Composable content lambda may call composables
- **WHEN** checking `compose fun Item() { }`, `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Screen() { Column { Item() } }`
- **THEN** the program typechecks without errors

#### Scenario: Composable function type is not a plain function type
- **WHEN** checking `fun later(f: () -> Unit) { }` and `compose fun P(content: compose () -> Unit) { later(content) }`
- **THEN** the type checker reports an argument type mismatch (`E4004`) because the compose flags differ
