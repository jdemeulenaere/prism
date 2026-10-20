# Type System Specification

## Purpose
The type system defines semantic analysis, lexical symbol resolution, nominal type definitions, bidirectional type checking, and expression/statement validation for Prism programs. Error codes are listed in the [`diagnostics`](../diagnostics/spec.md) spec.

## Requirements

### Requirement: Lexical Scope and Symbol Resolution
The type checker MUST resolve identifiers across nested lexical scopes (local blocks, parameters, file declarations, and imported ABI symbols) and reject unresolved or duplicate symbols. Parameters form a scope level above the function body, so a local declared in the body MAY shadow a parameter.

#### Scenario: Local variable shadowing and resolution
- **WHEN** an expression references an identifier declared in an outer or enclosing block
- **THEN** the identifier resolves to the innermost matching declaration

#### Scenario: Local shadowing a parameter
- **WHEN** `fun f(a: Int): String { val a = "s"; return a }` is checked
- **THEN** the file typechecks successfully, and `a` after the local declaration refers to the `String` local

#### Scenario: Unknown identifier error reporting
- **WHEN** an expression references an undeclared identifier
- **THEN** the type checker emits an unresolved symbol error (`E4001`)

#### Scenario: Duplicate symbol declaration rejected
- **WHEN** a scope declares two variables or parameters with the identical identifier, including two closure parameters with the same name whether or not the closure has an expected type
- **THEN** the type checker rejects the duplicate declaration with `DuplicateSymbol` (`E4010`)

### Requirement: Static Type Inference and Assignment Compatibility
The type checker MUST infer types for unannotated `val` and `var` bindings, enforce strict numeric typing without implicit coercion, and verify assignment compatibility for references, nullable types, and struct fields. The floating-point type is `Double`, which a type annotation MAY also write `Float`; every diagnostic, dump and ABI MUST name it `Double`.

#### Scenario: Primitive type inference
- **WHEN** `val x = 42`, `val c = 'a'`, or `val b = true` is declared without an explicit type annotation
- **THEN** the type checker infers `Int`, `Char`, and `Bool` respectively

#### Scenario: Mismatched type assignment rejected
- **WHEN** a variable declared as `Int` is assigned an expression of type `String`
- **THEN** the type checker rejects the assignment with a type mismatch error (`E4002`)

#### Scenario: Implicit numeric widening prohibited
- **WHEN** an arithmetic operation mixes numeric types (e.g. `val x: Double = 1 + 2.0`)
- **THEN** the type checker rejects the operation with a type mismatch error (`E4002`)

#### Scenario: Diagnostics name the floating-point type Double
- **WHEN** `fun f(x: Float, y: Double) { x(1); y(2) }` is checked
- **THEN** the type checker emits `E4004` twice, each saying `type 'Double' cannot be called as a function`: the alias `Float` is the same type, and no message calls it `Float`

#### Scenario: Ordering comparison needs Int, Double or Char
- **WHEN** `<`, `<=`, `>` or `>=` compares two `Int`s, two `Double`s or two `Char`s, it has type `Bool`; when it compares two `Bool`s (`false < true`), two `String`s, values of different types (`1 < 2.0`), or any other type
- **THEN** the type checker rejects it with `E4002` through `OperatorMismatch`, whose message names the operator and both operand types (`operator '<' cannot be applied to 'Bool' and 'Bool'`) and whose help names the accepted types, rather than a mismatch between two identical types

#### Scenario: Reassignment to immutable val rejected
- **WHEN** code attempts to reassign to a variable declared with `val`
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Struct field mutability validation
- **WHEN** code attempts to assign to a struct field declared with `val`, or mutates a field on an immutable `val` struct instance
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Mutability of nested field places
- **WHEN** code assigns to a field path such as `o.user.name = "x"` or takes `&mut o.user.name`
- **THEN** every hop must be mutable: the root is a `var` binding or is reached through `&mut`, every field on the path is `var`, and no hop goes through a shared `&` borrow; otherwise the type checker emits `E4003`

#### Scenario: Reference and borrow compatibility
- **WHEN** passing an argument to a parameter expecting `&T` or `&mut T`
- **THEN** a `&T` parameter accepts a matching borrow (with `&mut T` coercible to `&T`) or a `T`, which is borrowed implicitly; a `&mut T` parameter requires a `&mut` borrow, written explicitly unless the argument already is one; and taking `&mut` on an immutable `val` binding is rejected

#### Scenario: Borrowed constructor values match annotated references
- **WHEN** a value built by a constructor (`val u = User("a")` or `val c = Color.Red`) is borrowed and passed to a parameter `&User`, `&mut User` or `&Color`
- **THEN** the borrow matches: `&T` accepts `&U` and `&mut T` accepts `&mut U` only when `U` and `T` are the same type, since a reference cannot change the representation of the value it points at

#### Scenario: Error types absorb borrows and safe navigation
- **WHEN** the operand of `&`, `&mut` or a safe navigation `?.` already has an error
- **THEN** the result has the error type, and no follow-on mismatch such as `found '&mut <error>'` is reported

#### Scenario: Nullable type compatibility
- **WHEN** `null` is assigned to a variable of type `T?`
- **THEN** the assignment succeeds; whereas assigning `null` to non-nullable `T` emits `E4002`

#### Scenario: Safe navigation and Elvis operator evaluation
- **WHEN** accessing a field via safe navigation `opt?.field` on `T?` or coalescing via `opt ?: default`
- **THEN** `opt?.field` evaluates to `FieldType?` and `opt ?: default` evaluates to `T` when `default` is of type `T`

#### Scenario: Elvis right side is always checked
- **WHEN** `left ?: right` is evaluated and `left` is not nullable
- **THEN** `right` is still typechecked (an unresolved name emits `E4001`, an incompatible type emits `E4002`), and the expression has the type of `left`

#### Scenario: Nullable types never nest
- **WHEN** a safe navigation reads a nullable field through a nullable value (`b?.a` where `a: A?`), so the chain `b?.a?.name` continues
- **THEN** the result is `A?`, not `A??`, and the chain resolves `name` against `A` to give `String?`

#### Scenario: Indexing expression evaluation
- **WHEN** indexing into a collection or array `target[index]`
- **THEN** `index` must evaluate to `Int` and the expression produces the element type (or emits `E4002` if incompatible)

### Requirement: Function Signature and Invocation Typechecking
Function calls and closures MUST validate argument count, argument types, default parameter handling, and enforce composable calling context constraints across closure boundaries.

#### Scenario: Call argument type match
- **WHEN** calling `fun add(x: Int, y: Int): Int` with arguments `(1, 2)`
- **THEN** the call expression typechecks successfully with type `Int`

#### Scenario: Arity or argument type mismatch rejected
- **WHEN** calling a function with incorrect number of arguments or incompatible types
- **THEN** the type checker emits a call signature mismatch error (`E4004`)

#### Scenario: Default parameter omission succeeds
- **WHEN** calling a function omitting arguments that have default expressions
- **THEN** the call expression typechecks successfully

#### Scenario: Default parameter values are checked
- **WHEN** a parameter declares a default value (`fun f(a: Int, b: Int = "s")`)
- **THEN** the default is typechecked against the parameter type in a scope holding only the earlier parameters, so a mismatch emits `E4002` and an unknown name emits `E4001`

#### Scenario: Parameter assigned twice in one call rejected
- **WHEN** a call or struct constructor gives the same parameter or field both a positional and a named argument (`f(a = 1, 5)` or `f(5, a = 1)` for `fun f(a: Int, b: Int = 2)`)
- **THEN** the type checker emits `E4004`

#### Scenario: Closure checking against expected function type
- **WHEN** a closure `{ x -> x + 1 }` is passed to a parameter expecting `(Int) -> Int`
- **THEN** the closure parameters and body typecheck against the expected function signature

#### Scenario: Calling composable function outside composable context rejected
- **WHEN** a standard function or non-composable closure attempts to invoke a function declared with the `compose` modifier (`compose fun`)
- **THEN** the type checker rejects the call with an invalid compose context error (`E4008`)

#### Scenario: Calling composable function within composable context succeeds
- **WHEN** a function or closure declared with `compose fun` invokes another `compose fun`
- **THEN** the call expression typechecks successfully

#### Scenario: Exported function missing return type rejected with dedicated code
- **WHEN** a `public` or `package` function declaration omits its return type annotation (e.g. `public fun greet(name: String) { }`)
- **THEN** the type checker emits a missing return type error (`E4011`) naming the function, and does NOT report it as a call signature mismatch (`E4004`)

### Requirement: Control Flow Type Unification and Return Validation
Control flow expressions (`if`/`else`, `when`), blocks, and function return statements MUST satisfy type unification and exhaustiveness rules.

#### Scenario: If-else branch unification
- **WHEN** an `if (cond) e1 else e2` expression is evaluated
- **THEN** the condition must typecheck as `Bool`, and both branches must unify to a common type

#### Scenario: If without else where a value is expected
- **WHEN** `fun f(c: Bool) { val x: Int = if (c) nope }` is checked
- **THEN** the type checker emits `E4001` once, for `nope`, and `E4002` once, because an `if` without `else` has type `Unit`

#### Scenario: Branches join with null into a nullable type
- **WHEN** one branch of an `if`/`else` or `when` has type `T` and another is `null` or has type `T?` (e.g. `val x = if (c) 1 else null`)
- **THEN** the branches join to `T?`

#### Scenario: Diverging expression has bottom type Nothing
- **WHEN** an expression diverges (such as a `return` expression) inside an `if` or `when` branch
- **THEN** the diverging branch has bottom type `Nothing`, which unifies with any expected target branch type (e.g. `val x: Int = if (cond) return 0 else 42`)

#### Scenario: Subject when expression branch checking
- **WHEN** evaluating `when (subject) { p1 -> e1, p2 -> e2, else -> e3 }`
- **THEN** pattern expressions `p1` and `p2` must match the type of `subject`, and branch results unify to a common type

#### Scenario: Subjectless when expression condition checking
- **WHEN** evaluating `when { c1 -> e1, c2 -> e2, else -> e3 }`
- **THEN** conditions `c1` and `c2` must each evaluate to `Bool`

#### Scenario: Range pattern without a subject rejected
- **WHEN** `fun f(x: Int): Int = when { in 0..<10 -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4002`, because without a subject a pattern is a condition, which a range is not

#### Scenario: Non-exhaustive when expression rejected
- **WHEN** a `when` expression is used in value position and fails to cover all possible enum variants or omit an `else` branch
- **THEN** the type checker rejects the expression with a non-exhaustive pattern match error (`E4009`) naming a value no branch covers (see the [`patterns`](../patterns/spec.md) spec, "Uncovered Values Are Named")

#### Scenario: Boolean subject covered by true and false
- **WHEN** a `when` over a `Bool` subject (or a `&Bool`) in value position has a branch for the literal `true` and one for `false`, or a `Bool?` subject has branches for `true`, `false` and `null`, each alone or among several patterns of a branch
- **THEN** it is exhaustive without an `else` branch, as a `when` over an enum naming every variant is, and the typed tree records it as exhaustive; with only one of the literals, or without `null` over a `Bool?`, it still needs `else` (`E4009`)

#### Scenario: Only a variant name covers that variant
- **WHEN** a `when` over an enum has a pattern naming a payload-free variant (`Color.Red`, or bare `Red` when no local binding shadows it), a payload variant alone (`Shape.Circle`), or a constructor pattern whose sub-patterns cover every payload (`Shape.Circle(val r)`, `Shape.Circle(_)`)
- **THEN** the pattern covers that variant, while a constructor pattern with a refutable sub-pattern, such as `Shape.Circle(1)`, covers only the values it matches, and a value that only evaluates to a variant, such as a field `h.Green`, covers nothing

