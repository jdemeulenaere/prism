# Proposal: statements-and-loops

## Why

Prism has no loops, so iteration can only be written as recursion. The borrow checker planned after this change (see `ROADMAP.md`, "the road to the borrow checker") needs loops to exist first: "moved in a previous iteration" is a core use-after-move case, and the control-flow graph it runs on should have back edges from day one rather than an untested fixpoint. Loops are statements, and the AST has no statement type: `LocalVar` and `Assign` are expression kinds that the parser only allows in statement position. This change adds the `Statement` type, so later stages (the typed tree, MIR lowering) never have to tell statements and expressions apart by convention.

## What Changes

- **`Statement` AST type.** A block becomes `statements` plus an explicit optional `tail` expression, which gives the block its value. Local declarations, assignments, expression statements and the new loops are statements. `return` stays an expression.
- **Loops**, all statements of type `Unit`:
  - `while (condition) body`;
  - `do body while (condition)`, whose condition can see the body's locals;
  - `for (name in range) body` over the built-in `Range` only. The loop variable is a fresh `val` each iteration, and the range is evaluated once.
- **`break` and `continue`**: expressions of type `Nothing` (so `?: break` works) that carry no value. Outside a loop, or inside a closure nested in a loop, they are rejected with a new code, `E4015`.
- **Half-open range operator `..<`** at the same precedence as `..`. It produces a `Range` that excludes its end, in expressions and in `when` patterns (`in 0..<n`).
- **BREAKING:** `while`, `do`, `for`, `break` and `continue` become reserved keywords and can no longer be used as identifiers.
- Not in scope: loop labels (`break@outer`), iteration over anything but `Range`, loops as expressions, and `while (true)` typed as diverging.

## Capabilities

### New Capabilities
- `loops`: loop statements (`while`, `do`/`while`, `for`/`in`) and the `break`/`continue` jumps: their grammar, scoping and typing, and how jumps relate to loops and closures.

### Modified Capabilities
- `syntax`: blocks have statements plus an optional tail, and assignment is a statement (still allowed as an unbraced branch body); a new `..<` operator; new reserved keywords.
- `type-system`: a block's value comes from its tail; a range built with `..<` excludes its end, including in `when` patterns.

## Impact

- `crates/prism-lexer`: new keyword tokens and the `..<` token. The lexer must still read `4..10` as a range and `4..<10` as a half-open range.
- `crates/prism-ast`: new `Statement` and `Block` types replacing `BlockExpr.stmts: Vec<Expr>`. `ExprKind::LocalVar` and `ExprKind::Assign` move to `Statement`. New `ExprKind::Break` and `ExprKind::Continue`, and an end-exclusive form for ranges and `when`'s `in` patterns.
- `crates/prism-parser`: statement parsing, loop parsing, `break`/`continue` (bare, like `return` before a token that cannot start an expression), and `..<` in the precedence table.
- `crates/prism-typechecker`: block checking over statements plus tail, loop typing, a loop-depth stack that resets at closure boundaries for `E4015`, and `Range` for `..<`.
- `crates/prism-abi`: exported signatures and ABI hashes are unaffected, since loops live in bodies. Only the span-free body hash of generic functions walks the new statement kinds.
- `crates/prism-cli`: renders `E4015`. New `.prism` fixtures under `tests/fixtures/loops/`.
- `openspec/specs/diagnostics/spec.md`: `E4015` (`BreakOutsideLoop`) is added to the catalog, which is not a requirement and is edited directly, as past changes did.
- `docs/architecture.md` §4: the runtime meaning of loops, `break`, `continue` and `..<` is recorded as planned semantics awaiting code generation.
- Existing tests that build `BlockExpr` by hand need updating to the new AST.
