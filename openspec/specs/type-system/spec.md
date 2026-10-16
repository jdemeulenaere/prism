# Type System Specification

## Purpose
The type system defines semantic analysis, lexical symbol resolution, nominal type definitions, bidirectional type checking, and expression/statement validation for Prism programs. Error codes are listed in the [`diagnostics`](../diagnostics/spec.md) spec.

## Requirements

### Requirement: Lexical Scope and Symbol Resolution
The type checker MUST resolve identifiers across nested lexical scopes (local blocks, parameters, file declarations, and imported ABI symbols) and reject unresolved or duplicate symbols.

#### Scenario: Local variable shadowing and resolution
- **WHEN** an expression references an identifier declared in an outer or enclosing block
- **THEN** the identifier resolves to the innermost matching declaration

#### Scenario: Unknown identifier error reporting
- **WHEN** an expression references an undeclared identifier
- **THEN** the type checker emits an unresolved symbol error (`E4001`)

#### Scenario: Duplicate symbol declaration rejected
- **WHEN** a scope declares two variables or parameters with the identical identifier
- **THEN** the type checker rejects the duplicate declaration with `DuplicateSymbol` (`E4010`)

### Requirement: Static Type Inference and Assignment Compatibility
The type checker MUST infer types for unannotated `val` and `var` bindings, enforce strict numeric typing without implicit coercion, and verify assignment compatibility for references, nullable types, and struct fields.

#### Scenario: Primitive type inference
- **WHEN** `val x = 42`, `val c = 'a'`, or `val b = true` is declared without an explicit type annotation
- **THEN** the type checker infers `Int`, `Char`, and `Bool` respectively

#### Scenario: Mismatched type assignment rejected
- **WHEN** a variable declared as `Int` is assigned an expression of type `String`
- **THEN** the type checker rejects the assignment with a type mismatch error (`E4002`)

#### Scenario: Implicit numeric widening prohibited
- **WHEN** an arithmetic operation mixes numeric types (e.g. `val x: Float = 1 + 2.0`)
- **THEN** the type checker rejects the operation with a type mismatch error (`E4002`)

#### Scenario: Reassignment to immutable val rejected
- **WHEN** code attempts to reassign to a variable declared with `val`
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Struct field mutability validation
- **WHEN** code attempts to assign to a struct field declared with `val`, or mutates a field on an immutable `val` struct instance
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Reference and borrow compatibility
- **WHEN** passing an argument to a parameter expecting `&T` or `&mut T`
- **THEN** the argument must provide a matching borrow (with `&mut T` coercible to `&T`), and taking `&mut` on an immutable `val` binding is rejected

#### Scenario: Nullable type compatibility
- **WHEN** `null` is assigned to a variable of type `T?`
- **THEN** the assignment succeeds; whereas assigning `null` to non-nullable `T` emits `E4002`

#### Scenario: Safe navigation and Elvis operator evaluation
- **WHEN** accessing a field via safe navigation `opt?.field` on `T?` or coalescing via `opt ?: default`
- **THEN** `opt?.field` evaluates to `FieldType?` and `opt ?: default` evaluates to `T` when `default` is of type `T`

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

#### Scenario: Diverging expression has bottom type Nothing
- **WHEN** an expression diverges (such as a `return` expression) inside an `if` or `when` branch
- **THEN** the diverging branch has bottom type `Nothing`, which unifies with any expected target branch type (e.g. `val x: Int = if (cond) return 0 else 42`)

#### Scenario: Subject when expression branch checking
- **WHEN** evaluating `when (subject) { p1 -> e1, p2 -> e2, else -> e3 }`
- **THEN** pattern expressions `p1` and `p2` must match the type of `subject`, and branch results unify to a common type

#### Scenario: Subjectless when expression condition checking
- **WHEN** evaluating `when { c1 -> e1, c2 -> e2, else -> e3 }`
- **THEN** conditions `c1` and `c2` must each evaluate to `Bool`

#### Scenario: Non-exhaustive when expression rejected
- **WHEN** a `when` expression is used in value position and fails to cover all possible enum variants or omit an `else` branch
- **THEN** the type checker rejects the expression with a non-exhaustive pattern match error (`E4009`)

#### Scenario: Block statement evaluation and Unit typing
- **WHEN** evaluating a block of statements `{ s1; s2 }`
- **THEN** assignment statements evaluate to `Unit`, an empty block `{}` evaluates to `Unit`, and the block's final expression determines the block type

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

#### Scenario: Unknown nominal type name stays opaque
- **WHEN** an annotation names a type that is neither a primitive nor a declared or imported struct or enum
- **THEN** the annotation is accepted as an opaque nominal type, and accessing a member on a value of that type emits `E4006`

### Requirement: Cross-Module Imported Symbol and Visibility Validation
Phase 2 body checking MUST typecheck expressions against symbols exported in Phase 1 `FileAbi` from imported modules and enforce visibility boundaries.

#### Scenario: Accessing exported public function from imported package
- **WHEN** a file imports another module and calls an exported `public` function
- **THEN** the call resolves against the imported `FileAbi` signature and typechecks correctly

#### Scenario: Accessing package-visibility symbol across packages rejected
- **WHEN** a file imports a module from a different package and attempts to access a declaration marked with `package` visibility
- **THEN** the type checker rejects the access with a visibility access violation error (`E4007`)

#### Scenario: Accessing package-visibility symbol within same package succeeds
- **WHEN** a file accesses a `package`-visible declaration defined in another file within the identical package directory
- **THEN** the declaration resolves and typechecks successfully

## Design Rationale
Decisions from `type-checker-core` and `resolve-nominal-type-annotations`:

- **A separate `prism-typechecker` crate.** Semantic analysis stays decoupled from the grammar and from module resolution, and can be tested with in-memory ASTs.
- **Bidirectional checking, not Hindley-Milner.** Exported signatures and parameters always carry types, so local `infer`/`check` modes are enough. They are fast and deterministic and give precise errors without unification traces.
- **A small subtyping lattice.** `Nothing` is assignable to every type, `T` to `T?`, `T?` to `U?` when `T` is assignable to `U`, and `&mut T` to `&T`. Everything else is invariant: in particular `Int` never widens to `Float`. An `Error` type is compatible with everything so one mistake does not cascade into secondary errors.
- **Imports are checked against `FileAbi` only.** A file's body is checked against the exported signatures of its imports, never their bodies, which keeps each file an independent compilation unit. Symbols carry their declaring package so `package` visibility can be enforced (`E4007`).
- **Compose context is a stack.** Entering a `compose fun` pushes `true`. A closure pushes `true` or `false` depending on whether its expected function type is composable, so a normal callback such as `onClick` inside a composable cannot call composables (`E4008`). Checking this statically avoids slot-table failures at runtime.
- **Two passes per file, two phases per workspace.** Pass 1 registers all top-level symbols (private expression-bodied functions get their return type inferred in Pass 2), so declarations can refer to each other in any order. Across the workspace, every file's ABI is extracted before any body is checked, so files in an import cycle can see each other.
- **Nominal types resolve lazily.** An annotation such as `user: User` stays a field-less nominal type and is resolved against the type namespace only where a member is read. Eager resolution was rejected: it resolves only one level (`order.user.name` still needs a second lookup), needs cycle guards for recursive types, and would make `Type` equality compare whole field lists.
- **Type lookup skips values.** `lookup_type` returns the innermost struct or enum and ignores variables and functions, so a local `val User` cannot hide the struct `User`.
- **Unknown type names stay opaque for now.** Rejecting them (`E4001`) needs a builtin type list and generic scoping. Until then, misuse surfaces as `E4006` at the first member access.