#### Scenario: When statement needs no else branch
- **WHEN** a `when` is used as a statement, either before the last statement of a block or as the last statement of a block whose expected type is `Unit`
- **THEN** it needs no `else` branch, its branches need no common type, and it has type `Unit`

#### Scenario: When over a borrowed enum
- **WHEN** the subject of a `when` has type `&E` or `&mut E` for an enum `E`
- **THEN** its patterns are checked against `E` and its variants count toward exhaustiveness

#### Scenario: Block statement evaluation and Unit typing
- **WHEN** evaluating a block of statements `{ s1; s2 }`
- **THEN** a block without a tail expression (an empty block `{}`, or one ending in a declaration, an assignment or a loop) evaluates to `Unit`, and otherwise the tail expression determines the block type

#### Scenario: Return statement type verification
- **WHEN** a `return expr` statement occurs inside a function declared with return type `T`
- **THEN** `expr` must be compatible with `T`, and returning a value inside a `Unit` function or returning mismatched types emits a return type error (`E4005`)

### Requirement: User-Defined Struct and Nominal Type Validation
The type checker MUST validate struct instantiations, field access expressions, and enum variant constructors.

#### Scenario: Struct instantiation and field access
- **WHEN** a struct `Point { val x: Int, val y: Int }` is instantiated and `p.x` is accessed
- **THEN** instantiation verifies all required fields and `p.x` evaluates to `Int`

#### Scenario: Accessing unknown field rejected
- **WHEN** accessing a field not declared on the target struct
- **THEN** the type checker emits an unknown field error (`E4006`)

#### Scenario: Field access through an annotated struct type
- **WHEN** a value's type comes from an annotation naming a declared struct (a parameter `user: User`, a local `val p: Point = ...`, or a struct field `val user: User`) and a field of it is read
- **THEN** the field resolves against the declared struct and the access has the field's declared type, including chained access such as `order.user.name`

#### Scenario: Field access through a borrowed reference
- **WHEN** a parameter is declared `user: &User` or `user: &mut User` and `user.name` is read
- **THEN** the reference is peeled and `user.name` has the declared field type of `User`

#### Scenario: Field assignment through a mutable borrow
- **WHEN** a parameter is declared `user: &mut User` and the body assigns `user.name = n` to a `var` field with a compatible value
- **THEN** the assignment typechecks successfully, while assigning to a `val` field through the same borrow emits `E4003`

#### Scenario: Safe navigation on an annotated nullable struct
- **WHEN** a value is declared with type `User?` and `user?.name` is accessed
- **THEN** the access resolves against `User` and evaluates to the field type made nullable (`String?`)

#### Scenario: Field access on a struct imported from another file
- **WHEN** a file imports a module exporting `public struct User` and declares a parameter `user: User`
- **THEN** `user.name` resolves against the imported struct's ABI field list and typechecks with the exported field type

#### Scenario: Same-named variants in two enums
- **WHEN** two enums visible in one file declare a variant with the same name (`enum A { None }` and `enum B { None }`)
- **THEN** both declarations are accepted, the qualified paths `A.None` and `B.None` resolve, a bare `None` resolves as a `when` pattern over either enum, and a bare `None` elsewhere emits `E4001`

#### Scenario: Type names are not values
- **WHEN** a struct without a companion value, or an enum, is named as a value (`fun f(): User = User`), a field is read from a struct name (`User.name`), or an enum name is called (`A()`)
- **THEN** the type checker emits `E4002`, `E4006` and `E4004` respectively, while a struct constructor call `User(...)` and an enum variant path `A.X` or `A.Y(1)` still typecheck

#### Scenario: Unknown nominal type name stays opaque
- **WHEN** an annotation names a type that is neither a primitive nor a declared or imported struct or enum, nor a public struct or enum of a library module
- **THEN** the annotation is accepted as an opaque nominal type, and accessing a member on a value of that type emits `E4006`; an annotation naming a library module's type without its import is `E4001` (`standard-library`, "Library Names Are Imported")

### Requirement: Infinitely Sized Types Are Rejected
A struct or enum MUST NOT contain itself by value: through its fields or payloads, through nullables, and through other structs and enums, with type arguments substituted, but not through `Heap<T>`, references or function types. Each declaration of the file on such a cycle MUST be reported with `E4023` (`InfinitelySizedType`) at its name, with help suggesting a `Heap<...>` field.

#### Scenario: An enum containing itself
- **WHEN** `enum E { X(E), Y }` is checked
- **THEN** the type checker emits `E4023` at `E`

#### Scenario: A cycle through a nullable and another struct
- **WHEN** `struct A { val b: B? }` and `struct B { val a: A }` are checked
- **THEN** the type checker emits `E4023` at `A` and at `B`

#### Scenario: A cycle through a generic instance
- **WHEN** `struct Box<T>(val v: T)` and `struct A { val b: Box<A> }` are checked
- **THEN** the type checker emits `E4023` at `A`, and not at `Box`

