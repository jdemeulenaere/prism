# Proposal: payload-patterns

## Why

A `when` today can only name a payload-free variant or compare its subject with a value, so the payload of `Shape.Circle(Double)` can't be read at all, and no code can take ownership of a struct's non-`Copy` field: moving `u.name` is `E4017` ("Moves Take Whole Values Only"), and a `consuming fun` cannot take `this` apart (`Account(name)` is `E4017`, from `extension-methods`). Real apps, starting with the todo list of the road to Milestone 2, model their state as enums with payloads (`Loading`, `Loaded(items)`, `Failed(message)`) and need to read those payloads and move them out. This change is item 21 of "Then: the road to Milestone 2" in `ROADMAP.md`. It also covers the deferred item "Payload-binding `when` patterns and destructuring declarations" and settles the "possible relaxation: partial moves out of owned locals".

## What Changes

- **Patterns in `when` branches.** A branch pattern may be a constructor pattern that mirrors construction: `Shape.Circle(val r)`, `Shape.Rect(val w, _)` or `Event.Click(Point(0, val y))` for enum variants, positional and with one sub-pattern per payload; `Point(val x, val y)` or `Point(y = val y)` for structs, positional or named like constructor arguments. A sub-pattern is a constructor pattern, a wildcard `_`, a binding `val x` or `var x`, an `in a..b` range, or any value expression compared with `==` (`Shape.Circle(1.0)`, `Shape.Named(limit)`). A bare identifier still compares with a value, as in Kotlin's `when`. A payload variant named without parentheses (`Shape.Circle`) matches that variant whatever its payloads, like Kotlin's `is` and Swift's `case .circle`. `_` and `val x` may also stand alone at the top of a branch.
- **Guards**, as in Kotlin 2.1: `Shape.Circle(val r) if r > 10.0 -> ...`. The guard sees the branch's bindings. When it is false, testing continues with the next pattern.
- **Alternatives that bind.** In `A(val x), B(val x) ->`, every pattern of a branch binds the same names with the same types (new `E4025`).
- **Binding modes follow the subject's type.** For an owned subject (`T`) a binding takes its part by value: a `Copy` part is copied and any other part is moved. For a `&T` subject, a `Copy` part is copied and any other is bound as `&T`. For a `&mut T` subject every part is bound as `&mut T`, so a payload can be written in place. So `when (this)` borrows in a `fun`, borrows mutably in a `mut fun`, and takes the receiver apart in a `consuming fun`. To borrow from an owned local, match on `&s`.
- **Taking a value apart moves it whole.** A branch that moves a part moves the whole subject, and the subject's other non-`Copy` parts are dropped where the branch ends. Only a whole owned local, a by-value parameter, a consuming `this` or a temporary may be taken apart. A field, a borrow's referent, a top-level `val`, a capture or a branch expression is `E4017`, with help suggesting `&`. Using the value later is `E5001`, labelled at the binding. Partial moves stay out of the language: destructuring covers what they would have been for, so the "possible relaxation" item is closed.
- **Exhaustiveness and usefulness over nested patterns** (Maranget's algorithm). It covers enums, structs, `Bool`, nullables (a constructor pattern of `T` matches a non-null `T?`), wildcards and bindings. Guards, ranges, non-literal values and literals of open types cover nothing. **BREAKING (message only):** `E4009` names a value that no branch covers ("`Shape.Rect(_, _)` is not covered"). Unreachable branches are not reported until a warnings channel exists.
- **Destructuring declarations by field name**: `val (name, email) = user`, `val (name as userName) = user`, `var (x, y) = point`. Each entry names a field of a struct, and the binding modes above apply (`val (name) = &user` borrows). A destructuring declaration takes only structs. Kotlin's positional `componentN` destructuring was rejected because it silently swaps fields when they are reordered. `consuming fun Account.toUser(): User { val (name) = this; return User(name) }` is the way to take a receiver apart.
- **New codes.** `E4024` (`InvalidPattern`) covers a pattern that does not fit its subject: the wrong number of sub-patterns, named sub-patterns on a variant, a field named twice, arguments on a payload-free variant, a destructured call of a function, or destructuring something other than a struct. `E4025` (`InconsistentAlternativeBindings`) covers alternatives that bind different names or types. Existing codes cover the rest: `E4001` (help: "to bind it, write `val x`"), `E4002`, `E4006`, `E4010`, `E4014`, `E4017`.
- **Typed tree, MIR, borrow checker and backend.** The typed tree gets recursive patterns, guards, binding modes and a destructuring statement. MIR gets a variant-field projection and a `Destructure` statement that moves a whole local into its parts. The borrow checker treats it as one whole-value move plus one initialization per part. The backend gets variant-field offsets from `codegen-aggregates`' enum layout and drops the unbound parts by `runtime-strings-and-drops`' glue.
- `prism fmt` prints patterns, guards and destructuring declarations.
- Not in scope: destructuring in `for` loops and closure parameters; `is`/`if`-pattern expressions and smart casts, including a binding of a nullable's non-null value; named enum payloads; rest patterns (`..`); `when (val x = e)`; compiling `when` into decision trees; warnings for unreachable branches. Each becomes a `ROADMAP.md` item on archive.

## Capabilities

### New Capabilities
- `patterns`: the pattern language of `when` branches and destructuring declarations, covering its grammar, how a pattern is checked against its subject, binding modes, guards, alternatives, exhaustiveness and usefulness, and destructuring declarations.

### Modified Capabilities
- `syntax`: "Expression-Oriented Control Flow": a branch parses into patterns (now including constructor patterns, wildcards and bindings), an optional guard and its body.
- `type-system`: "Control Flow Type Unification and Return Validation": payload variants are covered by constructor patterns, and `E4009` names an uncovered value.
- `ownership`: taking a value apart is a whole-value move (`E4017` for sources that are not owned, `E5001` after it), and partial moves are not added.
- `references`: bindings of borrowed subjects borrow the payloads, and the borrow checker sees those borrows.
- `typed-tree`: "Control Flow Stays Structured" covers the new pattern forms and guards, and destructuring declarations become statements.
- `mir`: "When Branches Test Patterns in Order" and "MIR Is Well Formed" change, and new requirements cover variant fields as places and `Destructure`.
- `execution`: payload bindings, guards, alternatives and destructuring at run time, including when unbound parts are dropped.
- `formatting`: patterns, guards and destructuring declarations are printed in the canonical layout.

## Impact

- `crates/prism-ast`: a recursive `Pattern` (wildcard, binding, constructor, value, range), `WhenBranch::guard`, and `StatementKind::Destructure`.
- `crates/prism-parser`: `parse_when_pattern` becomes a pattern parser, with speculation telling a constructor pattern from a value expression; guards; `val (...)` and `var (...)` declarations in `parse_local_var`.
- `crates/prism-typechecker`: a new `patterns.rs` (checking patterns against a subject, binding modes, the take-apart check) and `exhaustiveness.rs` (usefulness and witnesses), called from `infer_when_expr` and `check_statement`; `E4024`, `E4025` and the `E4009` witness in `error.rs`.
- `crates/prism-typed-tree`: recursive `Pattern`, `BindingMode`, `LocalKind::PatternBinding`, `WhenBranch::guard`, `StatementKind::Destructure`, and their dump and validation.
- `crates/prism-mir`: `Projection::VariantField`, `StatementKind::Destructure`, the pattern lowering in `build/control_flow.rs` and a new `build/pattern.rs`, plus types, dump and validation.
- `crates/prism-borrowck`: `Destructure` effects in `uses.rs`, `move_paths.rs`, `loan_flow.rs` and `elaborate.rs`; `VariantField` in `places.rs`; the take-apart help on `E5001`.
- `crates/prism-codegen`: `VariantField` locations and `Destructure` copies in `lower.rs` and `supported.rs`.
- `crates/prism-formatter` and `crates/prism-abi` (the span-free hash of default values) walk the new syntax.
- Prerequisites: `codegen-aggregates` (enum layouts) and `runtime-strings-and-drops` (drop glue, allocation checks) must have landed, as the roadmap order already requires.
- Specs: the deltas above and a new `patterns` spec. The diagnostics catalog gains `E4024` and `E4025` and widens `E4009` and `E4017` (catalog rows, edited directly). On archive, `AGENTS.md` lists the new `patterns` spec.
