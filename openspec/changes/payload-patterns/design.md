# Design: payload-patterns

## Context

See `proposal.md` for motivation. This is item 21 of "Then: the road to Milestone 2" in `ROADMAP.md`, added by the orchestrator on 2026-10-18 (process row P6 of `docs/decisions-for-review.md`). It also covers the deferred item "Payload-binding `when` patterns and destructuring declarations" and the "possible relaxation: partial moves out of owned locals". The user was not available: every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. It builds on `codegen-aggregates` (item 15: enum layouts, `when` over enums in the backend) and `runtime-strings-and-drops` (item 16: drop glue, `--check-allocations`), both archived before this change was implemented.

**Reconciliation with the main specs (at implementation).** The change was proposed before `codegen-aggregates` and `runtime-strings-and-drops` were archived. Before implementing, every MODIFIED requirement of the delta specs was compared with the current main spec text: each differs from it only by this change's own edits, so no delta had drifted, and `openspec validate payload-patterns --strict` passed. One requirement the deltas missed was added: `mir`'s "Statements Assign Rvalues to Places" lists the projections a place may have, so it is MODIFIED to add variant payloads (D15). The diagnostics catalog still ends at `E4023`, so `E4024` and `E4025` stay the next free codes (D18).

Current state:

- **Syntax.** `prism-ast` has `WhenPattern::{Expr, InRange, Else}` and `WhenBranch { patterns, body }` (`crates/prism-ast/src/expr.rs`); `parse_when_pattern` in `crates/prism-parser/src/parser.rs` parses `in` ranges or an arbitrary expression. Enum payloads are positional types (`EnumVariant::fields: Vec<Type>`); struct fields are named and constructors take positional or named arguments. `_` lexes as an identifier. The parser has a `speculate` helper (used for call type arguments). `StatementKind::Local` declares one name.
- **Checking.** `infer_when_expr` in `crates/prism-typechecker/src/checker.rs` peels references off the subject (`peel_reference_nodes`), then classifies each pattern: `unit_variant_pattern` recognizes a payload-free variant (qualified, or bare when not shadowed); any other expression is a `Pattern::Value` checked against the subject's type and compared with `==`; `true`/`false`/`null` literals are tracked for `Bool`/`Bool?` coverage. Exhaustiveness is "every variant named, or `else`" (`E4009`). `Shape.Circle(1)` is a value, covering nothing.
- **Moves.** "Moves Take Whole Values Only": a non-`Copy` value is consumed only from a whole owned local or a temporary (`check_move_source`, `E4017`). A `consuming fun` therefore cannot take `this` apart, and nothing can take ownership of one field of a struct.
- **Typed tree.** `Pattern::{Variant, Value, Range, Condition, Else}` (`crates/prism-typed-tree/src/tree.rs`); the subject is read in place; `LocalKind` has no pattern binding.
- **MIR.** Places project `Deref`, `Field`, `Payload` (nullable) and `Index`; there is no projection into an enum's payload. `lower_when` (`crates/prism-mir/src/build/control_flow.rs`) switches on the discriminant when every pattern is a payload-free variant or `else` (`lower_variant_switch`), and otherwise tests patterns in order (`lower_ordered_tests`), each value pattern through `==`.
- **Borrow checker.** Move paths are a local plus `Field` projections; a nullable's payload folds into the nullable; drop elaboration works on whole locals with drop flags (`crates/prism-borrowck/src/move_paths.rs`, `elaborate.rs`). There are no partial moves and no partial drops.
- **Backend.** `codegen-aggregates` D6 lays an enum out as the smallest tag, then each variant's payloads laid out like a struct's fields after it, all variants sharing that storage; `runtime-strings-and-drops` D9 drops a value by its type's glue.

## Goals / Non-Goals