#### Scenario: Heap breaks the cycle
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }`, `struct Node { val next: Heap<Node>? }` and `struct Pair<A, B>(val a: A, val b: B)` with `struct Q { val p: Pair<Pair<Int, Int>, Int> }` are checked
- **THEN** the file typechecks

#### Scenario: A cycle across modules
- **WHEN** `a/one.prism` declares `public struct One { val two: Two? }` and imports `/b/two`, which declares `public struct Two { val one: One }` and imports `/a/one`
- **THEN** checking `a/one.prism` reports `E4023` at `One` and checking `b/two.prism` reports it at `Two`, each file seeing the other's definition through its ABI, and `prism check`, which stops at the first file with errors, reports the first of them

### Requirement: Cross-Module Imported Symbol and Visibility Validation
Phase 2 body checking MUST typecheck expressions against symbols exported in Phase 1 `FileAbi` from imported modules and enforce visibility boundaries.

#### Scenario: Accessing exported public function from imported package
- **WHEN** a file imports another module and calls an exported `public` function
- **THEN** the call resolves against the imported `FileAbi` signature and typechecks correctly

#### Scenario: Accessing package-visibility symbol across packages rejected
- **WHEN** a file imports a module from a different package and attempts to access a declaration marked with `package` visibility
- **THEN** the type checker rejects the access with a visibility access violation error (`E4007`)

#### Scenario: Package-visibility type named in an annotation across packages rejected
- **WHEN** a file in another package names a `package struct` or `package enum` in a parameter, return, field, variant payload or variable annotation (e.g. `fun g(s: Secret): Int`)
- **THEN** the type checker emits `E4007` at the annotation

#### Scenario: Accessing package-visibility symbol within same package succeeds
- **WHEN** a file accesses a `package`-visible declaration defined in another file within the identical package directory
- **THEN** the declaration resolves and typechecks successfully

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

### Requirement: Trailing Lambdas Bind to the Last Parameter
A trailing lambda MUST be bound to the callee's last parameter, for every callee: declared, extension (the receiver is not a parameter), generic (inferring type arguments as usual), function-typed value, struct constructor (last field) or variant (last payload). The arguments inside the parentheses MUST bind as they would without it, and a parameter they leave unset other than the last MUST take its default or be reported missing (`E4004`).

#### Scenario: Trailing lambda binds to the last parameter
- **WHEN** running `fun Button(label: String = "", onClick: () -> Unit = {}, content: () -> Unit) { print(label); onClick(); content() }` called as `Button(onClick = { println("hi") }) { println("content") }`
- **THEN** `label` takes its default, `onClick` the named closure and `content` the trailing lambda, so the program prints `hi` then `content`

#### Scenario: Trailing lambda binds to the last parameter of every callee
- **WHEN** a trailing lambda is passed to a generic function (`pick(3) { it + 1 }` for `fun <T: Copy> pick(value: T, choose: (T) -> T): T`), an extension (`2.twice { println(it) }` for `fun Int.twice(action: (Int) -> Unit)`), a function-typed value (`f(1) { println("body") }` for `f: (Int, () -> Unit) -> Unit`), a struct constructor (`Handler("h") { println("run") }` for `struct Handler(name: String, run: () -> Unit)`), a variant (`Event.Click(5) { println(it) }` for `enum Event { Click(Int, (Int) -> Unit) }`) or `listOf` (`listOf({ 1 }) { 2 }`)
- **THEN** it binds to the last parameter, field or payload, and the program runs with the lambda in that position

### Requirement: Trailing Lambdas Need a Free Last Parameter
A trailing lambda for a callee whose last parameter is already given an argument, by position or by name, or for a callee without parameters, MUST be rejected with `E4004`, naming that parameter.

#### Scenario: Trailing lambda for a last parameter already given rejected
- **WHEN** checking `fun Button(label: String = "", content: () -> Unit)` called as `Button(content = {}) { }` or `Button("a", {}) { }`, `f(1, {}) { }` for `f: (Int, () -> Unit) -> Unit`, or `Handler(run = {}) { }` for `struct Handler(name: String, run: () -> Unit)`
- **THEN** the type checker reports `E4004`, saying the trailing lambda is passed to the last parameter or field (`'content'`, `parameter 2`, `'run'`), which is already given an argument

#### Scenario: Trailing lambda for a callee without parameters rejected
- **WHEN** checking `fun none() { }` called as `none { }`
- **THEN** the type checker reports `E4004`, saying the function takes no parameters

### Requirement: Composable Function Types Keep Their Compose Flag
A function type written with the `compose` prefix MUST denote a composable function type wherever it is used, and one written without it MUST denote a non-composable one. A closure checked against a composable function type MUST be checked in a composable context, and composable and non-composable function types MUST NOT be assignable to each other.

#### Scenario: Composable content lambda may call composables
- **WHEN** checking `compose fun Item() { }`, `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Screen() { Column { Item() } }`
- **THEN** the program typechecks without errors

#### Scenario: Composable function type is not a plain function type
- **WHEN** checking `fun later(f: () -> Unit) { }` and `compose fun P(content: compose () -> Unit) { later(content) }`
- **THEN** the type checker reports an argument type mismatch (`E4004`) because the compose flags differ

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
Operator operands (arithmetic, comparison, equality, logical, unary and range bounds), and `if` and subjectless-`when` conditions, MUST be typed after peeling every `&`, `&mut` and `Heap<T>` layer, so a borrowed or boxed value can be read by operators. Positions that expect a value of type `T` (arguments, initializers, returns, assignment values) accept a `&T` or a `Heap<T>` only by copying a `Copy` value out of it, as the `references` spec defines.

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
- **WHEN** `fun g(s: String): Int` is declared and `fun f(a: &String): Int = g(a)` is checked
- **THEN** the type checker rejects the argument with `E4017`, as a move out of the borrow `a`

#### Scenario: Operators on boxed values
- **WHEN** `fun f(h: Heap<Int>, r: &Heap<Int>, c: Heap<Bool>): Int = if (c && h == r) h + r * 2 else -h` is checked
- **THEN** the function typechecks successfully

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

#### Scenario: Closure initializing an annotated value
- **WHEN** a file declares the top-level `val TWICE: (Int) -> Int = { n -> n * 2 }`, or a body declares `val g: (Int) -> Int = { x -> x + 1 }`
- **THEN** the annotation is the closure's expected type, so `n` and `x` have type `Int` and the file typechecks successfully

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
When two imported modules export the same name and a file uses that name (as a value, callee or type), the type checker MUST reject the use with an ambiguity error (`E4012`) that names both modules. Importing both modules without using the name MUST be accepted. A top-level declaration that has the same name as an imported symbol MUST be accepted and MUST shadow the import: every use of the name in the file resolves to the local declaration, and no ambiguity is reported for it.

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
- **WHEN** a file imports `a/lib`, which exports `public fun helper(x: Int): Int`, declares its own `fun helper(x: String): Int`, and calls `helper("s")`
- **THEN** the file typechecks successfully, and the call resolves to the local `helper`

#### Scenario: Local declaration resolving an ambiguity
- **WHEN** a file imports both `a/lib` and `b/lib`, which each export `helper`, declares its own `helper`, and uses it
- **THEN** the use resolves to the local declaration and no `E4012` is reported

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
A file MUST be able to receive and pass along a value whose nominal type is defined in a module it does not import directly. Reading or writing that value's fields, matching on its enum variants, constructing it, or naming the type in an annotation MUST be rejected with `E4014`, naming the type, its defining module and the import to add. The type checker MUST read only the ABIs of the file's direct imports.

#### Scenario: Field access on a type from an unimported module
- **WHEN** `shop/api` exports `public fun first(): Order`, where `Order` is declared in `shop/models`, and a file that imports only `/shop/api` evaluates `first().price`
- **THEN** the type checker emits `E4014` naming `Order`, `shop/models` and `import /shop/models`

#### Scenario: Passing the value along without touching its members
- **WHEN** the same file declares `fun keep(): Unit { send(first()) }`, where `send(o: Order)` is also exported by `shop/api`
- **THEN** the file typechecks successfully

#### Scenario: Importing the defining module
- **WHEN** the file also imports `/shop/models` and evaluates `first().price`
- **THEN** the access typechecks with the declared type of `price`

#### Scenario: Naming an unimported type in an annotation
- **WHEN** a file that imports only `/shop/api` declares `fun total(o: Order): Int = o.price`, or uses `Order` in any other annotation (a local, a return type, a struct field)
- **THEN** the type checker emits `E4014` at the annotation, naming `Order`, `shop/models` and `import /shop/models`, and reports no further error for uses of `o`

### Requirement: Half-Open Ranges
A half-open range `a..<b` MUST be typed exactly like a closed range `a..b`: both operands are checked as `Int`, and the result has the built-in `Range` type. An `in a..<b` pattern in a `when` MUST be checked like `in a..b`, with both bounds checked against the subject's type.

#### Scenario: Half-open range has the Range type
- **WHEN** `fun f(n: Int) { val r = 0..<n }` is checked
- **THEN** the program typechecks without errors and `r` has the built-in `Range` type

#### Scenario: Non-Int bound rejected
- **WHEN** `fun f() { val r = 0..<"s" }` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Half-open range pattern
- **WHEN** `fun f(x: Int): String = when (x) { in 0..<10 -> "small", else -> "large" }` is checked
- **THEN** the program typechecks without errors

### Requirement: Compound Assignment Typing
`t op= v` MUST typecheck as `t = t op v` with `t` written once. The target MUST be writable under the rules for `=` (`E4003` otherwise). The operator MUST apply to the target's type and the value's type under the operator rules: `Int` with `Int` and `Double` with `Double` for every compound operator, and `String` with `String` for `+=` only. Otherwise the type checker MUST report `E4002`, with help naming the operand types the operator accepts.

#### Scenario: Compound assignment of numbers and strings
- **WHEN** a body runs `var n = 1; n += 2; n -= 1; n *= 3; n /= 2; n %= 2`, `var d = 1.5; d += 0.5; d %= 1.0` and `var s = "a"; s += "b"`
- **THEN** the body typechecks successfully

#### Scenario: Compound assignment through fields and references
- **WHEN** `struct Counter { var count: Int }` and `mut fun Counter.bump() { count += 1; this.count *= 2 }` are declared, and `fun f(r: &mut Int, c: &mut Counter, k: &Int) { *r += k; c.count -= 1 }` is checked
- **THEN** the file typechecks successfully, and the value `k` is read through its reference

#### Scenario: Compound assignment to an immutable target rejected
- **WHEN** `struct P { val x: Int; var y: Int }` is declared and a body of `fun f(r: &Int, q: P)` runs `val n = 1; n += 1`, `var p = P(1, 2); p.x += 1`, `q.y += 1` or `*r += 1`
- **THEN** the type checker emits `E4003` for each, as it does for `=`

#### Scenario: Compound operator not applicable rejected
- **WHEN** a body runs `var n = 1; n += "a"`, `var s = "a"; s -= "b"`, `var m: Int? = 1; m += 1`, `var x = 1; x += 1.0` or `var b = true; b += true`
- **THEN** the type checker emits `E4002` for each, with help saying which operand types the operator accepts

#### Scenario: Compound assignment does not consume its value
- **WHEN** `fun take(s: String) {}` is declared and `fun f(t: String) { var s = "a"; s += t; take(t) }` is checked
- **THEN** the file passes `prism check`, because `+=` reads `t` in place

### Requirement: Compound Assignment to a Reference Binding
A compound assignment whose target is a binding of reference type MUST be rejected, because it would re-point the binding to a number or a string: with `E4002` for a `var` binding and `E4003` alone for a `val`, with help suggesting `*r op= v` when the reference is `&mut`, or saying that a shared borrow cannot be written through.

#### Scenario: Compound assignment to a reference binding rejected
- **WHEN** `fun f(r: &mut Int) { r += 1 }` and `fun g(a: Int) { var x = 1; var r: &mut Int = &mut x; r += a }` are checked
- **THEN** the type checker emits `E4003` for `f` and `E4002` for `g`, with help suggesting `*r += 1` and `*r += a`

### Requirement: Output Built-In Arguments
`print` and `println` (`/prism/core`) MUST take one positional argument, `println` also none, with a text form by the rule of template parts ("Built-In Text Forms of Template Parts", "Other Template Parts Call toString"). An argument without one (a nullable, `Unit`, a function value, a type parameter, or a struct or enum without a usable `toString()`) MUST be `E4026`, with a template part's help. Another count or a named argument MUST be `E4004`. A call MUST have type `Unit`.

#### Scenario: Printing an unsupported value
- **WHEN** `struct P { val x: Int }` is declared without a `toString()` and `fun f(n: Int?) { println(P(1)); println(n); println({ 1 }) }` is checked
- **THEN** the type checker emits `E4026` for each of the three arguments, the first with help suggesting `fun P.toString(): String` and the second with help suggesting `?:`

#### Scenario: Wrong argument count or form
- **WHEN** `fun f() { print(); println(1, 2); println(value = 1) }` is checked
- **THEN** the type checker emits `E4004` for each of the three calls

### Requirement: Printed Arguments Are Read in Place
The argument of `print` or `println` MUST be read in place through its `&`, `&mut` and `Heap<T>` layers, never moved. A `Copy` scalar (`Int`, `Double`, `Bool`, `Char`) MUST be passed by value, copied out, and a `String` borrowed implicitly, as for a `&String` parameter. Any other type MUST be printed through the `toString()` that `x.toString()` resolves, called with a shared borrow of the argument, its result printed and then dropped.

#### Scenario: Printing scalars and strings
- **WHEN** `fun f(r: &Int, s: String) { print(1); println(2.5); println(true); println('c'); println(r); println(s); println(s); println() }` is checked
- **THEN** the file typechecks, and `s` is not moved by either call

#### Scenario: Printing a value through its toString
- **WHEN** `struct P { val x: Int }` and `fun P.toString(): String = "P(" + x.toString() + ")"` are declared, and `fun f(p: P) { println(p); println(p) }` is checked
- **THEN** the file typechecks, each call borrows `p` for its `toString()` call, and `p` is not moved

#### Scenario: Printing boxed values
- **WHEN** `fun f(h: Heap<Int>, s: Heap<String>) { println(h); println(s); println(s) }` is checked
- **THEN** the file typechecks, and neither `h` nor `s` is moved

### Requirement: String Length
A `String` MUST have a built-in member `length` of type `Int`, read without parentheses through any `&`, `&mut` and `Heap<T>` layers, and `s?.length` on a `String?` MUST have type `Int?`. `length` is not a function: `s.length()` MUST be rejected as calling an `Int` (`E4004`), as a call of a field of type `Int` is.

#### Scenario: Reading the length
- **WHEN** `fun f(a: String, b: &String, c: String?): Int = a.length + b.length + (c?.length ?: 0)` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling the length
- **WHEN** `fun f(s: String): Int = s.length()` is checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: String Characters Are Values
`s[i]` on a `String`, read through any `&`, `&mut` and `Heap<T>` layers, MUST take an `Int` index (`E4002` otherwise) and have type `Char`. A string's character MUST be a value, not a place: it is computed from the string, so `s[i] = c` MUST be rejected with `E4003`, with help saying that strings are immutable, and a borrow of `s[i]`, written or implicit, MUST borrow a temporary holding the character.

#### Scenario: Reading a character
- **WHEN** `fun f(s: &String, i: Int): Char = s[i + 1]` is checked
- **THEN** the file typechecks successfully

#### Scenario: Assigning a character rejected
- **WHEN** `fun f() { var s = "ab"; s[0] = 'c' }` is checked
- **THEN** the type checker emits `E4003` at `s[0]`, with help saying that the characters of a string cannot be assigned

#### Scenario: A non-integer index rejected
- **WHEN** `fun f(s: String): Char = s['a']` is checked
- **THEN** the type checker emits `E4002`, expecting `Int`

### Requirement: Scalars Have a Built-In toString
An `Int`, `Double`, `Bool` or `Char`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have a built-in member function `toString()` taking no arguments and returning `String`, and `x?.toString()` on a nullable of one of these MUST have type `String?`. Arguments MUST be rejected with `E4004`. On any other type, `toString` MUST be an ordinary name, resolved to a field or an extension like any other member (`E4006` when none exists).

#### Scenario: Converting scalars to strings
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, m: Int?): String = n.toString() + d.toString() + b.toString() + c.toString() + (m?.toString() ?: "")` is checked
- **THEN** the file typechecks successfully

#### Scenario: toString takes no arguments
- **WHEN** `fun f(n: Int): String = n.toString(10)` is checked
- **THEN** the type checker emits `E4004`, saying that `toString()` takes no arguments

#### Scenario: toString of a struct is an extension
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red }`, `fun Point.toString(): String = x.toString() + "," + y.toString()`, `fun f(p: Point): String = p.toString()` and `fun g(c: Color): String = c.toString()` are checked
- **THEN** the type checker accepts `f` and the extension, and emits `E4006` for `toString` on `Color`

#### Scenario: toString through a box
- **WHEN** `fun f(h: Heap<Int>, r: &Heap<Char>): String = h.toString() + r.toString()` is checked
- **THEN** the file typechecks successfully

### Requirement: Heap Construction
A call `Heap(value)` MUST construct a `Heap<T>` holding `value`, where `T` is the call's explicit type argument (`Heap<Int?>(1)`), else the contents' type of the expected `Heap<T>`, else the type of `value`. The argument MUST be checked against `T` in a consuming position. Exactly one positional argument MUST be given, or the call is `E4004`.

#### Scenario: Boxing a value
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` is declared and `fun f(): Expr = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Lit(2)))` and `fun g(): Heap<Int> { val h = Heap(1); return h }` are checked
- **THEN** both typecheck, and `h` has type `Heap<Int>`

#### Scenario: The expected type gives the contents' type
- **WHEN** `fun f(): Heap<Int?> = Heap(1)` and `fun g(): Heap<Int?> = Heap<Int?>(null)` are checked
- **THEN** both typecheck, the `1` being converted to `Int?` before it is boxed

