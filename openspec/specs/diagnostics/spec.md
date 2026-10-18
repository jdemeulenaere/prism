# Prism Diagnostic Codes

## Purpose
The diagnostics specification is the single catalog of compiler error codes. Each code identifies one kind of error across all stages (resolution, parsing, ABI extraction, type checking and borrow checking), so tools and tests can key on codes instead of message text. The other specs reference these codes in their scenarios; this spec lists them all in one place.

## Requirements

### Requirement: Stable Diagnostic Codes
Every error the compiler reports MUST carry a code from the catalog below. A code MUST keep its meaning once assigned, and a retired code MUST NOT be reused.

#### Scenario: Rendered diagnostic header
- **WHEN** `prism check` or `prism run` reports an error
- **THEN** the diagnostic starts with `error[<code>]: <title>`, followed by the source location when one is known

#### Scenario: Syntax error location and lexer message
- **WHEN** `prism check` reports an `E2xxx` error, including one in an imported file
- **THEN** the diagnostic points at the error's own line and column, and a lexer error shows the lexer's message (`Unknown escape sequence '\q'`) rather than the parser's view of an error token

#### Scenario: Code range identifies the stage
- **WHEN** a diagnostic is reported
- **THEN** its code falls in the range of the stage that produced it: `E1xxx` resolution and I/O, `E2xxx` lexing and parsing, `E3xxx` Phase 1 ABI extraction, `E4xxx` Phase 2 type checking, `E5xxx` Phase 2 borrow checking, `E6xxx` code generation and the program's entry point

#### Scenario: New error kind gets a new code
- **WHEN** a change introduces a new kind of error
- **THEN** it takes the next unused code in its stage's range, adds it to this catalog, and does not reuse a retired code

#### Scenario: Runtime errors carry no code
- **WHEN** a running program raises a runtime error (see `execution`)
- **THEN** it is reported as `panic: <message>` with its location, without an `error[<code>]` header, because it is not a compiler error

### Requirement: Diagnostics Show Related Locations
When a diagnostic has a label for its primary location or related locations, such as where a used value was moved, the CLI MUST render the primary location's snippet with its label, then each related location in source order with its own location line, snippet and label, then the diagnostic's notes and help lines.

#### Scenario: Rendered related location
- **WHEN** `prism check` reports an `E5001` for `fun take(s: String) {}` and `fun f(s: String) { take(s); take(s) }`
- **THEN** after the header, the location and snippet of the second `s` labelled "value used here after move", it prints the location and snippet of the first `s` labelled "value moved here", then a `note` line and a `help` line

## Catalog

### E1xxx: Resolution and I/O (`crates/prism-resolver`, `crates/prism-cli`)

| Code | Name | Meaning |
|---|---|---|
| `E1000` | `Io` | A file or directory could not be read, or a `prism check` target does not exist. |
| `E1001` | `PackagePathMismatch` | The declared `package` does not match the file's directory path, including a case-only mismatch or more package segments than directories. |
| `E1002` | `FileNotFound` | An import targets a file that does not exist with the exact casing on disk. |
| `E1003` | `PackageHeterogeneity` | Files directly inside one directory declare different packages. |
| `E1004` | `InconsistentWorkspaceRoot` | A dependency resolves to a different workspace root than the entry file. |
| `E1005` | `WorkspaceBoundaryViolation` | An import, entry file or scanned file resolves outside the workspace root, for example through a symlink. |
| `E1006` | `InvalidImportPath` | A file imports itself, or a sibling import has more than one segment. `.` and `..` segments are rejected earlier by the parser (`E2001`). |
| `E1007` | (retired) | Cyclic dependency. Retired by `fix-cli-workspace-audit`, which allowed mutually recursive imports. Do not reuse. |

### E2xxx: Lexing and parsing (`crates/prism-lexer`, `crates/prism-parser`)

| Code | Name | Meaning |
|---|---|---|
| `E2001` | syntax error | The lexer produced an error token or the parser rejected the source, including an assignment or compound assignment (`x += 1`) used as a value and the reserved `++` and `--`. The parser stops at the first error. |
| `E2002` | `NestingTooDeep` | Expressions or types nest deeper than 256 levels, counting each operator of a left-associative chain such as `a + a + ... + a` as one level. |

### E3xxx: Phase 1 ABI extraction (`crates/prism-abi`)