**Goals:**
- Read and move enum payloads and struct fields with patterns that read like Kotlin and are unambiguous, so state machines (`Loading`, `Loaded(items)`, `Failed(message)`) can be written as real apps need.
- Make taking a value apart a whole-value move, so no partial-move state, partial drop or field-level drop flag enters the borrow checker or the backend.
- Exhaustiveness and usefulness over nested patterns with an example of what is missing, as Rust and Swift report.
- Every new scenario has a `.prism` fixture; runtime scenarios run with `--check-allocations`.

**Non-Goals:**
- Destructuring in `for` loops (only `Range` is iterable) and in closure parameters (`{ (a, b) -> }`); `is`- or `if`-pattern expressions (`if (s is Shape.Circle(val r))`) and smart casts, including a pattern binding a nullable's non-null value; named enum payloads; rest patterns (`Rect(..)`); `when (val x = e)`; decision-tree compilation; warnings for unreachable branches. Each is a `ROADMAP.md` item on archive (D20).
- Patterns through `Heap<T>` (deref patterns), which wait for `heap-boxes`: a `Heap<T>` part is matched by `_`, a binding or a value only.
- Changing `when` statements: they still need no coverage.

## Decisions

### D1. Patterns mirror construction, and `val` marks a binding (autonomous)

A pattern is written the way its value is built: `Shape.Circle(val r)`, `Shape.Rect(val w, _)`, `Point(0, val y)`, `Point(y = val y)`. A sub-pattern is a constructor pattern, `_`, `val name` / `var name`, an `in` range, or any value expression compared with `==` (`Shape.Circle(1.0)`, `Shape.Named(limit)`).

A binding needs `val` (or `var` for a mutable local), as Java's record patterns need `var r`, C#'s positional patterns `var r` and Swift's `case .circle(let r)`. A bare identifier keeps today's meaning, a value compared with the subject, which is how Kotlin's `when (x) { y -> }` reads. So `Shape.Circle(r)` compares the payload with a local `r`, and when no `r` is in scope it is `E4001` with help suggesting `val r`. The same written pattern never changes meaning because a name came into scope.

Rejected: Rust's rule (a bare identifier binds, unless it names a constant or a unit variant), which silently turns `Box.Of(limit)` into a new binding when `limit` is renamed, and would invert the meaning of today's `when (x) { y -> }`. Kotlin's `is Shape.Circle ->` with smart casts: Prism's payloads are positional and unnamed, so a smart-cast `s` has nothing to read the payload by, and `is` would suggest that variants are types. Swift's distributive `case let .rect(w, h)`: one more form, for a few saved keywords.

### D2. Telling a constructor pattern from a value (autonomous)

In pattern position the parser tries, by speculation, a dotted name followed by `(` on the same line as a constructor pattern: arguments are patterns, optionally named (`y = pattern`). The attempt is kept when the closing `)` is followed by `,`, `->`, `if` or, nested, by `,` or `)`. Otherwise (`g(1) + 1`, `f(x).y`, `f(1) { }`), the stream is restored and the pattern is parsed as an expression. The type checker resolves the name: a variant or a struct makes a constructor pattern; anything else (a function) is the value expression the pattern spells, rebuilt from its arguments when all are values (`when (x) { g(3) -> }` keeps working), and `E4024` when an argument is `_`, a binding or a range. A constructor pattern takes no type arguments: they come from the subject.

Without a subject a branch is a condition, so the parser parses expressions only, and `_`, `val`, `var` and guards there are `E2001` with a message saying so.

### D3. `_` and a payload variant named alone (autonomous)

`_` is the wildcard in pattern position (it lexes as an identifier, so `when (x) { _ -> }`, which compared `x` with a local `_`, now matches anything; no program in the repository does that). Elsewhere `_` stays an identifier, until a change reserves it. At the top of a branch `_` is accepted beside `else`, and a top-level `val n` binds the whole subject.

A payload variant written without parentheses (`Shape.Circle`, bare `Circle`) matches that variant whatever its payloads and binds nothing, as Swift's `case .circle:` and Kotlin's `is Circle`. Today such a pattern is the variant's constructor function compared with an enum, a type error, so no program changes meaning. Rejected: requiring `Shape.Circle(_)` (verbose for many payloads) and Rust's `Rect(..)` rest pattern now (deferred).