#### Scenario: The boxed value is consumed
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User): Heap<String> = Heap(u.name)` and `fun g(u: User): Heap<User> = Heap(u)` are checked
- **THEN** the type checker emits `E4017` for `u.name` and accepts `g`

#### Scenario: Wrong arguments of Heap rejected
- **WHEN** `fun f() { val a = Heap(); val b = Heap(1, 2); val c = Heap(value = 1); val d = Heap<Int>("x") }` is checked
- **THEN** the type checker emits `E4004` for each call, the last for `"x"`, an argument that does not fit `Int`

#### Scenario: Borrows in the type argument rejected
- **WHEN** `fun f(a: Int) { val h = Heap<&Int>(&a) }` is checked
- **THEN** the type checker emits `E4022`

### Requirement: The Name Heap Is Built In
`Heap` in call position MUST name the built-in constructor whatever is in scope. A top-level function, struct, enum or `val` named `Heap` MUST be rejected with `E4010`, and `Heap` named without a call MUST be rejected with `E4001`.

#### Scenario: A declaration named Heap rejected
- **WHEN** a file declares `struct Heap(val x: Int)` or `fun Heap(x: Int): Int = x`
- **THEN** the type checker emits `E4010`, saying that the declaration would hide the built-in `Heap`

#### Scenario: Heap is not a value
- **WHEN** `fun f() { val make = Heap }` is checked
- **THEN** the type checker emits `E4001` for `Heap`

### Requirement: Heap Types Are Invariant
`Heap<U>` MUST be assignable to `Heap<T>` only when `U` and `T` are the same type, because a box cannot change the representation of its contents. A mismatch MUST be reported like any other, with help saying that the contents can be converted when the box is built.

#### Scenario: A box is not a box of a nullable
- **WHEN** `fun g(h: Heap<Int?>) {}` is declared and `fun f(h: Heap<Int>) { g(h) }` and `fun k(h: Heap<Int>): Heap<Int?> = h` are checked
- **THEN** the type checker emits `E4004` for `f` and `E4005` for `k`, each with help suggesting that the box be built as a `Heap<Int?>`

#### Scenario: Branches of different boxes have no join
- **WHEN** `fun f(c: Bool) { val h = if (c) Heap(1) else Heap(null) }` and `fun g(c: Bool): Heap<Int?> = if (c) Heap(1) else Heap(null)` are checked
- **THEN** the type checker emits `E4002` for `f`, and accepts `g`, whose branches are each built as a `Heap<Int?>`

### Requirement: String Templates Have Type String
A string template MUST have type `String`, and each of its parts MUST be checked as an expression with no expected type, in the scope where the template appears: a name part resolves as the identifier would (a local, a parameter, a top-level value, a field of the receiver through implicit `this` in an extension body, or `this`).

#### Scenario: Templates are strings
- **WHEN** `fun f(n: Int): String = "n = $n"` and `fun g(n: Int): Int = "n = $n"` are checked
- **THEN** the type checker accepts `f` and emits `E4005` for `g`, which returns a `String` where an `Int` is declared

#### Scenario: Names resolve as in code
- **WHEN** `struct User { val name: String }`, `fun User.toString(): String = "user $name"` and `fun User.label(prefix: String): String = "$prefix: $this (${name.length})"` are checked
- **THEN** the file typechecks successfully: `$name` reads the receiver's field, and `$this` inserts the receiver through its `toString()` extension

### Requirement: Built-In Text Forms of Template Parts
A template part MUST be read through any `&`, `&mut` and `Heap<T>` layers. A `String` part MUST be inserted as it is, and an `Int`, `Double`, `Bool` or `Char` part as the text its built-in `toString()` gives. A part of type `Nothing` MUST be accepted. A template MUST NOT consume or mutate any of its parts.

#### Scenario: Parts of every built-in text type
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, s: &String, t: String, h: Heap<Int>): String = "$n $d $b $c $s $t $h ${s.length} ${n > 0}"` is checked
- **THEN** the file typechecks successfully

#### Scenario: A part is not moved
- **WHEN** `fun f(s: String): String { val t = "$s!"; return s + t }` is checked
- **THEN** the file typechecks successfully, since the template does not move `s`

### Requirement: Other Template Parts Call toString
A part of any other non-nullable type MUST be inserted as the result of `toString()` on it, resolved exactly as the method call `part.toString()` would be: a field, then an extension of the file or of a direct import, with the precedence and import rules of method calls. That member MUST be callable without arguments and return `String`, and an extension MUST take a shared receiver: a `mut fun` or `consuming fun` `toString()` is never called implicitly.

#### Scenario: A toString extension gives a text form
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red, Green }`, `fun Point.toString(): String = "($x, $y)"`, `fun Color.toString(): String = when (this) { Color.Red -> "red"; Color.Green -> "green" }` and `fun f(p: &Point, c: Color): String = "$p is $c"` are checked
- **THEN** the file typechecks successfully
- **WHEN** `lib/geo.prism` declares `public struct Point(val x: Int, val y: Int)` and `public fun Point.toString(): String = "($x, $y)"`, and a file importing `/lib/geo` declares `fun f(p: Point): String = "at $p"`
- **THEN** the importing file typechecks successfully, calling the imported extension

### Requirement: Parts Without a Text Form Are Rejected
A template part MUST be rejected with `E4026` at the part (the name of a name part, the expression of an expression part) when it is nullable (any `T?`, including `String?` and `null`), `Unit`, a function value or a type parameter, or when it has no `toString()` member usable by "Other Template Parts Call toString". The diagnostic MUST name the type and the reason.

#### Scenario: A nullable part rejected
- **WHEN** `fun f(name: String?, n: Int?): String = "Hello $name, ${n}"` is checked
- **THEN** the type checker emits `E4026` at `name` and at `n`, and accepts `fun g(name: String?): String = "Hello ${name ?: "guest"}"`

#### Scenario: Unit and function parts rejected
- **WHEN** `fun log() {}`, `fun f(): String = "${log()}"`, `fun g(h: () -> Int): String = "$h"` and `fun <T> k(t: T): String = "$t"` are checked
- **THEN** the type checker emits `E4026` in each of `f`, `g` and `k`

### Requirement: Text Form Diagnostics Name the Fix
An `E4026` diagnostic MUST carry help naming a fix: for a nullable part, a fallback with `?:` written with the part's text (`${n ?: ...}`); for a struct or enum without `toString()`, the extension to declare (`fun User.toString(): String`) and, when a name part is followed by text that starts with `.` and an identifier, the braced member (`${u.name}`); for a `mut fun` or `consuming fun` `toString()`, the explicit call (`${c.toString()}`).

#### Scenario: A struct without toString rejected
- **WHEN** `struct User { val name: String }` and `fun f(u: User): String = "user $u.name"` are checked
- **THEN** the type checker emits `E4026` at `u`, saying that `User` has no text form, with help suggesting `fun User.toString(): String` and `${u.name}`

#### Scenario: An unusable toString rejected
- **WHEN** `struct Point : Copy { val x: Int }`, `struct Counter { var n: Int }`, `fun Point.toString(): Int = x`, `mut fun Counter.toString(): String { n += 1; return "c" }`, `fun f(p: Point): String = "$p"` and `fun g(c: Counter): String = "$c"` are checked
- **THEN** the type checker emits `E4026` in `f`, saying that `toString()` of `Point` returns `Int`, and `E4026` in `g`, with help suggesting `${c.toString()}`

#### Scenario: Nullable help gives a fallback
- **WHEN** `fun f(n: Int?): String = "n = $n"` is checked
- **THEN** the type checker emits `E4026` at `n`, saying that a nullable `Int?` has no text form, with help suggesting `${n ?: ...}`
- **WHEN** `fun f(a: Int?, b: Int): String = "${if (b > 0) a else null}"` is checked
- **THEN** the help parenthesizes the part, suggesting `${(if (b > 0) a else null) ?: ...}`, since `?:` binds more tightly than `if`

### Requirement: Function Values Convert Only Without Changing Representation
A function value MUST convert to another function type only when parameters and result are equal or convert by `&mut T` to `&T` (also inside a nullable or function type), compose flags are equal, and each parameter's `escaping` flag is equal or set only in the target; otherwise its site's mismatch code (`E4002`, `E4004`, `E4005`) MUST be reported, with help suggesting a calling closure. A named function or variant constructor MAY convert its non-function parameters and result through any step.

#### Scenario: Converting a function value through a nullable parameter
- **WHEN** `fun show(n: Int?) {}` and `fun each(f: (Int) -> Unit) { f(1) }` are declared, and a body runs `val g: (Int?) -> Unit = show; each(g)`
- **THEN** the type checker emits `E4004` at `g` in `each(g)`, with help suggesting `{ x -> g(x) }`

#### Scenario: Converting a function value to a nullable result
- **WHEN** a body runs `val k = { 2 }; val m: () -> Int? = k`
- **THEN** the type checker emits `E4002` at `k` in the second declaration

#### Scenario: Branches joining function values of different representations
- **WHEN** `fun show(n: Int?) {}` and `fun count(n: Int) {}` are declared, and a body runs `val g: (Int?) -> Unit = show; val h = if (c) g else count` for a `Bool` `c`
- **THEN** the type checker emits `E4002` at `g`, since the branches join to `(Int) -> Unit`

#### Scenario: Named functions and closure literals still convert
- **WHEN** `fun show(n: Int?) {}`, `fun each(f: (Int) -> Unit) { f(1) }` and `fun pick(f: () -> Int?): Int? = f()` are declared, and a body runs `each(show)`, `val h: (Int) -> Unit = show` and `pick { 1 }`
- **THEN** the file passes `prism check`

#### Scenario: A named function's function-typed parameter converts as a value
- **WHEN** `fun each(f: (Int) -> Unit) { f(1) }` and `fun all(g: ((Int?) -> Unit) -> Unit) {}` are declared, and a body runs `all(each)`
- **THEN** the type checker emits `E4004` at `each`, because its parameter of type `(Int) -> Unit` would have to receive a `(Int?) -> Unit`

#### Scenario: A mutable reference parameter accepts a shared one
- **WHEN** `fun read(r: &Int): Int = r` is declared, and a body runs `val f: (&Int) -> Int = read; val g: (&mut Int) -> Int = f`
- **THEN** the file passes `prism check`

#### Scenario: A function value lent to a reference parameter
- **WHEN** `fun call(f: &(() -> Int)): Int = f()` is declared, and a body runs `val g: () -> Int = { 4 }; call(g); call(&g); call { 5 }`
- **THEN** the file passes `prism check`: lending a function value converts nothing

### Requirement: Escaping Parameters Are Part of Function Types
Whether each parameter of a function or function type is `escaping` MUST be part of its type, rendered `(escaping () -> Unit) -> Unit`. A function type whose parameter is unmarked MUST be assignable to one that is equal except that the parameter is `escaping`, in parameter position contravariantly as usual, and that step MUST keep the value's representation. The reverse MUST be a type mismatch.

#### Scenario: Unmarked parameters convert to escaping ones
- **WHEN** `fun runNow(f: () -> Unit) { f() }`, `fun keepIt(f: escaping () -> Unit) {}`, `fun register(r: (escaping () -> Unit) -> Unit) {}` and `fun now(r: (() -> Unit) -> Unit) {}` are declared, and `fun f() { register(runNow); register(keepIt); now(runNow); val g: (() -> Unit) -> Unit = runNow; register(g) }` and `fun h() { now(keepIt) }` are checked
- **THEN** the type checker accepts `f`, and emits `E4004` for `h`, saying that `(escaping () -> Unit) -> Unit` is not assignable to `(() -> Unit) -> Unit`

#### Scenario: A closure's parameters take the expected flags
- **WHEN** `fun keepIt(f: escaping () -> Unit) {}`, `fun register(r: (escaping () -> Unit) -> Unit) {}` and `fun now(r: (() -> Unit) -> Unit) {}` are declared, and `fun f() { register { g -> keepIt(g) } }` and `fun h() { now { g -> keepIt(g) } }` are checked
- **THEN** `f` passes `prism check`, because the closure's parameter `g` is `escaping`, and the borrow checker emits `E5007` for `h`, saying that the parameter `g` cannot escape

#### Scenario: Escaping types in diagnostics and dumps
- **WHEN** `fun keepIt(f: escaping () -> Unit): Int = 1` is declared and `fun h(): String = keepIt` is checked
- **THEN** the type checker emits `E4005`, naming the type `(escaping () -> Unit) -> Int`, and `prism dump-typed-tree` on a file declaring only `keepIt` shows its parameter `f` as escaping

### Requirement: Implicit Closure Parameter
A closure without a parameter header, checked against a function type with exactly one parameter, MUST have one parameter named `it`, bound as a `val` in the closure's scope with that parameter's type and `escaping` flag, as if written `{ it -> ... }`. Whether a closure has `it` MUST depend only on its header and its expected type, never on its body.

#### Scenario: it takes the expected parameter type
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` is declared and a body runs `val a = apply { it + 1 }` and `val g: (String) -> Int = { it.length }`
- **THEN** the file typechecks successfully, `it` having type `Int` in the first closure and `String` in the second

