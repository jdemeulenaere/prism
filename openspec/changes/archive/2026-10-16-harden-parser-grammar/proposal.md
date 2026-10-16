# Proposal

## Why

The borrow checker is the next milestone, and it works directly on the parser's AST. A read-only audit found five grammar questions that the specs do not settle and that the current parser answers wrongly. I re-verified all five against the working tree on 2026-10-16, including the plain-fix commits in progress. A parameterless lambda is an eagerly evaluated block, `val` is an expression, a newline does not end an expression before a binary operator, statements need no separator, and two function-type forms cannot be written. Each answer is a grammar decision that changes what programs mean, so it goes through review here. Clear bugs are fixed separately as plain commits (see Non-goals).

## What Changes

- **BREAKING: parameterless lambdas are closures.** `Column { Item() }`, `run { 1 }`, `g({ 1 })` and `val f = { 1 }` produce a closure with zero parameters instead of a `Block` evaluated in the caller. A `{` is a block only in block position: a function body, an `if`/`else` or `when` branch body, or a statement. A closure body is always a block. The type checker is adapted only as far as the new node requires. As a result, `takesInt { 42 }` is rejected (`E4002`), and `later { Child() }` inside a `compose fun` reports `E4008` instead of a type mismatch.
- **Function types.** `() -> T` and `compose (A) -> T` parse. The AST function type records `is_compose`, `Type::from_ast` keeps it, and the ABI type string prints it. Today `onClick: () -> Unit` cannot be written, and composable content lambdas cannot be typed.
- **BREAKING: declarations are statements.** `val`/`var` are accepted only as statements in a block or closure body. `g(val z = 1)`, `x = val y = 3` and `if (c) val x = 1` become syntax errors. A declaration's initializer cannot be an assignment, so `val a = b = c` is rejected like `a = b = c`.
- **BREAKING: a newline before a binary operator ends the expression**, following Kotlin's rule. `val a = b` followed by `-1` on the next line is two statements, and `0 -> a` followed by `-1 -> b` inside a `when` is two branches. Only `.`, `?.`, `&&`, `||` and `?:` continue an expression across a newline. Newlines are insignificant inside `( )` and `[ ]`.
- **BREAKING: separators are required on one line.** Two statements, struct fields, enum variants or `when` branches on the same line need a `;`, or a `,` where one is already accepted. `val a = 1 val b = 2` becomes a syntax error.
- **Multi-error recovery: none in this change.** The parser stays fail-fast. Recovery remains deferred to the LSP work, as `ROADMAP.md` already records (design.md D6).

## Non-goals

These are handled by plain-fix commits, because the spec already states or implies the right behaviour:
- the lexer panic on a backslash before a non-ASCII character;
- the nesting-depth limit (`E2002`) and the stack overflow when dropping deep trees;
- the `Parser::new(vec![])` panic;
- a bare `return` before `else`, `,`, `)` and similar tokens;
- `when (x) { -> 1 }` accepted with no pattern;
- validating assignment targets;
- `g(compose = 1)` as a named argument;
- splitting the `>=` and `&&` tokens;
- literal edge cases: unknown escapes, `1e999` and `i64::MIN`;
- the span bugs S1–S4;
- reporting lexer errors with their own message instead of `Unexpected token '<Error: …>'`.

Also out of scope:
- node IDs, resolved names, binding-site spans and typed closure parameters, which belong to a later typed-IR change;
- methods, `this` and loops;
- implicit `it` closure parameters;
- every pure type-checker finding, which `typechecker-soundness` owns.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `syntax`: closure versus block positions, function-type grammar, statement-only declarations, newline rules for binary operators, and required separators.
- `type-system`: parameterless closures are checked as closures, and a written function type keeps its `compose` flag.

## Impact

- `crates/prism-ast`: `ExprKind::Closure { params, body: BlockExpr }`, `TypeKind::Function { is_compose, .. }`, and their `Display` output.
- `crates/prism-parser`: statement parsing, brace handling by position, a newline-significance stack, separator checks, and function-type parsing.
- `crates/prism-typechecker`: the two closure arms read a `BlockExpr` body, and `Type::from_ast` copies `is_compose`. Nothing else in the checker changes.
- `crates/prism-abi`: span-free hashing of the new closure shape. ABI strings of `compose` function types begin with `compose `. No `.abi` caches exist yet, so nothing needs migrating.
- Sequencing: apply this after the in-flight parser plain fixes are committed, and before `typechecker-soundness` (or rebase that change onto this one).
