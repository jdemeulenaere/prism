# Design

## Context

See proposal.md for the motivation. These are the parts of the parser that matter, as they stand in the working tree on 2026-10-16. A plain-fix session is editing `crates/prism-parser` at the same time (a nesting budget in `parse_expr`/`parse_type`, `ParseError.code`, and `parse_prefix` split into one method per construct). The design below refers to roles, not line numbers.

- `val`/`var` are prefix expressions, so they parse in any operand position. `=` is an infix operator at `Precedence::Assign`, which rejects a chain only when one side of the `=` is itself an `Assign`.
- A `{` becomes a closure only when lookahead finds an `ident, … ->` header. Otherwise it is an `ExprKind::Block`, evaluated where it appears. A closure body is unwrapped to a bare expression when it has a single non-declaration statement, and is a `Block` otherwise.
- Only `(`, `[` and `{` consult `Token::is_preceded_by_newline`. Every other infix operator continues across newlines.
- Blocks, struct bodies, enum bodies and `when` bodies loop "parse item, then eat an optional `;` or `,`", so nothing requires a separator.
- `parse_type` returns `Unit` for `()` before checking for `->`, and it has no `compose` prefix. `TypeKind::Function` has no compose flag, and `Type::from_ast` hard-codes `is_compose: false`.
- The type checker's two closure arms (check mode against an expected `Fn`, and inference mode) already push the expected type's compose flag, or `false` in inference mode. Calls check each argument against its parameter type.

## Goals / Non-Goals

**Goals:**
- The AST says what the program means: a lambda is a `Closure`, a declaration sits at a statement boundary, and a line break ends a statement where a Kotlin reader expects it to.
- Each decision is pinned by tests that parse real `.prism` source.

**Non-Goals:**
- A separate `Stmt` type in the AST (D2).
- Making assignment statement-only, or validating assignment targets. Targets are a plain fix, and assignment stays an expression node with the existing non-associative rule.
- Any type-checker behaviour beyond what the new closure and function-type shapes need. `typechecker-soundness` owns the rest.

## Decisions

### D1. A brace is a block only in block position. **Confirmed by the user on 2026-10-16.**
The parser decides from position, not from lookahead:

| Position | `{` means |
|---|---|
| function body; `if`/`else` branch body; `when` branch body | block |
| start of a statement in a block or closure body | block (a nested scope) |
| anywhere else: operand, argument, initializer, trailing lambda, `return` value, expression function body | closure |

The `params ->` header is still found by the existing lookahead. The plain fix for `g(compose = 1)` may extend it to accept the contextual `compose` identifier. Without a header, the closure has zero parameters. `ExprKind::Closure { params: Vec<String>, body: BlockExpr }`: the body is always a block, so its shape no longer depends on how many statements it has. The body block's span runs from the `->` (or, with no header, from the first statement) through the closing `}`, so it differs from the closure's span and from any single statement's span. This keeps the convention the S3 plain fix introduced for headed closures.

**Statement-position braces stay blocks.** This departs from Kotlin, where `{ … }` as a statement is a lambda that is never called. Prism keeps it as a scope, because an explicit scope is the natural way to end a borrow early under the zero-cost memory invariant. A `{` at the start of a line was already never a trailing lambda, so this adds no ambiguity.

**`fun f() = { 1 }` returns a closure**, as in Kotlin. This is a known Kotlin pitfall, but a typed body (`fun f(): Int = { 1 }`) fails with a type mismatch that points at the cause, and a lint can warn later.

**Alternatives:**
- Closures only in trailing-lambda and argument position, with `val f = { 1 }` staying an eager block. Rejected: the meaning of `{ … }` would then hinge on argument versus initializer position, which is exactly the ambiguity a borrow checker cannot tolerate.
- Kotlin's rule everywhere, including statement position. Rejected because it gives up scope blocks for no gain.

### D2. Declarations are parsed by a statement parser, with no new AST type
A new `parse_statement` handles each element of a block or closure body. If the element starts with `val`/`var`, it parses a declaration; otherwise it parses an expression. `parse_prefix` no longer accepts `val`/`var`, so a declaration in any other position reaches the "unexpected token" path. That path gains a specific message for `val`/`var`: "a declaration is only allowed as a statement; wrap it in braces". Unbraced `if`/`else` and `when` branch bodies stay expressions.

A declaration's initializer, local or top-level, is parsed at `Precedence::Assign` so that it never consumes an `=`. A following `=` reports the existing "Chained assignments are not allowed" message. This closes the `val a = b = c` bypass without changing how assignment works elsewhere.

`ExprKind::LocalVar` stays inside `BlockExpr.stmts: Vec<Expr>`. A doc comment records the invariant that it appears only as a direct element of `BlockExpr.stmts`.

**Alternatives:**
- A `Stmt` enum. This is cleaner, but it rewrites every block consumer in the type checker that `typechecker-soundness` is changing at the same time. The typed IR planned before the borrow checker is the right home for it.
- Kotlin's rule that an unbraced branch body may be a declaration with its own scope. Rejected: Prism's `if` is an expression, a declaration there is almost always a mistake, and the audit's scope leak (`if (c) val x = 1` followed by `return x` typechecks) shows the cost.

### D3. Newline significance follows Kotlin
The parser keeps a stack of newline modes. `(` and `[` push *insignificant*, and `{` (block, closure or `when` body) pushes *significant*. The matching closer pops the mode. At the top level of a file and in declaration headers, the mode is significant.