#### Scenario: it in a generic higher-order call
- **WHEN** `fun <T, R> map(x: T, f: (T) -> R): R = f(x)` is declared and a body runs `val n: Int = map(2) { it * 3 }` and `val s: String = map(2) { it.toString() }`
- **THEN** the file typechecks successfully: `T` is inferred as `Int` from the first argument before the closure is checked, and `R` from the closure's body

#### Scenario: it is a reference for a borrowed parameter
- **WHEN** `fun each(xs: &List<String>, f: (&String) -> Unit) { for (x in xs) f(x) }` and `fun count(s: &String): Int = s.length` are declared, and a body runs `var total = 0; each(listOf("a", "bc")) { total += it.length; total += count(it) }`
- **THEN** the file typechecks successfully, `it` having type `&String`, read through it by `.length` and passed on by `count(it)`

#### Scenario: it where a nullable function type is expected
- **WHEN** `fun apply(f: ((Int) -> Int)?, x: Int): Int { val fallback: (Int) -> Int = { -it }; val g = f ?: fallback; return g(x) }` is declared and `main` runs `val f: ((Int) -> Int)? = { it + 1 }`, `println(apply(f, 1))`, `println(apply({ x -> x * 2 }, 2))` and `println(apply({ it * 10 }, 4))`
- **THEN** the file typechecks successfully, each closure being checked against `(Int) -> Int` and then made nullable, and it prints `2`, `4` and `40`

#### Scenario: Unused it is accepted
- **WHEN** `fun each(f: (Int) -> Unit) { f(1) }` is declared and a body runs `each { println("x") }`
- **THEN** the file typechecks successfully

### Requirement: Closures Without an Implicit Parameter
A closure with a header, including the empty header `{ -> }`, a closure whose expected function type has no parameters, and a closure with no expected function type MUST have no implicit `it`. A use of `it` that no enclosing scope binds MUST be rejected with `E4001`, with help saying why the innermost closure has no `it`, or, outside any closure, which closures have one.

#### Scenario: it in a closure expecting no parameters rejected
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun f(): Int = run { it }` is checked
- **THEN** the type checker emits `E4001` at `it`, with help saying that the closure's type `() -> Int` has no parameters, so it has no `it`

#### Scenario: it in a closure without an expected type rejected
- **WHEN** a body runs `val f = { it + 1 }`
- **THEN** the type checker emits `E4001` at `it`, with help saying that only a closure passed where a function of one parameter is expected has an implicit `it`

#### Scenario: it in a closure with a header rejected
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` is declared and `fun f(): Int = apply { x -> it + x }` and `fun g(): Int = apply { -> 1 }` are checked
- **THEN** the type checker emits `E4001` at `it` in `f`, with help saying that the closure declares its parameter `x`, and `E4004` for the closure of `g`, which declares zero parameters where one is expected

#### Scenario: it outside a closure rejected
- **WHEN** `fun f(): Int = it` is checked
- **THEN** the type checker emits `E4001` at `it`, with help saying that `it` names the parameter of a closure that expects one

### Requirement: Implicit Parameters Shadow Outer Names
The implicit `it` MUST shadow every outer binding of `it`: a variable, a parameter, and the `it` of an enclosing closure. A closure without an implicit `it` MUST see the `it` of the enclosing scope, as any other name.

#### Scenario: Nested closures each have their own it
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` and `fun show(f: (String) -> Int): Int = f("ab")` are declared and `fun f(): Int = apply { it + show { it.length } }` is checked
- **THEN** the file typechecks successfully: the outer `it` is an `Int`, the inner one a `String`

#### Scenario: A closure without it sees the enclosing it
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(): Int = apply { run { it + 1 } }` and `fun g(it: String): Int = apply { it + 1 }` are checked
- **THEN** both typecheck successfully: in `f` the inner `it` is the outer closure's `Int`, and in `g` the closure's `Int` `it` shadows the `String` parameter

### Requirement: Header-Less Closures Expecting Several Parameters
A closure without a header checked against a function type with two or more parameters MUST be rejected with `E4004`, naming the expected parameter types, with help writing a header that declares them and saying that only a closure expecting one parameter has an implicit `it`.

#### Scenario: Header-less closure for two parameters rejected
- **WHEN** `fun pairs(f: (Int, String) -> Unit) { f(1, "a") }` is declared and a body runs `pairs { println(it) }`
- **THEN** the type checker emits `E4004` at the closure, saying that it expected 2 parameters of types `Int` and `String` but declares none, with help suggesting `{ a, b -> ... }`

### Requirement: Function Types With a Receiver
A function type with a receiver `T.(A) -> R` MUST be a function type whose first parameter is the receiver, of type `&T`, followed by the written parameters; with `mut`, the receiver's parameter MUST be `&mut T`. Whether a function type has a receiver MUST be part of its identity and of how it is rendered (`ColumnScope.(Int) -> Unit`, `mut Builder.() -> Unit`, `compose ColumnScope.() -> Unit`), and its `compose` and `escaping` flags MUST behave as for any function type.

#### Scenario: Receiver function types in diagnostics
- **WHEN** `struct ColumnScope : Copy {}` and `fun Column(content: ColumnScope.(Int) -> Unit) {}` are declared and `fun f(): String = Column` is checked
- **THEN** the type checker emits `E4005`, naming the type `(ColumnScope.(Int) -> Unit) -> Unit`

#### Scenario: Composable receiver function types keep their compose flag
- **WHEN** `struct ColumnScope : Copy {}`, `compose fun Item() {}`, `compose fun Column(content: compose ColumnScope.() -> Unit) { content(ColumnScope()) }`, `compose fun Screen() { Column { Item() } }` and `fun later(f: ColumnScope.() -> Unit) {}` are declared, and `compose fun P(content: compose ColumnScope.() -> Unit) { later(content) }` is checked
- **THEN** `Screen` typechecks, the lambda being checked in a composable context, and the type checker emits `E4004` at `content` in `later(content)`, because the compose flags differ

### Requirement: Receiver and Plain Function Types Convert in Place
A function value of a receiver function type MUST convert to the plain function type with the same parameters (`T.(A) -> R` to `(&T, A) -> R`, `mut T.(A) -> R` to `(&mut T, A) -> R`) and back, in place, under the other rules of "Function Values Convert Only Without Changing Representation": a shared receiver's type MAY become a mutable receiver's, and a mutable receiver's MUST NOT become a shared one's.

#### Scenario: Converting between receiver and plain function types
- **WHEN** `struct Scope : Copy {}`, `fun header(s: &Scope) {}`, `fun column(content: Scope.() -> Unit) { content(Scope()) }` and `fun plain(f: (&Scope) -> Unit) { f(Scope()) }` are declared, and a body runs `val h: (&Scope) -> Unit = header`, `column(h)`, `val c: Scope.() -> Unit = { }`, `plain(c)`, `val d: Scope.() -> Unit = { }`, `val m: mut Scope.() -> Unit = d` and `val p: (&mut Scope) -> Unit = m`
- **THEN** the file passes `prism check`

#### Scenario: A mutable receiver does not become a shared one
- **WHEN** `struct Builder { var size: Int }` and `fun read(f: Builder.() -> Unit) {}` are declared and a body runs `val m: mut Builder.() -> Unit = { size += 1 }; read(m)`
- **THEN** the type checker emits `E4004` at `m` in `read(m)`

### Requirement: Lambdas With a Receiver
A closure checked against a function type with a receiver MUST bind `this`, as a `val` of the receiver's parameter type, in its own scope before its parameters. Its header MUST declare the other parameters only, and a closure without a header MUST have the implicit `it` exactly when there is one other parameter. A closure with no expected type, or checked against a function type without a receiver, MUST have no receiver of its own.

#### Scenario: this in a lambda with a receiver
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun nest(f: Scope.(String) -> Int): Int = f(Scope(1), "ab")` are declared and a body runs `val a = nest { this.depth + it.length }` and `val b = nest { s -> depth + s.length }`
- **THEN** the file typechecks successfully: `this` has type `&Scope`, `it` and `s` have type `String`, and `depth` reads `this.depth`

#### Scenario: Receiver lambdas from every expected type
- **WHEN** `struct Scope : Copy { val depth: Int }`, `struct Section { val content: Scope.() -> Int }` and `fun <T> with(receiver: &T, block: T.() -> Int): Int = block(receiver)` are declared, and a body runs `val f: Scope.() -> Int = { depth }`, `val g: (Scope.() -> Int)? = { depth + 1 }`, `val s = Section { depth * 2 }` and `val n = with(Scope(3)) { depth }`
- **THEN** the file typechecks successfully, each closure having a receiver of type `&Scope`, the last one after `T` is inferred as `Scope` from the first argument

#### Scenario: A mutable receiver may be written through
- **WHEN** `struct Builder { var size: Int }` is declared and a body runs `val f: mut Builder.() -> Unit = { size += 1 }` and `val g: Builder.() -> Unit = { size += 1 }`
- **THEN** the type checker accepts `f` and emits `E4003` in `g`, because its `this` is a shared borrow

#### Scenario: A closure against a plain function type has no receiver
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun plain(f: (&Scope) -> Int): Int = f(Scope(1))` are declared and `fun f(): Int = plain { depth }` and `fun g(): Int = plain { this.depth }` are checked
- **THEN** the type checker emits `E4001` at `depth` in `f` and at `this` in `g`, saying that `this` is only available in an extension function or a lambda with a receiver

