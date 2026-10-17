# Design: statements-and-loops

## Context

See `proposal.md` for motivation. This change is step 1 of the "road to the borrow checker" in `ROADMAP.md`. The user agreed that sequence and the decisions below in an `/opsx:explore` session on 2026-10-17.

Current state:

- **The AST has no statement type.** `BlockExpr { stmts: Vec<Expr> }` holds every item of a block. `ExprKind::LocalVar` and `ExprKind::Assign` are expression kinds that the parser only allows in statement position.
- **A block's value is implicit.** The type checker treats the last item as the block's value, or as a statement when the expected type is `Unit` (`check_block`).
- **Assignments already work as unbraced branch bodies,** typed `Unit` (`if (c) x = 1 else x = 2`, `0 -> x = 3`), and closures can assign captured variables (`{ x = 5 }`).
- **Ranges.** `..` is `BinaryOp::Range` with `Int` operands and the built-in `Range` type (`NominalId::builtin_range()`). `when` patterns have `WhenPattern::InRange { start, end }`, whose bounds are checked against the subject's type.
- **No loop keywords.** `while`, `do`, `for`, `break` and `continue` are ordinary identifiers. `in` is already a keyword.
- **This design supersedes one `syntax` rationale.** The `harden-parser-grammar` bullet "Declarations are statements, without a separate statement type" left a `Stmt` type to the typed IR. With loops, the source itself has statements that are not expressions, so the AST should model them. The typed tree (step 5) will mirror this `Statement` type rather than invent it.

## Goals / Non-Goals

**Goals:**
- The AST separates statements from expressions, and a block's value is an explicit tail.
- Loops parse and typecheck, `break`/`continue` are checked against their enclosing loop, and `..<` exists.
- Every new spec scenario has a `.prism` fixture, and `E4015` is exercised by one.

**Non-Goals:**
- Runtime behaviour. There is no backend; what loops mean at runtime is recorded in `docs/architecture.md` §4 as planned semantics.
- Control-flow analysis such as unreachable-code detection. MIR lowering (step 6) builds the graph.
- Loop labels, iterating anything but `Range`, loops as expressions, and diverging `while (true)`.

## Decisions

Decisions marked **(user)** were made by the user in the exploration session. The others follow from them or from existing rules, and are open to review here.

### D1. `Block { statements, tail }` with a `Statement` enum (user)

```
Block     { statements: Vec<Statement>, tail: Option<Box<Expr>>, span }
Statement = Local { is_var, name, ty, initializer }
          | Assign { target, value }
          | Expression(Expr)              // value discarded
          | While { condition, body: Block }
          | DoWhile { body: Block, condition }
          | For { variable, variable_span, iterable: Expr, body: Block }
ExprKind  gains Break and Continue; Return stays; Block(Block) replaces Block(BlockExpr)
```

- **Each `Statement` carries a span.** Names are spelled out (`Statement`, `Expression`, `condition`), per the project's naming rule. `BlockExpr` becomes `Block`; existing abbreviated names such as `Expr` stay.
- **The parser decides the tail.** If the last item parsed in a block is an expression statement, it becomes `tail`, and a trailing `;` doesn't change that, as in Kotlin. Rust's "`;` discards the value" rule was rejected because Kotlin users don't expect `;` to change meaning.
- **Why an explicit tail:** downstream stages (the typed tree, MIR lowering) never have to re-derive "is the last statement the value?", and the type checker's `check_block` becomes a loop over statements plus one optional tail check.
- **`when` statements are unchanged.** A `when` in tail position whose block expects `Unit` is still checked as a statement, so the "When statement needs no else branch" rule keeps its meaning.

### D2. Assignment is a statement; unbraced branch bodies may hold one

- `Assign` moves from `ExprKind` to `Statement` (user).
- **Unbraced branch and loop bodies keep accepting assignments.** To keep `if (c) x = 1 else x = 2`, `0 -> x = 3` and `while (c) i = i + 1` working (all valid in Kotlin), an unbraced body of `if`/`else`, a `when` branch, `while` or `for` may be an assignment. The parser wraps it in a `Block` with that one statement and no tail, spanning the assignment.
- **Typing such a branch:** it is `Unit`, exactly as today, so an `if` used as a value with an assignment branch still reports a mismatch.
- **Everywhere else an assignment is rejected (`E2001`),** for example a parenthesized assignment used as a value: `val y = (x = 1)`.
- **Rejected alternative:** keeping `Assign` as a `Unit`-typed expression kind. It would leave the "statement or expression?" ambiguity the change exists to remove.

### D3. Loop forms and typing (user)