In significant mode, an infix token preceded by a newline ends the current expression unless it is `.`, `?.`, `&&`, `||` or `?:`. Today's rule for `(`, `[` and a trailing `{` after a newline is the same check, so it keeps its behaviour. In insignificant mode, newlines never end an expression, so multi-line argument lists such as `g(a\n+ b)` keep working. Without this, they would regress.

The stack lives in the parser, not the lexer, because only the parser knows whether a `{` opens a block or a type-argument context. The existing `is_preceded_by_newline` flag is enough input.

**Alternative:** make only `+` and `-` newline-sensitive, which is the minimum that fixes the audit repros. Rejected: `*`, `==`, `..` and `=` after a newline would still silently join lines, and Kotlin readers expect Kotlin's exact set.

### D4. Separators are checked after each item
After a statement, the next token must be `;`, the body's closing `}`, or a token preceded by a newline; otherwise the parser reports "expected a newline or ';' between statements". Struct fields, enum variants and `when` branches use the same check, with `,` also accepted. This reuses the newline flag that D3 reads, and needs no new token.

**Alternative:** keep accepting `val a = 1 val b = 2`. Rejected: combined with D3 it would let `a\n-1` and `a -1` mean different things while `a b` silently means two statements, which is inconsistent and hides typos such as a missing operator.

### D5. Function types carry `is_compose`
In type position, `parse_type` treats `compose` followed by `(` as a composable function-type prefix; otherwise `compose` is a contextual type name. It checks for `->` after `()` before returning `Unit`. `TypeKind::Function` gains `is_compose: bool`. `Display` prints `compose (A) -> R`, which `Type::parse_str` already reads, and `Type::from_ast` copies the flag. A nullable function type needs parentheses, `(() -> Unit)?`, because `() -> Unit?` returns `Unit?`, as in Kotlin.

The compose flag is part of an exported function's ABI. A `compose` content parameter is a different call contract from a plain one, so changing it must change the hash. It already does, because parameter types are hashed by their string.

`Type::parse_str` still mis-splits nested function types (audit C6, owned by `typechecker-soundness`, whose plan replaces string types with structured ones). Until that lands, a higher-order type in an exported signature does not round-trip across files. The parser scenarios here do not depend on that round trip.

### D6. No multi-error recovery in this change
The parser stays fail-fast, and the existing ROADMAP item ("Parser error recovery … needed for the LSP") stays open. Reasons:
- Recovery needs error nodes in the AST, or a policy for half-built nodes, and the AST is changing in this very change and again in the planned typed IR.
- Synchronization points depend on D3 and D4 (newline and separator rules), so they should settle before recovery is built on them.
- Fail-fast with exact spans is enough for `prism check` and for the borrow-checker work that comes next.

**Alternative considered:** a minimal recovery that resynchronizes at the next top-level `fun`/`struct`/`enum`/`val`/`var` and reports one error per declaration. It is cheap and useful, but it would need `parse_source` to return several errors (an API change for the CLI and tests) and a decision on whether to typecheck the declarations that did parse. That is better done together with the LSP.

### D7. Minimal type-checker adaptation
Only these checker sites change:
- Both `ExprKind::Closure` arms read `body: BlockExpr`. Check mode checks the block against the expected return type, the same way a block-bodied function is checked. Inference mode infers the block.
- `Type::from_ast` copies `is_compose`.
- Hand-built test ASTs that construct `Closure` or `TypeKind::Function` get the new fields.

Parameterless closures then fall into the existing closure paths. Those paths already push the expected type's compose flag and report a mismatch against an expected type that is not a function.

### D8. Sequencing with the plain fixes and with `typechecker-soundness`
- **Plain fixes first.** The parser plain fixes (nesting budget, named `compose` argument, bare `return`, token splitting, spans, lexer-error messages) are being committed now. Apply this change on top of them. The new parsing paths (statement parser, closure body, function types) must enter the nesting budget the same way existing paths do. The bare-`return` fix's "can this token start an expression" test should treat newlines the way D3 does.
- **`typechecker-soundness` second.** This change alters `ExprKind::Closure` and `TypeKind::Function`, so apply it first and rebase `typechecker-soundness` onto it. The `type-system` delta here only ADDS requirements. `typechecker-soundness` plans to move their `E4002` scenarios to `E4004` (argument sites) after this change is archived.
- **ADDED-only syntax delta.** The plain-fix rule lets those commits add scenarios to `openspec/specs/syntax/spec.md` directly. A MODIFIED block here would silently drop them at archive, so every syntax requirement here is ADDED. The existing "Trailing Lambda and Multi-Statement Closure Syntax" scenarios still hold: their wording says "block", but it describes the trailing argument's braces, which are now a closure. The archive step rewords them.

## Risks / Trade-offs

- [The newline rule changes the meaning of existing multi-line code] → Only code that starts a line with a non-continuation binary operator changes, and that code is mis-parsed today. Every example and test source is re-run (tasks 6.1 and 6.2, plus the separator and newline tests in group 5).
- [`fun f() = { … }` now returns a closure] → This matches Kotlin, and a typed body reports a mismatch that points at the closure.
- [Statement-position braces are scopes, not lambdas] → This is recorded in the syntax spec's rationale at archive.
- [A concurrent parser rewrite conflicts with this change's edits] → This change is applied only after the plain fixes are committed (D8), and its tasks refer to roles rather than line numbers.
- [ABI strings of `compose` function types change] → No `.abi` cache exists yet, and such types could not be written before this change.

## Migration Plan

No persisted artifacts need migrating. Before archiving, run `prism check` on every file under `examples/` and fix any source that relied on the old grammar. None is expected: the examples use trailing lambdas, which keep their meaning. Rolling back is a revert.