#### Scenario: The receiver is no implicit it
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun none(f: Scope.() -> Int): Int = f(Scope(1))` are declared and `fun g(): Int = none { it.depth }` is checked
- **THEN** the type checker emits `E4001` at `it`, with help saying that the closure's type `Scope.() -> Int` has no parameters besides its receiver, so it has no implicit `it`

#### Scenario: Parameter counts exclude the receiver
- **WHEN** `struct Scope : Copy {}` and `fun pair(f: Scope.(Int, Int) -> Int): Int = f(Scope(), 1, 2)` are declared and a body runs `pair { a, b -> a + b }` and `pair { it }`
- **THEN** the type checker accepts the first closure and emits `E4004` for the second, saying that it expected 2 parameters of types `Int` and `Int` but declares none

### Requirement: Calling Function Values With a Receiver
A value `f` of a receiver function type MUST be callable as `f(r, a)`, with the receiver as the first argument, as `r.f(a)`, and as `f(a)` when the innermost `this` has the receiver's type, which is then passed as the receiver. A call `f(a)` with one argument fewer than its parameters and no innermost `this` of the receiver's type MUST be rejected with `E4004`, saying that `f` takes a receiver of that type and how to pass one.

#### Scenario: Three ways to call a receiver function value
- **WHEN** `struct Scope : Copy { val depth: Int }` is declared and `fun a(f: Scope.(Int) -> Int): Int = f(Scope(1), 2)`, `fun b(f: Scope.(Int) -> Int): Int { val s = Scope(1); return s.f(2) }` and `fun Scope.c(f: Scope.(Int) -> Int): Int = f(2)` are checked
- **THEN** the file typechecks successfully, each call passing a `Scope` as the receiver of `f`

#### Scenario: A receiver function value called without a receiver
- **WHEN** `struct Scope : Copy {}` and `struct Other : Copy {}` are declared and `fun a(f: Scope.() -> Int): Int = f()` and `fun Other.b(f: Scope.() -> Int): Int = f()` are checked
- **THEN** the type checker emits `E4004` for each call, saying that `f` takes a receiver of type `Scope`, passed as `f(scope)` or `scope.f()`

### Requirement: Default Values Are Not Composable Contexts
A parameter's default value MUST be checked outside any composable context, also for a parameter of a composable, since its generator runs in the caller before the call. A composable call in a default value MUST be rejected with `E4008`, with a note that default values are not composable.

#### Scenario: A composable call in a default value rejected
- **WHEN** `compose fun Size(): Int = 4` and `compose fun Box(width: Int = Size()) { }` are checked
- **THEN** the type checker reports `E4008` for the call of `Size`, noting that default values are not composable

#### Scenario: A constant default of a composable
- **WHEN** `compose fun Box(width: Int = 4) { }` and `compose fun Screen() { Box() }` are checked
- **THEN** the file typechecks

### Requirement: Mutable State Handles Convert to Read-Only Ones
A `MutableState<T>` MUST be assignable where a `State<T>` of the same `T` is expected, by value and behind a shared reference (`&MutableState<T>` as `&State<T>`), as a subtype step that keeps the state object, and an argument of type `MutableState<T>` MUST be borrowed implicitly for a `&State<T>` parameter. A `State<T>` MUST NOT convert to `MutableState<T>`, and `MutableState<A>` MUST NOT convert to `State<B>` for another `B`.

#### Scenario: Passing a mutable handle as a read-only one
- **WHEN** `fun show(s: &State<Int>): Int = s.value` and `fun main() { val m = mutableStateOf(7); println(show(m)) }` run
- **THEN** stdout is `7`

#### Scenario: Type arguments inferred through the conversion
- **WHEN** `fun <T : Copy> get(s: &State<T>): T = s.value` and `fun main() { println(get(mutableStateOf(5))) }` run
- **THEN** stdout is `5`: the `MutableState<Int>` argument, borrowed as a temporary, solves `T` as `Int`

#### Scenario: A read-only handle does not become mutable
- **WHEN** `fun f(s: &State<Int>): &MutableState<Int> = s` is checked
- **THEN** the type checker emits `E4005`

### Requirement: Lookups on a Mutable Handle Fall Back to State
A member or extension looked up on a `MutableState<T>` receiver and not found for `MutableState` MUST be looked up for `State<T>`, with the receiver converted, so the members and extensions of `State<T>` apply to both handles.

#### Scenario: Reading through a mutable handle
- **WHEN** `fun <T> State<T>.describe(): String = "state"` and `fun main() { val m = mutableStateOf(2); println(m.value + 1); println(m.describe()) }` run
- **THEN** stdout is `3` and `state`

### Requirement: A Struct May Have a Companion Value
A top-level `val` MAY have the name of a struct declared in the same file, as that struct's companion value. The name MUST then mean the struct in a type position and the struct's constructor as the callee of a call. Anywhere else in a value position, as a receiver or before `.field` included, it MUST mean the value.

#### Scenario: A companion value and its struct
- **WHEN** a file declares `struct Color : Copy { val argb: Int }`, `struct Colors : Copy { val Red: Color }`, `val Color: Colors = Colors(Red = Color(0xFFFF0000))` and `fun pick(c: Color): Int = c.argb`, and `fun main() { println(pick(Color.Red)); println(pick(Color(7))) }` runs
- **THEN** it prints `4294901760` and `7`: `Color` is the struct in `c: Color`, the constructor in `Color(7)`, and the value in `Color.Red`

#### Scenario: An extension called on a companion value
- **WHEN** a file declares `struct Gap : Copy { val size: Int }`, `struct Gaps : Copy { val None: Gap }`, `val Gap: Gaps = Gaps(Gap(0))` and `fun Gaps.of(size: Int): Gap = Gap(size)`, and `fun main() { println(Gap.of(8).size) }` runs
- **THEN** it prints `8`

### Requirement: Only a Same-File Struct and Value Share a Name
Every other pair of top-level declarations with one name MUST still be rejected with `E4010`: two `val`s, a `val` and an enum or a function, and a `val` and a struct that are not declared in the same file, an imported one included. An extension is a member of its receiver type, not a top-level name, so a `val` may share its name, as before.

#### Scenario: A value named like an imported struct
- **WHEN** a file imports a module exporting `public struct User` and declares `val User: Int = 1`
- **THEN** the type checker emits `E4010` for `User`

#### Scenario: A value named like an enum
- **WHEN** a file declares `enum Mode { On, Off }` and `val Mode: Int = 1`
- **THEN** the type checker emits `E4010` for `Mode`

### Requirement: Importing a Struct With Its Companion Value
Importing a module MUST bring a public struct and its public companion value under their shared name, each meaning what it means in its own file.

#### Scenario: Using an imported companion value
- **WHEN** `lib/shape.prism` declares `public struct Size : Copy { val w: Int }`, `public struct Sizes : Copy { val Zero: Size }` and `public val Size: Sizes = Sizes(Size(0))`, and `app/main.prism` imports `/lib/shape` and declares `fun main() { val s: Size = Size.Zero; println(s.w + Size(2).w) }`
- **THEN** `prism run app/main.prism` prints `2`

### Requirement: Attached Parameters Are Part of Function Types
Whether each parameter of a function or function type is `attached` MUST be part of its type, rendered `(attached () -> Unit) -> Unit`. A function type MUST be assignable to one equal to it except that a parameter unmarked in it is `attached`, or one `attached` in it is `escaping`, keeping its representation. Any other difference of the flags MUST be a type mismatch.

#### Scenario: Attached parameters in conversions
- **WHEN** `fun runNow(f: () -> Unit) { f() }`, `fun keepIt(f: attached () -> Unit) {}`, `fun store(f: escaping () -> Unit) {}`, `fun register(r: (attached () -> Unit) -> Unit) {}` and `fun registerEscaping(r: (escaping () -> Unit) -> Unit) {}` are declared, and `fun f() { register(runNow); register(keepIt); registerEscaping(keepIt) }` and `fun h() { register(store) }` are checked
- **THEN** the type checker accepts `f`, and emits `E4004` for `h`, saying that `(escaping () -> Unit) -> Unit` is not assignable to `(attached () -> Unit) -> Unit`

#### Scenario: An escaping value given to an attached parameter
- **WHEN** `fun keepIt(f: attached () -> Unit) {}` is declared and `fun g(h: escaping () -> Unit) { keepIt(h) }` is checked
- **THEN** the file passes `prism check`

## Design Rationale
Decisions from `type-checker-core` and `resolve-nominal-type-annotations`:

- **A separate `prism-typechecker` crate.** Semantic analysis stays decoupled from the grammar and from module resolution, and can be tested with in-memory ASTs.
- **Bidirectional checking, not Hindley-Milner.** Exported signatures and parameters always carry types, so local `infer`/`check` modes are enough. They are fast and deterministic and give precise errors without unification traces.
- **A small subtyping lattice.** `Nothing` is assignable to every type, `T` to `T?`, `T?` to `U?` when `T` is assignable to `U`, and `&mut T` to `&T`. Everything else is invariant: in particular `Int` never widens to `Double`. An `Error` type is compatible with everything so one mistake does not cascade into secondary errors.
- **Imports are checked against `FileAbi` only.** A file's body is checked against the exported signatures of its imports, never their bodies, which keeps each file an independent compilation unit. Symbols carry their declaring package so `package` visibility can be enforced (`E4007`).
- **Compose context is a stack.** Entering a `compose fun` pushes `true`. A closure pushes `true` or `false` depending on whether its expected function type is composable, so a normal callback such as `onClick` inside a composable cannot call composables (`E4008`). Checking this statically avoids slot-table failures at runtime.
- **Two passes per file, two phases per workspace.** Pass 1 registers all top-level symbols, so declarations can refer to each other in any order. Across the workspace, every file's ABI is extracted before any body is checked, so files in an import cycle can see each other. (Unannotated private declarations are now inferred on demand; see `typechecker-soundness` below.)
- **Nominal types are identities; names bind eagerly, definitions are looked up lazily** (`typechecker-soundness`, superseding the lazy resolution of `resolve-nominal-type-annotations`). A struct or enum type is a `NominalId` (defining module plus name) and carries no members; fields and variants live in the checker's type table. An annotation binds its name to an identity where it appears, and a member is looked up in the table where it is read. Lazy name resolution was right while types carried field lists, because eager resolution then meant cycle guards and whole-list equality; with identities neither cost exists, and equality is exact, so same-named types from different files, a struct and an enum, or a user `Range` and the built-in one, are never conflated. Every struct and enum name in a file is declared before any annotation is lowered, so annotations may name types declared later.
- **Type lookup skips values.** `lookup_type` returns the innermost struct or enum and ignores variables and functions, so a local `val User` cannot hide the struct `User`.
- **Unknown type names stay opaque for now.** Rejecting them (`E4001`) needs a builtin type list; the generic scoping it also needed came with `generics`, and the item is in `ROADMAP.md`. Until then, misuse surfaces as `E4006` at the first member access. A name known through an import's signature is not unknown: it reports `E4014` at the annotation (`shadowing-and-import-paths`). Since `explicit-library`, a name a library module declares as a public struct or enum is not opaque either: without its import the annotation is `E4001` with help naming the import (`explicit-library` D15), decided by the driver from the names the checker records as opaque, so the checker keeps no library knowledge. Rejecting every unknown name was tried then and left on the ROADMAP: the `E6001` scenarios of `code-generation` reach the backend only through unknown types (`Widget`, `Gadget`), and `examples/demo` names a `UInt64`.

Decisions from `typechecker-soundness`:

- **Operators read through references; value positions do not** (D1). Operator operands and conditions peel `&`/`&mut`, matching field access, which already read through borrows, and needing no new syntax. An explicit prefix `*` was rejected for now: it needs grammar work, and would make operators inconsistent with implicit field access. Value positions rejected a `&T` where a `T` is expected until `reference-access` added copy-out of `Copy` values and prefix `*` for writes (see the `references` spec); reads through `*` remain unnecessary.
- **A mismatch's code comes from where the expectation came from** (D2). The checker tracks an origin (value, return, argument): `E4005` for returned values, `E4004` for arguments, `E4002` otherwise. `if`/`when` branches and block tails inherit the origin, so a mismatch inside a returned branch is still `E4005`; every other nested expectation resets it. The alternative, `E4002` for every value mismatch (like Rust's single `E0308`), was rejected because the catalog already promised the dedicated codes and site-specific codes let tools offer site-specific fixes. Non-assignability errors such as a type name used as a value keep their own code.
- **Unannotated private declarations are inferred on demand** (D5). The first use of a pending declaration checks its body in a fresh body context (module scope only, its own return type and compose flag), and the result is memoized, so a type no longer depends on declaration order and no body is checked twice. Reaching a declaration whose inference is in progress is a self-dependency and reports `E4013`, as Kotlin does for recursive functions without a return type. Requiring annotations everywhere was rejected as removing a convenience the spec grants, and a dependency-ordered pre-pass as needing a second walk over every body.
- **Closure parameters need an expected type** (D6). Without one, nothing gives a parameter its type: inferring it from first use needs constraint solving across statements, which bidirectional checking avoids, and silently typing it as the error type left calls through the closure unchecked. So each such parameter reports `E4013`, bound to the error type so the body is still checked.
- **Imports form a layer below the module scope, and ambiguity is reported at use** (D7). Each imported name keeps one candidate per exporting module. A use with several candidates reports `E4012`; importing two modules that export the same name is fine while the name is unused, as in Kotlin and Go. Reporting at the import would forbid combining libraries that merely share a name, and first-import-wins was silent and order-dependent. A top-level declaration named like an import shadows it (see `shadowing-and-import-paths` below).
- **A `Unit` body discards its final value** (D8). The block body of a `Unit` function, or of a closure expected to return `Unit`, checks its final expression as a statement and ignores its type, so a body may end in a call that returns a value. Expression bodies are not discarded, because there the author wrote the value explicitly.
- **Members of a type need its module imported** (D9, decided by the user). A value whose type comes from a module the file does not import can be received and passed along, since assignability compares identities, but reading its members, matching its variants or constructing it reports `E4014` with the import to add. The checker therefore still reads only direct imports' ABIs, which keeps builds hermetic. Embedding each mentioned type's shape in the exporter's ABI (duplicating definitions) and relaxing the hermetic-builds invariant to read transitive ABIs were rejected.

Decisions from `harden-parser-grammar`:

- **Parameterless closures use the existing closure rules.** The type checker sees a zero-parameter `Closure` wherever source has braces in expression position, so trailing lambdas are checked against the parameter's function type and its compose flag, and a closure where a non-function is expected is a mismatch. Only the closure body's shape (always a block, checked like a block-bodied function's body) changed in the checker.
- **The compose flag comes from the written type.** `Type::from_ast` keeps `is_compose` from a `compose (…) -> T` annotation, and composable and plain function types stay mutually unassignable, so a composable content lambda can call composables while a plain callback such as `onClick` cannot.

Decisions from `shadowing-and-import-paths` (decided by the user):

- **Locals may shadow parameters.** Parameters form a scope level above the body, so `val a` in the body shadows parameter `a` from its declaration on, as in Rust. Kotlin warns instead; Prism has no warnings channel.
- **Top-level declarations shadow imports.** Lookups check module scope before the import layer, so a file's own `helper` wins over an imported one for values, callees and type names, and also resolves what would otherwise be an `E4012` ambiguity, as a file's own declarations win over star imports in Kotlin. Two declarations in the same scope still report `E4010`.
- **Naming a type requires importing its module** (extending D9). A type name known only through an import's signature reports `E4014` with the import hint at the annotation, and binds to the error type so the annotated value causes no further errors.

Decisions from `statements-and-loops`:

- **One `Range` type for both range operators.** `a..<b` and `a..b` both have the built-in `Range` type, and `in a..<b` patterns are checked like `in a..b`. Whether the end is included is a property of the runtime value, so `when` patterns, `for` loops and future APIs treat both alike. Two range types were rejected as duplicating every API.
- **A block without a tail is `Unit`.** When a value is expected, the mismatch is reported at the block's last statement, where the value is missing, with the code of the expectation's site (`E4005` for a function body).

Decisions from `reference-access`:

- **References are invariant in their referent** (user). `&U` is assignable to `&T` only for the same type, except that `&mut` may weaken to `&` at the outer layer and behind shared borrows; `&mut U` to `&mut T` needs equal types. A reference cannot change the representation of what it points at (`&Int` is not a `&(Int?)`), so covariance would have hidden a converted copy behind a borrow. `Heap<T>` is invariant for the same reason (`heap-boxes`, below).
- **Value positions copy `Copy` values out of borrows.** The "Operators read through references; value positions do not" rule above now ends at copy-out: where a `T` is expected, a `&T` of a `Copy` type is accepted, and a non-`Copy` one is `E4017` (see `references`).

Decisions from the `Bool` plain fixes (2026-10-18, logged in `docs/decisions-for-review.md`):

- **Only `Int`, `Double` and `Char` are ordered.** `false < true` is `E4002`, which Kotlin allows; `Bool` has no natural ordering that a program should rely on, and `String` is unordered until comparison becomes a trait (a type parameter's ordering is `E4002` for the same reason). Allowing it later is compatible. An unordered or mixed comparison is reported through `OperatorMismatch`, as compound assignment's operators are, not as a type mismatch between two identical types; the other binary operators still report `TypeMismatch` (ROADMAP).
- **`true` and `false` cover a `Bool`, and with `null` a `Bool?`.** Only the literals count, as only a variant's name covers a variant; a constant or a condition that happens to be `true` covers nothing. No new typed-tree or MIR concept is needed: the patterns stay value tests and the exhaustive flag makes the fall-through unreachable.

Decisions from `compound-assignment` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`t op= v` evaluates the place, reads it, then evaluates the value.** This is Kotlin's and Java's order (`a[i] += f()` reads `a[i]` before calling `f`) and what `t = t op v` already does under "Operands Are Evaluated in Order", so `n += inc(&mut n)` adds the old `n`. Rejected: Rust's order for primitives (value first, place last), which Rust reverses for overloaded operators; desugaring with the target duplicated, which evaluates a call or an index twice. Plain `=` still evaluates its value first; it never reads the old value, so the orders cannot be confused.
- **The operator follows the binary operator rules, and the value is a non-consumed operand.** `Int` with `Int`, `Double` with `Double`, and for `+=` `String` with `String`; the result always has the target's type, so no extra assignability check is needed. The value is read through references and in place (`s += t` leaves `t` usable), as Kotlin's `s += "x"` suggests. `String += String` means `s = s + t`; an in-place append is observably identical, so the backend may implement it later (ROADMAP). Rejected: consuming the value; rejecting `String +=` until an append exists.
- **Compound-assignment diagnostics reuse existing codes.** Non-writable targets share `=`'s target typing and its `E4003` messages. An operator that does not apply is `E4002` through `OperatorMismatch`, whose message names the operator and both operand types, with help naming the accepted operand types, instead of `TypeMismatch`'s "expected 'String', found 'String'"; it is the same kind of error as an operand mismatch, so no new code. A reference binding target (`r += 1`) is rejected as `r = 5` is, with help suggesting `*r += 1`. The binary operators still report `TypeMismatch`; moving them is a follow-up (ROADMAP).

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`print` and `println` are compiler built-ins, found last** (D15). Prism has no overloading, traits or prelude through which a library function could accept several types, so the type checker resolves a bare `print` or `println` callee as the built-in only when no local, top-level declaration or import of that name exists, like the built-in `Range` type; fixtures declaring their own `fun print(n: &Int)` keep their meaning. A `Copy` argument is copied out of references, a `String` borrowed like a `&String` argument, anything else is `E4004` with help listing the printable types, and a built-in named without a call is `E4001`. `String` arguments typecheck now and wait for `runtime-strings-and-drops` in the backend (they run since that change). Rejected: a Prism-written prelude (needs overloading or traits), string templates (need strings). Since `standard-library`, `print` and `println` are rule-typed intrinsics of `/prism/core`, named without a call `E4013`, and accept any argument with a text form (`E4026` otherwise); since `explicit-library` a file imports `/prism/core` to call them, and its own declaration of either shadows the import.

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Infinitely sized types are a type error** (D20). A struct or enum that contains itself by value, through fields, payloads, nullables and other structs and enums with their arguments substituted, but not through `Heap<T>`, references or function types, is `E4023` at its name, with a note naming the path and help suggesting a `Heap<...>` field. Each declaration is walked over its own type parameters; a cycle that does not lead back to the declaration is reported at the types on it, not at every type holding one, and a depth limit of 64 and a size limit of 256 type nodes only guard termination for generic types growing their own arguments (`struct W<T> { val w: W<Pair<T, T>>? }` is reported at once). A cycle across modules is visible because the ABI carries reachable definitions, and each file declaring one of its types reports it when checked. Rejected: rejecting it in the backend (it is wrong in every tier, and `prism check` should say so), a parse-time check, a user-visible recursion limit.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A string's character is a value, not a place** (D5). A `Char` decoded from UTF-8 has no address, so `s[i]` on a `String` is a call of the built-in `StringCharacter` with a shared receiver borrow and the index, like `clone()`, and `is_place` is false for an index expression. `&s[i]` borrows a temporary (see `references`), and `s[i] = c` is `E4003` with help saying that strings are immutable. Rejected: keeping a place that cannot be borrowed (a special case in every analysis), or copying the character out in the MIR builder (the borrow checker would still see a borrow of `s`).
- **Built-in `length` and `toString()`** (D6). `s.length` is a built-in member read without parentheses, through `&`, `&mut`, `Heap<T>` and `?.`, counting as a field of `String` for member precedence (`s.length()` is `E4004`); `x.toString()` is a built-in member function of `Int`, `Double`, `Bool` and `Char`, through references and `?.`, giving the text `print(x)` writes, and wins over extensions. On every other type `toString` is an ordinary name, so `fun Point.toString(): String` is how a struct will display in templates. Rejected: `String.toString()` (a clone under another name), a generated `toString()` for every type (no text form committed for aggregates), a library `fun Int.toString()` (no prelude), `length()` or `size`. Since `standard-library` both are intrinsic declarations of `/prism/core` (`public intrinsic val String.length: Int`, `public intrinsic fun Int.toString(): String`), members by its rule for intrinsic extensions.
- **The receiver of `s[i]` and `s.length` is borrowed after the other arguments** (D22). Both are calls of built-ins with a receiver borrow, so the index of `s[i]` is evaluated before `s` is borrowed, as for every receiver; the order is observable only through an index that reassigns the string (`s[{ s = "cd"; 0 }]` reads the new `s`, where Kotlin reads the old one). Rejected: a special evaluation order for the built-in members alone.
- **`s.length = n` is `E4003`** (D22): `length` counts as an immutable field, so assigning it reports "cannot assign to immutable 'val' struct field 'length'". Rejected: `E4006`, a string-specific message.

Decisions from `heap-boxes` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **`Heap(value)` is a built-in constructor** (D2). It reads as the type's constructor, as Kotlin and Swift users expect, and is a call of the built-in `HeapAllocate`. The contents' type comes from an explicit type argument (`Heap<Int?>(1)`), else the expected type, else the argument, which is checked in a consuming position, so `1` is wrapped into `Int?` before it is boxed. `Heap` resolves like a type name in call position, so no local hides it, and a top-level declaration named `Heap` could never be named: `E4010`. Rejected: a free function `box(v)`, Rust's `Box::new` (no associated functions), a `heap`/`new` keyword, and letting user declarations shadow it like `print`.
- **Boxes are invariant** (D8), like references and generic types: `Heap<Int>` and `Heap<Int?>` hold contents of different layouts, so covariance would need a re-allocating adapter behind an assignment. The contents are converted where the box is built instead, and the mismatch help says so. After this the typed tree's only representation-changing `Subtype` step is function-type variance. Rejected: covariance with a re-boxing adapter; covariance for representation-preserving steps only (there are none left in a box, whose contents are borrow-free).
- **Operators, conditions, output and `toString()` read through boxes** (D4), as through references, so `h + 1` and `if (flag)` need nothing written.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A part means what `.toString()` means, without a move or mutation** (D6). After reading through `&`, `&mut` and `Heap<T>` layers, a `String` is read in place, `Int`, `Double`, `Bool` and `Char` give the text `print` writes, `Nothing` is accepted, and any other type resolves `part.toString()` exactly as the method call would (a field of function type, then an extension of the file or an import, `E4012` and `E4007` as for an explicit call), so a template and an explicit call never run different code. The member must need no arguments and return `String`, and an extension must take a shared receiver: a `mut` or `consuming` `toString()` would mutate or move a part that looks borrowed. The check reuses the receiver half of the method-call resolution (`method_call_on`), so it costs one member lookup per non-scalar, non-`String` part, and nothing is inferred across parts. A generic `toString()` extension is a generic call, which runs as an instance since `monomorphization`. Rejected: extensions only, calling `mut`/`consuming` extensions, a generated text form, and extending `print` in the same change.
- **Nullable parts are rejected** (D7). Kotlin prints `null`; Prism rejects every `T?` part, `null` included, since "Hello null" is a bug users see and accepting nullable parts later breaks no program, while rejecting them later would. The fallback costs one `?:`. Note that `?:` consumes a place operand (see `ownership`), so `${name ?: "guest"}` moves a non-`Copy` `name`; `${&name ?: &"guest"}` reads it in place. Rejected: Kotlin's `null`, Swift's warning (no warnings yet), printing nothing.
- **One code, `E4026`, whose help names the fix** (D8). A part is not a call argument (`E4004`) and the user never wrote `toString` (`E4006`). The message names the type and the reason, and the helps are a fallback with `?:` written with the part, the extension to declare, the braced member when `$u.name` or `${u}.name` was written, or the explicit `${c.toString()}`. A part that binds less tightly than `?:` and `.` (an `if`, a `when`) is parenthesized in the help, so following it keeps the part whole (review fix). A template whose part has an error still has type `String`. Rejected: one code per reason.

Decisions from `closures` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Function values convert only without changing representation** (D8, BREAKING; settles the risk `typed-tree` D6 left). The function-type steps `T` to `T?` and `Nothing` to `T` change a representation: `(Int?) -> Unit` used as `(Int) -> Unit` must receive an `Int` where its code reads an address of an `Int?`. A converted value would need adapter code holding it, by borrow (a temporary that dies with the statement) or by move (a hidden heap allocation per conversion, as `references` and `heap-boxes` refused for references and boxes). So a function value converts only when each parameter and the result are equal or convert by `&mut T` to `&T`, also inside nullables and function types (`Type::converts_in_place`), and the mismatch of its site is reported otherwise, with help writing the closure that converts it. A named function or variant constructor converts freely, since its adapter (`mir` D7) does the conversion, except a function-typed parameter or result, which the adapter would have to convert as a value. A closure literal is checked against its expected type, so it never needs one. Branch joins (`if`, `when`, `?:`) apply the rule too, as `E4002` without help. Lending a function value converts nothing: a lent place keeps its type, since references are invariant, and a lent temporary is converted to the referent first, under the same rule (found in review). Rejected: adapters for every function value, no conversion even for named functions (`each(show)` is ordinary Kotlin), and rejecting in the backend (`E6001` for a program `prism check` accepts).

Decisions from `escaping-closures` (planned autonomously on 2026-10-18, revised for compile speed on 2026-10-19, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`escaping` is part of a function's type, contravariantly** (D3). `Type::Fn` carries one flag per parameter, rendered `(escaping () -> Unit) -> Unit`. A function whose parameter is unmarked is assignable to the type whose parameter is `escaping` (a function accepting any function value accepts an owned one), in place, since both move their argument and are called alike; the reverse is a mismatch, also nested, inside nullables and across branch joins. A closure literal takes its parameters' flags from the expected type. The cost is one comparison per parameter in assignability.

Decisions from `implicit-it` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A header-less closure has `it` exactly when its expected function type has one parameter** (D1), Kotlin's rule, decided in `check_closure` from the header and the expected type alone, wherever a closure is checked against a function type (a parameter or trailing lambda, a reference parameter's referent, an annotation, a return type, a deferred closure of a generic call once its parameter types are solved, a field or payload, a named or default argument, and a nullable function type, whose closure is checked against the function type and then made nullable, found in review). The cost is one comparison per closure: nothing is inferred from the body, re-checked or backtracked. Rejected: inferring `it` from the body without an expected type (`val f = { it + 1 }`, constraint solving that D6 of `typechecker-soundness` refused for declared parameters too); deciding from whether the body mentions `it` (a body scan per closure, and Kotlin accepts a closure that ignores its `it`); re-checking a body as one-parameter after a zero-parameter attempt (speculative, quadratic on nesting).
- **`it` is bound like a declared parameter, and shadows by scope** (D3, D4). It is a `val` of exactly the expected parameter's type with its `escaping` flag, in the scope the closure opens, so copy-out, reborrows, writes through `&mut`, drops, `E5007` and the elision rules follow from `{ it -> ... }`, and the implicit `it` shadows any outer `it`, including an enclosing closure's, while a closure without its own `it` sees the enclosing one (`xs.forEach { run { println(it) } }`), as in Kotlin. Rejected: rejecting a nested `it` that shadows another (Kotlin only warns; a lint once warnings exist), and Swift's per-closure `$0`, which breaks `run { it }`.
- **Diagnostics without new codes** (D6). An unbound `it` is `E4001` with a help naming why the innermost closure has none (its type, such as `() -> Int`, has no parameters; it has no expected function type; it declares its parameters; or the use is in no closure), computed only on that error path from state the closure scope already saves. A closure whose parameter count does not fit is `E4004` (`ClosureParameterCount`), naming the expected types and the header to write, and saying for a header-less closure expecting two or more that only a closure expecting one has an implicit `it`. Rejected: `E4001` at a use of `it` in a closure expecting several parameters, which would give one mistake two codes.

Decisions from the trailing-lambda binding fix (a plain fix of 2026-10-18; the syntax spec already passed the trailing lambda "as the final argument"):

- **A trailing lambda binds to the last parameter, as in Kotlin.** The parser marks the argument (`CallArg::is_trailing_lambda`), and argument matching binds it to the last parameter, field or payload after the parenthesized arguments bind as they would alone, so `Button(onClick = { }) { }` fills `content` and lets `label` take its default. Before, the lambda was one more positional argument and filled the first unset parameter, so the call bound `onClick` twice or the lambda to `label`. The cost is one slot check per call. A last parameter already given, or a callee without parameters, is `E4004` naming that parameter, like the other twice-assigned arguments. Rejected: binding to the first unset parameter (not Kotlin, and it makes a content lambda depend on which earlier parameters are written), and a new code for a conflict that is one more kind of call mismatch.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **The receiver is the first parameter, and the flag is part of the type** (D2). `Type::Fn` has `receiver: bool`; when set, `params[0]` is `&T` or `&mut T`, so everything that walks parameters (the borrow checker, MIR, the backend, change words) needs no change. The flag decides how a lambda is checked against the type, so it is part of its identity and display (`ColumnScope.(Int) -> Unit`). Rejected: a separate receiver field beside `params`, which every consumer counting parameters would need to handle, and a flag outside identity, which would have to travel through inference and substitution beside the type.
- **Receiver and plain function types convert in place, both ways** (D2). `T.(A) -> R` and `(&T, A) -> R` share one representation, so `converts_in_place` ignores the flag, as Kotlin lets `T.() -> R` and `(T) -> R` stand for each other; the other rules apply to the receiver's parameter as to any, so a shared receiver may become a mutable one but never the reverse. Glue names write the receiver as source does, since glue is keyed by type and a unit may need both forms; the contract hash, which describes representations, ignores it. Rejected: no conversion, which would need a calling closure to pass a `(&ColumnScope) -> Unit` as content.
- **A lambda against a receiver function type binds `this`** (D3). The closure binds `this` in its own scope before its parameters, as a `val` of the receiver's parameter type declared at its `{`, whatever the expected type's source (a parameter, an annotation, a return type, a field, a deferred closure of a generic call, a nullable function type). Its header and the implicit `it` count the other parameters only. A closure with no expected type, or checked against a plain function type, has no receiver of its own. Rejected: rewriting the lambda in the parser (it does not know the expected type). Cost: one binding per receiver lambda.
- **Receiver function values are called in Kotlin's three ways** (D5): `f(r, a)` as an ordinary call; `r.f(a)`, where a local or parameter `f` of a receiver function type whose receiver is `r`'s type is considered after members and before extensions (Kotlin resolves locals first, and a parameter named like an imported extension would otherwise be unreachable in method form); and `f(a)` with the innermost `this` as the receiver when its type matches, which is how Compose's scopes call their content. Without such a `this`, the call is `E4004`, naming the receiver type and the two ways to pass one. Cost: one scoped lookup for a method call that found no member, and one comparison for `f(a)`.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **Default values are not composable contexts** (D17). A default's generator runs in the caller before the call, outside the callee's groups, and is not a composable body, so a composable call in a default is `E4008` with a note. Kotlin allows composable defaults (`LocalContentColor.current`), which would need composer parameters on generators and groups in the caller; with no `CompositionLocal` yet, they are a follow-up.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`MutableState<T>` converts to `State<T>`** (D3). One built-in subtyping edge, in the positions where `&mut T` converts to `&T`, keeping the word (`Rvalue::Subtype`), with type arguments inferred through it; a member or extension lookup that misses on a `MutableState` receiver is retried on `State`. It costs one constructor comparison in assignability and one more hash lookup on a miss. Rejected: a general nominal subtyping or interface mechanism before traits.

Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **Companion values** (D4). A top-level `val` may share its name with a struct of the same file: a type position or a call means the struct, any other value position the value, so `Modifier.padding(16.dp)`, `Color.Red` beside `Color(0xFF336699)` and `TextStyle.Default` read as in Compose without Kotlin's `companion object`. Every other same-name pair stays `E4010`, a pair across files included, so an import never silently changes what a local struct name means; a `val` and an extension of one name coexist, as an extension is a member of its receiver. The checker defines the pair through `define_companion`, the value being the name's innermost binding, and a call or struct pattern finds the struct through `lookup_constructor`; the cost is one filtered lookup and one kind compare. A prelude module cannot give a struct a companion, since the prelude layer keeps one symbol per name. Rejected: struct field defaults (`Modifier()`), enum colours, static members, pairs across files.
- **Intrinsic properties of `Int`** (apply). `n.dp` and `n.sp` are found through the member table of the file and its imports, as `String` and `List` members are, and typed as a call of their built-in with the `Int` copied, so any later unit property needs no new rule. Rejected: special-casing `dp` and `sp` by name.

Decisions from `owned-state` (review items A2′, A2‴ and L5 of A4, agreed with the user on 2026-10-20 in `docs/milestone-2-review-notes.md`: `MutableState<T>` an owning state object as in Compose, no stale-state errors in UI code, weak capture declared by the library; the rest decided while proposing, implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`attached` in function types converts contravariantly** (D9). A function type converts to one where an unmarked parameter is `attached`, or an `attached` one `escaping`, and nothing else converts; `MutableState<T>` converts to `State<T>` by value and behind a shared reference, keeping the representation (D5). `remember`'s result is typed by the catalog's rule: a copy of a `Copy` value, else a shared borrow (D6).