### D4. Struct patterns are positional or named, like constructor calls (autonomous)

A struct pattern takes positional sub-patterns for every field in declaration order, or named ones (`Point(y = 0)`), positional ones first as in a call. A named form may leave fields out, which are wildcards. A positional one must give them all (`E4024` otherwise), since a short positional list would be ambiguous about what it skipped. Variant patterns are positional only: payloads have no names (`E4024` for a named sub-pattern). Rejected: Rust's `Point { x, .. }` braces, which would collide with a branch body block; positional struct patterns with optional trailing fields.

### D5. Binding modes follow the subject's type (autonomous)

The mode of every binding of a pattern is decided by the type of the subject it matches, after the type checker peels references off it, as `infer_when_expr` already does:

| Subject | `Copy` part | Other part |
|---|---|---|
| `E` (owned) | copied | moved: takes the subject apart (D6) |
| `&E` | copied (copy-out) | `&T` |
| `&mut E` | `&mut T` | `&mut T` |

Nested patterns inherit their subject's mode, and a pattern with no by-value non-`Copy` binding never moves anything. A `var` binding is a mutable local of the binding's type: a copied or moved `var` can be changed freely, and a `var` borrow can be re-pointed as any reference `var` can.

This is Rust's rule ("default binding modes": by value for owned places, by reference through `&`), minus partial moves, plus Prism's copy-out of `Copy` values from shared borrows, the same ergonomics as `reference-access` D3. Under `&mut` a `Copy` part is borrowed too, so `when (state) { State.Loading(val p) -> *p += 1 }` updates a payload in place, as UI state machines need. It fits the receiver modes exactly: `when (this)` borrows in a `fun`, borrows mutably in a `mut fun` and takes `this` apart in a `consuming fun`. To borrow from an owned local, the subject is written `&m`, as any borrow outside an argument is written (`val r = &u`).

Rejected: Swift's rule for noncopyable values (SE-0432): a place subject is borrowed and only a temporary or `consume x` is consumed. Prism has no move operator: moves happen by position (initializers, by-value arguments, returns), and adding `when (consume m)` for one construct would be the only explicit move in the language, while `consuming fun` could still not take `this` apart without it. Rejected also: inferring the mode from later uses of the subject (a move when it is not used again), which makes a body's meaning depend on code after the `when`. The cost of the chosen rule is that `when (m) { Msg.Text(val s) -> println(s) }` moves `m`. A later use of `m` is `E5001`, labelled at `val s` with help suggesting `when (&m)` (D16).

Under `&mut`, a `when` whose branches give a `&mut Int` binding and an `Int` has no join (`reference-access` rejected copy-out in joins), so `val x = when (m) { A(val n) -> n; B -> 0 }` over a `&mut` subject needs an annotation (`val x: Int = ...`), which copies out at each branch. That is rare: matching mutably is for writing.

### D6. Taking a value apart moves it whole (autonomous)

A branch whose pattern binds a non-`Copy` part by value moves its whole subject on that branch's path, and so does a destructuring declaration. The source must be one the move rules already accept as a whole value: a whole owned local, a by-value parameter, `this` of a `consuming fun`, or a call, constructor or literal temporary. A field, a borrow's referent, a top-level `val`, a binding a closure or default value borrows, or a branch, block or `?:` expression is `E4017` ("cannot take apart field 'last' of 'i'"). Its help suggests `when (&i.last)`, or binding a branch result to a `val` first. The check runs once the patterns' modes are known, on the subject expression, reusing `check_move_source`'s classification, so `when (u.name)` over a `String` field with only value patterns stays a read in place.

A branch expression subject (`when (if (c) a else b)`) is rejected rather than consumed because the subject is checked before its patterns: whether its leaves must be moved into the temporary is only known afterwards, and re-checking it would duplicate diagnostics. Binding it first costs one line.