- **`while (c) body`, `do { … } while (c)` and `for (x in range) body` are statements of type `Unit`,** as in Kotlin. There is no Rust-style `loop` and no `break` value.
- **Loop conditions read through references,** like `if` conditions (`check_operand` against `Bool`).
- **`do`/`while`: the condition is checked inside the body's scope** before that scope is exited, so the body's top-level locals are visible in it, as in Kotlin. The `do` body must be braced, since `do x = 1 while (c)` reads poorly and Kotlin code almost always braces it.
- **`for` iterates the built-in `Range` only.** Its iterable is checked against `Type::Struct(NominalId::builtin_range())` with `check_operand`, so it reads through references like conditions do, and reports `E4002` otherwise. The loop variable is bound as an immutable `Int` in a scope level between the enclosing block and the body, mirroring how parameters sit above a function body, so the body may shadow it. General iteration needs generics and traits, which come after the borrow checker.
- **Loops are not diverging.** `while (true)` with no `break` is still `Unit`, and a non-`Unit` function must end in a value or `return` after it. Treating a literal-`true` loop as `Nothing` is a pure relaxation and can be added later without breaking code; the MIR stage will know more about reachability than the type checker does today.

### D4. `break` and `continue` (user)

- **`ExprKind::Break` and `ExprKind::Continue` have type `Nothing`,** so `xs ?: break` and `if (c) continue` work as they do in Kotlin.
- **They never take an operand.** The parser treats them as complete expressions; anything after them on the same line hits the existing separator rule (`E2001`).
- **The checker tracks loop depth per body.** `BodyContext` gains a counter that `while`/`do`/`for` bodies increment, and closure bodies start at zero, like the compose-context stack. A jump at depth zero reports `BreakOutsideLoop` (`E4015`, the next free `E4xxx` code). Reporting it in the checker rather than the parser keeps the parser free of context stacks and puts the rule next to the closure-boundary logic that already exists.
- **Labels (`break@outer`) are deferred** (user). They are additive, and Kotlin's syntax for them is fixed, so deferring costs nothing.

### D5. `..<` half-open ranges (user)

- **A new `DotDotLess` token.** The lexer's `4..10` versus `4.10` handling extends so that `4..<10` lexes as `Int(4)`, `DotDotLess`, `Int(10)`.
- **In the AST,** `BinaryOp::HalfOpenRange` sits next to `BinaryOp::Range`, and `WhenPattern::InRange` gains `end_inclusive: bool`.
- **In the type checker,** both range operators give the same `Range` type, and both pattern forms are checked identically. Whether the end is included is a property of the runtime value: one `Range` type with an end-inclusive flag, not two types, so `when` patterns, `for` and future APIs treat both alike. The runtime meaning goes in `docs/architecture.md` §4.
- **Alternatives rejected:** closed ranges only (off-by-one prone; `0..n - 1` with `0..-1` for empty), and Kotlin's `until`, which is an infix function there and would be a special-cased keyword here.

### D6. Reserved keywords

`while`, `do`, `for`, `break` and `continue` become hard keywords, as in Kotlin, so they can't be identifiers. Soft keywords like `compose` need a token to disambiguate; a loop keyword at statement start can't, because `while (x)` would also read as a call. No program depends on these names yet.

### D7. Fixtures and tests

- **One fixture per new scenario** under `crates/prism-cli/tests/fixtures/loops/`, plus `syntax/` and `type-system/` for the modified requirements. The harness's catalog check then requires a fixture producing `E4015`.
- **Unit tests that build `BlockExpr` by hand** move to `Block`. The 317 existing tests must keep passing with only that mechanical change. The one exception is the scenario wording that changes in `type-system` (D1).

## Risks / Trade-offs

- **[Risk] The AST refactor touches every block-handling path** in the parser and the 2,300-line checker. → It's mechanical, and the existing fixture harness covers every current scenario through `prism check`, so a behaviour change shows up as a failing fixture. Do the refactor first with no new syntax and land the tests green, then add loops.
- **[Risk] The tail rule changes how some trailing items are classified.** A block whose last item is a `when` or `if` statement is now a tail rather than a statement. → The checker already decides by expected type (`Unit` → statement), and D1 keeps that, so typing is unchanged. The fixtures for "When statement needs no else branch" and "Discarded Block Values" guard it.
- **[Trade-off] No diverging `while (true)`.** Some functions need a dummy `return` after an infinite loop. → This can be relaxed later without breaking code (D3).
- **[Trade-off] Hard keywords** can't be used as names, for example a field called `for`. → Kotlin makes the same trade.

## Migration Plan

No users or stored artifacts. The ABI is unaffected, because bodies never reach it, so existing ABI hashes don't change.

## Open Questions

None. Everything the specs depend on is decided above.