| Code | Name | Meaning |
|---|---|---|
| `E3001` | `MissingExplicitReturnType`, `MissingExplicitVarType` | An exported declaration lacks a type required for ABI extraction: an expression-bodied `public`/`package` function without a return type, or an exported top-level variable without a type. |

### E4xxx: Phase 2 type checking (`crates/prism-typechecker`)

| Code | Name | Meaning |
|---|---|---|
| `E4001` | `UnresolvedSymbol`, `ThisOutsideExtension` | An identifier is not found in any enclosing scope or imported ABI, or `this` is used outside an extension function. An extension is never found by its bare name, except on the implicit receiver of an extension body. |
| `E4002` | `TypeMismatch`, `OperatorMismatch`, `TypeNameAsValue` | An expression's type is not assignable to the type expected by an annotation, assignment, operator operand or default value (return and argument sites have their own codes), including implicit numeric widening and `null` for a non-nullable type; a compound assignment's operator does not apply to its target's and value's types (`s -= "a"`, `n += 1` for `n: Int?`), or it would re-point a `var` reference binding (`r += 1`); or a struct or enum name is used as a value. |
| `E4003` | `ImmutabilityViolation` | Reassigning a `val` (also by a compound assignment such as `n += 1`), assigning a `val` field, mutating a field of a `val` instance, taking `&mut` of a `val` (also one holding a `&mut T`, which is reborrowed by passing it instead), or writing through a shared `&` borrow (`*r = v`). Also the receiver of a `mut fun` call that is not a mutable place (a `val` binding, a `val` field, a place reached through a shared `&`), including `this` in a `fun` or `consuming fun`. |
| `E4004` | `CallSignatureMismatch`, `ArgumentTypeMismatch` | Wrong argument count, an argument whose type is not assignable to its parameter (positional, named or trailing-lambda argument, struct constructor field, enum payload), a missing, duplicate or twice-assigned named argument, or a missing or duplicate struct field in a constructor call. |
| `E4005` | `ReturnTypeMismatch` | A returned value (a `return` operand, an expression body, the final expression of a non-`Unit` block body, or a closure body) is incompatible with the declared or expected return type, including a value returned from a `Unit` function and a `return` outside a function. |
| `E4006` | `UnknownField` | A field does not exist on the struct, or member access on a non-struct or opaque type, including a member of a type parameter, which has none until traits exist. Also a method call `x.name()` with neither a field nor a visible extension function `name`, a `.` call of an extension on a nullable receiver (help suggests `?.`), an extension read without a call (help suggests the call), and a call of an extension whose declared receiver the receiver does not match (`Box<Int>.inc` on a `Box<String>`; help names the declared receiver). |
| `E4007` | `VisibilityViolation` | A `package`-visible symbol is accessed from another package. |
| `E4008` | `InvalidComposeContext` | A `compose fun` is called outside a composable function or closure. |
| `E4009` | `NonExhaustiveWhen` | A `when` in value position covers neither every enum variant nor has an `else`. |
| `E4010` | `DuplicateSymbol`, `UncallableExtension`, `InvalidTypeParameterName` | Two declarations in the same scope share a name, including a top-level declaration with the same name as an imported symbol. Also a duplicate type parameter of one declaration, or one named like a built-in type (`Int`, `Float`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `Heap`), two extensions in one file with the same name on the same receiver type constructor (`fun Box<Int>.size()` and `fun <T> Box<T>.size()`), and an extension that could never be called: one named like a field of its receiver struct, or named `clone` on a `Clone` type. |
| `E4011` | `MissingReturnType` | A `public` or `package` function omits its return type. |
| `E4012` | `AmbiguousImport` | A name the file uses (as a value, callee or type) is exported by more than one of its imports, or an extension called on a receiver type is exported by more than one import. Importing both without using the name is accepted. |
| `E4013` | `CannotInferType` | A type cannot be inferred and must be written: a closure parameter with no expected function type, an unannotated private declaration whose inferred type depends on itself (such as a recursive function without a return type), a type argument of a generic call that neither the receiver, the expected type nor the arguments solve (`val o = none()`, `val o = Option.None`; help suggests writing the type arguments or annotating the expected type), or a generic function or variant constructor used as a value (`val k = keep`). |
| `E4014` | `MemberOfUnimportedType` | A member of a struct or enum is used (a field read or written, its variants matched, the type constructed or a variant named) but the module defining the type is not a direct import; the message names the import to add. Receiving and passing along such a value is accepted. |
| `E4015` | `BreakOutsideLoop` | A `break` or `continue` has no enclosing loop in its function or closure body, including one inside a closure that sits in a loop. |
| `E4016` | `InvalidMarkerDeclaration` | A `: Copy` struct or enum has a field or variant payload that is not `Copy`, or a `: Clone` one has a member that is not `Clone`, where each type parameter occurring in a member type counts as satisfying the marker (`struct Box<T> : Copy { val v: T }` is accepted, `struct W<T> : Copy { val h: Heap<T> }` is not). Reported once per offending member. |
| `E4017` | `CannotMoveOut` | A non-`Copy` value in a consuming position (an initializer, an assigned value, a by-value argument, a returned value) is taken from a field (also through a borrow, `Heap<T>` or a temporary), a safe navigation, an index or a top-level `val`, or out of a borrow given where a value is expected (`take(u)` for `u: &User`). The receiver of a `consuming fun` call is a consuming position too, and so is a parameter's default value, inside which an earlier parameter or `this` is borrowed and cannot move (`fun f(a: String, b: String = a)`). Inside a closure, a variable the closure captures is borrowed and cannot move either (`run { take(s) }`). A move must take a whole local, parameter or temporary; only a `Copy` value is copied out of a borrow. |
| `E4018` | `NotCloneable` | `clone()` is called on a value whose type is not `Clone`: a struct or enum without `: Clone` or `: Copy` (or an instantiation whose marker parameters' arguments are not `Clone`), a function value, an unbounded type parameter (help suggests the bound `<T : Clone>`) or an unknown type. |
| `E4019` | `NotAReference` | Prefix `*` writes through a value that is not a reference (`*x = 1` for `x: Int`). `*` is only allowed on the left of `=`; reads through references are implicit. |
| `E4020` | `WrongTypeArgumentCount` | A generic type, `Heap` or type parameter is written with the wrong number of type arguments (`val b: Box`, `Box<Int, Int>`, `Heap<A, B>`, `T<Int>`), or a call gives the wrong number of explicit type arguments, or any to a callee that is not generic (`id<Int, Int>(1)`, `one<Int>()`). The message names the expected and the given number. |
| `E4021` | `UnsatisfiedBound` | A type argument, written or inferred, is not `Copy` (or `Clone`) where its type parameter is bounded by `Copy` (or `Clone`), including a type parameter of a generic body passed on with too weak a bound (`dup(x)` for an unbounded `x: U`). The message names the argument, the type parameter and the bound. |
| `E4022` | `BorrowInTypeArgument` | A type argument, written or inferred, contains a reference outside a function type: an explicit type argument (`id<&Int>(&a)`), an annotation (`Box<&Int>`, `Heap<&Int>`) or an inference through a nested position (`apply(s)` with `s: (&Int) -> Unit` for `(T) -> Unit`). Type parameters stand for owned values; help suggests taking `&T` in the generic signature. |
| `E4023` | `InfinitelySizedType` | A struct or enum contains itself by value, through its fields or payloads, nullables and other structs and enums with their type arguments substituted (`enum E { X(E), Y }`, `struct A { val b: B? }` with `struct B { val a: A }`, `struct A { val b: Box<A> }`), so its values would have no finite size. Reported at the name of each of the file's declarations on the cycle, with a note naming the path back to it; help suggests a `Heap<...>` field, since `Heap<T>`, references and function types hold their values elsewhere. |

A `public` or `package` function without a return type is reported as `E3001` when it has an expression body (Phase 1 rejects it first) and as `E4011` when it has a block body (Phase 1 accepts it as returning `Unit`). Unifying these is tracked in `ROADMAP.md`.

### E5xxx: Phase 2 borrow checking (`crates/prism-borrowck`)

| Code | Name | Meaning |
|---|---|---|
| `E5001` | `UseAfterMove` | A non-`Copy` value is used (read, copied, moved, borrowed, called, captured by a closure, or one of its fields assigned) on a path on which it was moved and not assigned again: after a move into a binding, an argument, a constructor or the receiver of a `consuming fun`, in one branch of an `if`, `when`, `&&`, `||` or `?.` ("possibly moved"), or in an earlier iteration of a loop. The diagnostic labels each move that reaches the use. |
| `E5002` | `UseOfUninitialized` | A local is read on a path on which it was never assigned: today only a `do`/`while` condition reading a body local whose declaration a `continue` skipped. The diagnostic labels the jump that skips the declaration. |
| `E5003` | `BorrowConflict` | A place is borrowed, or read through its owner, while a conflicting borrow of an overlapping place is live: a mutable borrow while any other borrow is live, a shared borrow or a read while a mutable borrow is live. Closure captures (by their inferred mode), argument, receiver (a safe call's from its `?.` test) and operator borrows count; a borrow lasts until the last use of every reference holding it, a call's result holds the borrows of the inputs its elision rule ties it to, and different fields are borrowed independently. The diagnostic labels where the borrow was made (for a closure, also the use in the closure that decided its mode) and where it is used later. |
| `E5004` | `AssignToBorrowed` | A place is assigned, or a call writes its result to it, while a borrow of an overlapping place is live; re-pointing a reference variable is not an assignment to its referent. The diagnostic labels the borrow and its later use. |
| `E5005` | `MoveOutWhileBorrowed` | A non-`Copy` value is moved while a borrow of an overlapping place is live. The diagnostic labels the borrow and its later use, with help suggesting `.clone()` for a `Clone` type. |
| `E5006` | `DroppedWhileBorrowed` | A local or a temporary is dropped, or its storage ends, while a borrow of it is live: a reference that outlives the block of what it points at, a borrow of a temporary that ends with its statement (`r = &make()`), a closure that stores a borrow of its own local in a variable it captures, or a body that stores a borrow of a by-value parameter through a `&mut` parameter or in a captured variable. The diagnostic points at the borrow and labels where the value ends (a parameter at its declaration) and where the borrow is used later (by the caller, at the `&mut` parameter it sees it through). |
| `E5007` | `EscapingClosure` | A function value that may be a closure with captures, or a function-typed parameter, escapes: it is returned from a function, closure or default value, initializes a top-level `val`, is stored in a struct field or enum payload, is written through a reference, is assigned to a captured variable, or is passed where the callee's declared parameter type has a type parameter outside a function type (`keep({ greet(s) })` for `fun <T> keep(x: T): T`). A closure without captures may escape. The diagnostic labels the capture or the parameter. |
| `E5008` | `DanglingBorrowReturn` | A body returns a borrow of memory it owns, which ends when it returns: a local, a temporary or a by-value parameter of a function, method, closure or default value, or a temporary of a top-level `val`'s initializer (`val R: &Int = &1`). Borrows of top-level `val`s may be returned under any elision rule. The diagnostic labels the borrow when it is elsewhere and, for a function without a borrowed input, notes elision rule 4. |
| `E5009` | `UntiedBorrowReturn` | A body returns a borrow derived from an input its elision rule does not tie the result to: a parameter other than the borrowed receiver of a method (rule 2), the result of calling a function-typed parameter whose type has no borrowed input, a capture of a closure whose type takes a borrowed input, or, in a default value, an earlier by-value parameter or one the call borrows mutably (`&mut`, a `mut fun` receiver). The diagnostic labels the parameter or capture and notes the rule. |

### E6xxx: Code generation and the entry point (`crates/prism-codegen`)

| Code | Name | Meaning |
|---|---|---|
| `E6001` | `UnsupportedByBackend` | `prism run` or `prism dump-clif` found a construct the Tier 1 backend does not compile yet, in a body it compiles (every body except generic and closure bodies, whether or not `main` reaches it): a struct, enum, nullable, `String`, `Heap<T>` or function value, a drop, a reference to one of these, a `Range` outside a local, a call of a generic function or a composable function. One error per body, at the declaration for an unsupported parameter or return type or a composable body, otherwise at the first statement or terminator using the construct; a note names the change that will support it. `prism check` never reports it. |
| `E6002` | `MissingMain` | The entry file of `prism run` declares no top-level function `main`. Help shows `fun main() {}`. |
| `E6003` | `InvalidMain` | The entry file's `main` breaks a rule of the entry point: it has type parameters, a receiver or parameters, returns a type other than `Unit`, is `compose`, or is a top-level `val`. The message names the rule. |

## Design Rationale
- **One code per error kind, separate from message text.** Tooling and tests match on codes, so messages can improve without breaking them.
- **`E4011` instead of reusing `E4004`** (`resolve-nominal-type-annotations`): a missing return type is not a call error, and the type checker also runs without the ABI stage (unit tests, a future LSP), so it needs its own diagnostic rather than relying on `E3001`.
- **`E1007` retired, not reused** (`fix-cli-workspace-audit`): mutually recursive imports are valid because exported declarations carry explicit types, so Phase 1 can extract every file in a cycle without reading any body.
- **Site-specific mismatch codes** (`typechecker-soundness`): `E4002`, `E4004` and `E4005` all report a value whose type is not assignable, distinguished by where the expected type came from (an annotation or operand, a parameter, a return type), so tools can suggest the right fix.
- **`E4020` to `E4022`, one per fix** (`generics`): a wrong number of type arguments (fix the count), an unsatisfied bound (bound the caller's type parameter, or pass a `Copy` value) and a borrow in a type argument (take `&T` in the signature) each have their own fix, so one code for every generic error was rejected. Members of a type parameter, receivers that do not match a generic extension, duplicate type parameters, uninferable type arguments, conditional markers and passing a capturing closure to a generic parameter widen existing codes instead.
- **`E4012`, `E4013` and `E4014` are new kinds, not reuses** (`typechecker-soundness`): an ambiguous import is not an unresolved symbol (`E4001`), a type that cannot be inferred is not a mismatch, and a member of a type whose module is not imported is not an unknown field (`E4006`), so each needs its own code and its own help line.
- **`E5xxx` for the borrow checker** (`borrowck-moves`, user for the range). A flow-sensitive error is the borrow checker's and takes the next `E5xxx` code; later borrow-checker errors (conflicting borrows, escaping closures, dangling returned borrows) continue the range.
- **Borrow errors are structured and show related locations** (`borrowck-moves`). A borrow error has a primary label, related labels (where the value moved), notes and help lines, so a use after move can say where the value went, which a single span and help line cannot. The CLI renders each location with its own snippet and label, related ones in source order.
- **`E5003` to `E5007`, one per fix** (`borrowck-exclusivity`). A conflicting borrow or read (`E5003`: end the other borrow earlier), an assignment (`E5004`) or a move (`E5005`: clone first) of a borrowed value, a value that ends while borrowed (`E5006`: make it outlive the reference, or bind a temporary with `val`) and an escaping capturing closure (`E5007`: pass the value as a parameter). Rust's split of borrow-borrow (E0499, E0502) and use-borrow (E0503) conflicts was rejected, since their fix is the same; whether reads while mutably borrowed deserve their own code is open. `borrowck-elision` continues the range at `E5008`.
- **`E5008` and `E5009`, one per fix** (`borrowck-elision`). A returned borrow of memory the body owns (`E5008`, the name `docs/architecture.md` gave it: return the value instead) differs from one derived from an input the elision rule does not tie the result to (`E5009`, the "own code" of rule 2, also for function-typed parameters, a closure's captures and defaults' earlier arguments: return a borrow of a tied input or change the signature). The wording follows rustc's E0515 with Prism's "borrow". Rejected: one code for both; a code per `E5009` reason; rustc's "lifetime may not live long enough", which names lifetimes Prism has no syntax for.


Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Three new codes, by fix.** `E4020` for the wrong number of type arguments (including none on a generic type in an annotation and any on a non-generic callee), `E4021` for an unsatisfied `Copy` or `Clone` bound, `E4022` for a borrow in a type argument; arity, bounds and borrows each have a different fix. The rest widens existing codes: `E2001` for the grammar, `E4006` for members of type parameters and mismatched receivers, `E4010` for type parameter names and one extension per receiver constructor, `E4013` for uninferable type arguments, `E4016` for conditional markers, `E4018` with a bound suggestion, and `E5007` for generic slots. Rejected: one code for every generic error.

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`E6xxx` for code generation and the entry point, one code per fix** (D13 and D25): `E6001` (wait for the backend change its note names, or avoid the construct), `E6002` (add a `main`) and `E6003` (fix its signature). The entry point is checked at `prism run` rather than by the type checker, because `prism check` checks libraries, which have no `main`. Runtime errors are not diagnostics and carry no code.