In a guard (D9), a moving binding is not moved yet, so moving it there is `E4017` ("cannot move binding 's' in a guard").

### D7. Partial moves stay out of the language (autonomous)

The deferred "possible relaxation: partial moves out of owned locals" is decided against. Every use it had is covered by taking the value apart: `val (name, email) = user` instead of moving `user.name` and then reading `user.email`, `val (name) = this` in a `consuming fun`, a `when` that moves a payload. Partial moves would make drop elaboration track fields (a field-level drop flag per moved field, partial drops of the remainder), and make every later use of the local ask which fields are left, for a convenience destructuring already gives. The `E4017` help for a field of a whole owned local now suggests the destructuring (`val (name) = u`). The roadmap item is closed on archive, with this rationale in `ownership`'s Design Rationale. Revisit only if real apps show patterns that destructuring cannot express.

### D8. Parts no binding takes are dropped where the branch ends (autonomous)

When a value is taken apart, each non-`Copy` part no binding takes (`_`, a value sub-pattern, an unnamed field of a destructuring) is moved with the others into an anonymous local of the branch, or of the enclosing block for a declaration, and dropped where that scope ends with the bindings. So no value is ever partly dropped, and drop elaboration keeps working on whole locals. Timing is unobservable today: there are no user destructors, and `--check-allocations` only checks that each part is freed once. Rejected: dropping the unbound parts at once, which would need a drop in the middle of the binding sequence and, once destructors exist, would run them before the branch body, where Rust runs them after it.

### D9. Guards, as in Kotlin 2.1 (autonomous)

`pattern if condition ->` adds a guard to a branch of a `when` with a subject, without parentheses (Kotlin 2.1's guard syntax; the mandatory parentheses of `if (c)` exist to keep `{` from being read as a trailing lambda, and a guard ends at `->`). The guard is checked as a `Bool` condition (`E4002`), sees the branch's bindings, and applies to every pattern of the branch: a branch matches when one of its patterns matches and the guard is true for that pattern's bindings, otherwise testing continues with the next pattern. `else if` and guards in a `when` without a subject are `E2001` (use `&&`).

Inside a guard the branch's bindings are read-only (`E4003` for an assignment through one, `&mut`, or a `mut fun` receiver), and a moving binding cannot be moved (`E4017`), because the guard may be false and the next branch must still find the subject whole. In MIR, copying and borrowing bindings are made before the guard and moving ones after it, while the guard reads a moving binding's part in place (D15), which is Rust's "bind by reference in the guard, by move in the arm" without a second set of locals.

Rejected: Rust's guards with mutable access through bindings, which need the "fake borrows" of the subject during guards; guards only on single-pattern branches (Kotlin's restriction), which buys nothing once alternatives must bind alike (D10).

### D10. Alternatives bind the same names (autonomous)

A branch's comma-separated patterns may bind, but each must bind the same set of names, each with the same type and mode, which then name one binding of the branch (`Shape.Circle(val d), Shape.Square(val d) -> d`). Any difference is `InconsistentAlternativeBindings` (`E4025`), naming the first binding that differs. This is the rule of Rust's and Swift's or-patterns. Rejected: forbidding bindings in multi-pattern branches (the cheap rule), which forces duplicated branch bodies for the common "two variants carry the same payload" case.

### D11. Exhaustiveness and usefulness by Maranget's algorithm (autonomous)

A new `crates/prism-typechecker/src/exhaustiveness.rs` implements the usefulness algorithm of Maranget's "Warnings for pattern matching" (2007), as Rust's `rustc_pattern_analysis` and Swift's space engine do. It works over a reduced pattern form, independent of the AST:

- constructors: an enum's variants (arity = payload count, payload types substituted for the instance), a struct's single constructor (arity = field count), `true` and `false` for `Bool`, and `null` and "present" (arity 1) for `T?`;
- a literal of an open type (`Int`, `Double`, `Char`, `String`) is a constructor of a type whose constructor set is never complete;
- a wildcard or a binding is a wildcard;
- a guarded branch, a range, and a value that is neither a literal, `null` nor a payload-free variant cover nothing; they take part in usefulness only as rows that never make anything exhaustive.

For a `T?` subject a constructor pattern or a non-`null` literal of `T` is "present" with that pattern inside, so `null` plus patterns covering `T` cover `T?`; today's `Bool`/`Bool?` rule falls out of the algorithm. A `when` in value position is exhaustive when the wildcard row is not useful after all branches; otherwise the algorithm's witness is printed as a pattern (`Shape.Rect(_, _)`, `Outer.Wrap(Inner.B)`, `null`, `false`, `_` for an open type) in the `E4009` message, with help suggesting a branch for it or an `else` branch. An enum without variants keeps needing `else` in value position, as today, so a `when` with no branches is never accepted as exhaustive (the algorithm's empty constructor set is special-cased). Patterns are finite and do not look through `Heap<T>`, so the algorithm terminates. The typed tree's `exhaustive` flag is the algorithm's answer, also for `when` statements.

### D12. Unreachable branches are not reported yet (autonomous)

The algorithm also knows when a branch is useless (`Box.Of(_)` after `Box.Of(val n)`, an `else` after full coverage). Prism has no warnings channel (`ROADMAP.md`, from `typechecker-soundness`), and an error would reject today's valid `when (b) { true -> 1; false -> 2; else -> 3 }`. So usefulness is computed and unit-tested but not reported. A `ROADMAP.md` item ties the warning to the warnings channel. Rejected: an error now.

### D13. Destructuring declarations name fields (autonomous)

`val (name, email) = user` binds fields by name: each entry is a field of the value's struct, bound to a local of the same name, or to another name with `field as local` (`val (name as userName) = user`). Order is free and fields may be left out. `var (...)` makes every local mutable. Only structs, or references to them, can be destructured (`E4024`, help: use `when` for an enum). The binding modes are D5's over the value: `val (name) = this` takes a consuming receiver apart, `val (name) = &user` borrows.

Kotlin's `val (a, b) = pair` is positional (`component1()`, `component2()` in primary-constructor order), so reordering two fields of one type silently swaps the bindings. Kotlin is itself moving toward name-based destructuring, and Rust's struct patterns and JavaScript's object destructuring bind by name. A positional entry that is not a field name is `E4006` with help suggesting `first as a`, which catches code written the Kotlin way. `as` is directional ("field as local"), as in Kotlin's import aliases, and is a keyword only inside the list, so it stays free for a future cast operator. Rejected: positional destructuring; Kotlin's proposed `(val local = field)` entry form, which reads as an assignment in the opposite direction from named constructor arguments and named struct patterns (`Point(y = val y)`); destructuring single-variant enums (rare, and `when` with one branch does it); nested patterns and type annotations in entries (destructure again instead).

### D14. The typed tree

`prism_typed_tree::Pattern` becomes recursive:

- `Variant { enum_id, index, name, arguments, fields: Vec<Pattern> }` (one per payload; empty for a payload-free variant; all `Wildcard` for a variant named alone),
- `Struct { id, arguments, fields: Vec<Pattern> }` (one per field, in declaration order, named and omitted ones normalized into place),
- `NonNull(Box<Pattern>)`, a nullable that is not `null` whose payload matches the inner pattern: a constructor pattern of `T` against a `T?` (added at implementation, so that MIR tests the tag before the payload's sub-patterns and the exhaustiveness check sees "present" explicitly; a literal of `T` against a `T?` stays a value compared with `==`),
- `Wildcard`, `Binding { local, mode: BindingMode }`, `Value(ExpressionId)`, `Range { .. }`, and at the top only `Condition(ExpressionId)` and `Else`.

`BindingMode` is `Copy`, `Move`, `SharedBorrow` or `MutableBorrow`. Each binding is a `Local` of the new `LocalKind::PatternBinding`, whose type is the part's type for `Copy` and `Move` and `&T`/`&mut T` for the borrows. Alternatives of a branch share one local per name. `WhenBranch` gains `guard: Option<ExpressionId>`. A destructuring declaration is `StatementKind::Destructure { pattern, value }`, its value read in place and its pattern a `Pattern::Struct`. The subject of a `when` and the value of a destructuring stay read in place; a `Move` binding is what tells MIR to take it apart. The dump prints each sub-pattern indented under its pattern (`pattern variant Shape.Circle`, `pattern binding r (copy): Double`, `pattern wildcard`, `guard`). `TypedFile::validate` checks that sub-pattern counts match the definitions, that binding types fit their modes, and that a `Move` binding's subject is a whole local or a temporary.

The checker's pattern code moves into a new `crates/prism-typechecker/src/patterns.rs` (an `impl TypeChecker` block like the rest): `check_pattern(pattern, subject_type, mode, bindings)` returns a typed-tree pattern and records bindings in a branch-local map, which `infer_when_expr` defines in a new scope for the guard and body and checks for alternative consistency (D10). `unit_variant_pattern` becomes part of resolving a constructor or value pattern.

### D15. MIR: payload places and one destructuring statement

- **`Projection::VariantField { variant, field }`**, a place of payload `field` of variant `variant`, typed by `place_type` from the type table's `instantiated_variants`. It is printed `(_1 as Text).0`. One projection rather than Rust's `Downcast` followed by `Field` keeps every intermediate place typed by a Prism type.
- **`StatementKind::Destructure { source: LocalId, variant: Option<usize>, parts: Vec<(usize, LocalId)> }`** moves the whole local `source` (an enum holding `variant`, or a struct for `None`) into one local per listed part. The validator requires every non-`Copy` part of the variant or struct to be listed exactly once and each target to be a whole local of the part's type, so nothing can leak. It is printed `destructure move _1 as Pair => (0: _4, 1: _5)`. A nullable payload or a nested part is first moved into a temporary (`_t = move (_1 as payload)`, which already counts as moving `_1`), then destructured. A statement that writes several locals is new to MIR, but it is the honest shape of the operation: one whole-value move and several initializations, which is what the borrow checker must see (D16). Rejected: one `Move` operand per part out of `(_1 as V).i`, which would need move paths and drop elaboration to understand partial moves (D7).
- **Lowering.** A new `crates/prism-mir/src/build/pattern.rs` holds `test_pattern(place, pattern, on_failure)`, which tests a constructor's tag (`Discriminant` and `SwitchInt`) and then its sub-patterns left to right, recursing into `VariantField`, `Field` and `Payload` places, and `bind_pattern(place, pattern, phase)`, which makes copies and borrows (`Rvalue::Use(Copy)`, `Rvalue::Borrow` with `BorrowOrigin::Pattern`) or the destructuring. `lower_ordered_tests` calls them per pattern. A branch's patterns each get their own test path, binding block and guard copy, all jumping to one body block. While a guard is lowered, a map from each `Move` binding's local to its part's place makes uses of the binding read that place. `lower_variant_switch` is kept for `when`s whose top-level patterns are variants with only wildcard and binding sub-patterns and no guards, binding in each target block. Binding locals and the temporaries of unbound parts are declared in the branch's scope (the block scope for declarations), so ordinary scope drops drop them (D8). The subject temporary of a non-place subject lives in the `when`'s temporary scope, and drop elaboration removes or guards its drop where branches moved it.
- **Destructuring statements** lower as an irrefutable struct pattern over their value's place (or a temporary of the enclosing block's statement for a non-place), without tests.

Decision trees (Maranget's compilation to a minimal switch tree) are an optimization for later. Ordered tests re-read a discriminant per pattern, which Cranelift's local optimizations partly undo.

### D16. The borrow checker

- `uses.rs` (`statement_effects`): a `Destructure` is a `Move` of its source's root path, then an `Init` of each part's root path. `move_paths.rs` folds `VariantField` into its enum, like `Payload`, since nothing moves out of a payload place except through a whole-value `Destructure`. Reads and borrows of payloads are uses of the enum's path.
- `elaborate.rs`: a `Destructure` clears its source's drop flag before the statement and sets each part's flag after it, like an assignment's initialization.
- `places.rs` (`places_overlap`): two `VariantField`s of the same variant and different fields are disjoint, like fields; of different variants they overlap, because the variants share storage; a `VariantField` is a prefix-extension like `Field` for `is_prefix`.
- `loan_flow.rs`: a `Destructure` moves its source's loans into the parts, fitted to each part's type. Payloads cannot hold references, so this only matters for function values, which `E5007` keeps from capturing. `conflicts.rs` checks the source as a move (`E5005` while borrowed).
- A new `BorrowOrigin::Pattern` labels a binding's borrow, so `E5003`/`E5004` can say "borrowed by the binding `s`".
- `E5001` for a use after a `Destructure` labels the binding of the first moved part (its span), and when the source is the local the `when` matched, adds the help "match on `&m` to borrow its parts". MIR records the `Destructure` statement's span as that binding's span and its source as the subject local.

### D17. The backend

`codegen-aggregates`' D6 already places each variant's payloads after the tag like a struct's fields. `lower.rs`'s `location` adds the offset of field `field` in variant `variant`'s payload layout for `VariantField`, and `Destructure` copies each listed part's bytes from the source's payload offsets into its target's storage (a move is a byte copy, `codegen-aggregates` D12). Unlisted `Copy` parts need nothing. `supported.rs` accepts both. Drops of the parts are ordinary `Drop`s of whole locals, run by `runtime-strings-and-drops`' glue. Nothing changes for hot reload: patterns are body code, and a layout change already recompiles every unit that uses the type.

### D18. Diagnostics (autonomous)

The next free codes after `codegen-aggregates`' `E4023`:

- `E4024` `InvalidPattern`: a pattern that does not fit its subject's definition. That is the wrong number of payload or positional field sub-patterns, a named sub-pattern on a variant, a field named twice, a positional sub-pattern after a named one, arguments on a payload-free variant, a constructor pattern whose name is a function (D2), or a destructuring of something other than a struct. These are one kind of error, fixed by making the pattern fit the definition.
- `E4025` `InconsistentAlternativeBindings` (D10), fixed differently: make the alternatives agree.

Reused: `E4002` (a constructor pattern of another type than the subject, as value patterns already report; a non-`Bool` guard), `E4001` (with help `val name`), `E4006` (an unknown field), `E4010` (a name bound twice), `E4014` (a member of an unimported type), `E4017` (D6, D9), `E4003` (D9), `E4009` (with a witness, D11), `E5001` (D16). Syntax errors are `E2001`. The diagnostics catalog rows for `E4009` and `E4017` are widened.

### D19. A `patterns` capability spec

Patterns get their own spec, as loops did (`loops`): grammar, checking against a subject, binding modes, guards, alternatives, exhaustiveness and destructuring. The pattern rules then live in one place, while the `when` requirements of `syntax`, `type-system`, `typed-tree`, `mir` and `execution` point to it. Move rules stay in `ownership` and borrow rules in `references`, whose new requirements cover taking apart and borrowing bindings.

### D20. Deferred, as `ROADMAP.md` items on archive

Destructuring in `for` loops (with `collections`' iteration) and closure parameters; `is`/`if`-pattern expressions and smart casts, with a binding of a nullable's non-null value; named enum payloads (`Rect(val w: Double, val h: Double)`), which would allow named variant patterns; rest patterns (`Rect(..)`); `when (val x = e)`; decision-tree lowering (D15); warnings for unreachable branches (D12); reserving `_` outside patterns (D3).

### D21. Existing tests

The `type-system` fixtures of "Only a variant name covers that variant" keep their names. Their expectations change only where a payload variant named alone or a constructor pattern now covers a variant. The `E4009` fixtures gain the witness in their expected messages. Typed-tree and MIR snapshots of `when` change shape (sub-patterns are printed, variant patterns list their empty sub-patterns), and are regenerated and read. Parser tests of `WhenPattern::Expr` move to the new pattern type.

### D22. Variant names match through a nullable subject (autonomous, at implementation)

A constructor pattern of `T` matches a non-null `T?` (the `patterns` delta, "Nullable Subjects Match Patterns of Their Payload"), and so does a variant written without parentheses: over `c: Color?`, `Red` is the typed-tree pattern `NonNull(Variant Red)`, so `null`, `Red` and `Green` cover `Color?` without `else`. Before this change `Red` against a `Color?` was a value compared with `==`, which covered nothing; the run-time meaning is the same. Rejected: keeping bare names as values there, which would make `Color.Red(..)`-style constructor patterns and payload-free variant names cover nullables differently.

### D23. Pattern errors do not also report `E4009` (autonomous, at implementation)

A `when` whose patterns reported an error is not also reported as non-exhaustive: the pattern that failed to check covers nothing reliably, so the witness would be noise (`Box.Of(n) -> n` with `n` unresolved reports `E4001` only). Rejected: reporting both, which the fixture convention (an exact set of codes) would pin.

### D24. Guard mutation and the take-apart help (autonomous, at implementation)

In a guard, passing a mutable-borrow binding to a `&mut` parameter (a reborrow) mutates through it, so it is `E4003` like an assignment, `&mut` or `mut fun` receiver. The `E5001` help after a take-apart ("match on its borrow instead, as in `when (&m)` or `val (...) = &m`") is given whenever the move was a `Destructure` of the used local, for a `when` subject and a destructured value alike, which needs no record of where the `Destructure` came from. Rejected: a flag on `Destructure` to give the help for `when` subjects only.

## Risks / Trade-offs

- [`when (m)` with a by-value binding moves `m`, surprising Kotlin users who only wanted to read] → it is a compile error at the later use, labelled at the binding with help suggesting `when (&m)` (D16), and the common places to match (methods with `fun` receivers, `&T` parameters) borrow. If it proves noisy, a borrow-by-default for place subjects can be revisited together with a move operator.
- [Speculative parsing of constructor patterns could misread an unusual value pattern] → only a dotted name with arguments followed by a pattern terminator is tried, and a function callee falls back to the value expression (D2); parser tests cover calls followed by operators, members and trailing lambdas.
- [Taking a branch-expression subject apart is `E4017`, which is stricter than necessary] → a `val` first fixes it; relaxing it later only accepts more programs.
- [Destructuring by name differs from Kotlin's positional `componentN`] → Kotlin-style code gets `E4006` with an `as` suggestion, never a silent swap.
- [Ordered tests re-read discriminants] → the MIR is simple to check, and a decision-tree lowering can replace it without changing semantics.
- [A `Destructure` writes several locals, unlike every other statement] → `uses.rs` is the single source of effects for every analysis, and the validator checks its shape; tests cover drop flags for conditional take-apart.

## Migration Plan

No external users. Programs using `_` as a value pattern change meaning (none in the repository). The `E4009` message changes. The diagnostics catalog gains `E4024` and `E4025` and widens `E4009` and `E4017`. On archive: tick item 21 and the deferred "payload-binding patterns" item, close the "partial moves" relaxation with D7's rationale, add D20's items to `ROADMAP.md`, promote D1, D5, D6, D7, D11 and D13 into the Design Rationale of `patterns` and `ownership`, and list `patterns` in `AGENTS.md`'s spec list.

## Open Questions

- Whether the typed-tree dump should print alternatives' shared bindings once per branch or under each pattern; settle when writing the dump.
- Whether `E4009`'s witness should list several missing values (Rust lists up to three); one is enough to start.
