# Tasks

Prerequisite: the parser plain fixes listed under the proposal's Non-goals are committed on `main` (design D8). Every regression test below parses real `.prism` source with `prism_parser::parse_source`, never a hand-built AST. Parser tests go in `crates/prism-parser/tests/parser_tests.rs`. Type-checker tests go next to the existing source-parsing tests (`crates/prism-typechecker/tests/regressions.rs`, or the `parse_source` helpers in `checker.rs`).

## 1. Closure AST shape

- [x] 1.1 Change `ExprKind::Closure` to `body: BlockExpr`, and update its `Display` in `crates/prism-ast/src/expr.rs` and its arm of the span-free body hash in `crates/prism-abi/src/signature.rs`; verify that `cargo build --workspace` succeeds after task 1.2 and `cargo test -p prism-abi` passes
- [x] 1.2 Adapt the two `ExprKind::Closure` arms in `crates/prism-typechecker/src/checker.rs` to the block body: check mode checks the block against the expected return type, and inference mode infers it. Update hand-built test ASTs and change nothing else (design D7). Verify that the existing closure and compose-isolation tests pass

## 2. Function types

- [x] 2.1 Add `is_compose` to `TypeKind::Function`; parse `() -> T`, `compose (…) -> T` and right-associative arrows in `parse_type`; and print `compose ` in `Display` (design D5). Verify with parser tests for `() -> Unit`, `compose () -> Unit`, `compose (Int) -> Unit`, `(Int) -> (Int) -> Int`, `(() -> Unit)?` versus `() -> Unit?`, and that `(Int, String)` is still rejected as a tuple
- [x] 2.2 Copy `is_compose` in `Type::from_ast`. Verify with type-checker tests that `compose fun P(content: compose () -> Unit) { content() }` is clean, and that passing a `compose () -> Unit` value to a `() -> Unit` parameter reports `E4002`
- [x] 2.3 Verify with an ABI test that the exported `public fun Column(content: compose () -> Unit): Unit` has the parameter type string `compose () -> Unit`, and that its hash differs from the same signature without `compose`

## 3. Braces by position

- [x] 3.1 Parse `{` by position (design D1): a block in function-body, branch-body and statement positions, and a closure everywhere else, with zero parameters when there is no `->` header and the body span from the first body token through `}`. New paths enter the nesting budget like the existing ones. Verify with parser tests that `Column { Item() }`, `Button(onClick = handler) { Text("Click") }`, `g({ 1 })`, `Button(onClick = { count = 1 })`, `val f = { 1 }` and `fun f() = { 1 }` yield zero-parameter closures; that `run { x -> x + 1 }` has a one-statement body block; that `if (c) { a() } else { b() }`, `0 -> { a() }` and a statement-position `{ val t = 1 }` stay blocks; and that the closure body span differs from the closure's span and its single statement's span
- [x] 3.2 Verify with type-checker tests that `fun takesInt(n: Int) { }` plus `takesInt { 42 }` reports `E4002`; that `fun run(body: () -> Int): Int = body()` plus `fun f(): Int = run { 1 }` is clean; that `later { Child() }` inside a `compose fun`, with `later(f: () -> Unit)`, reports `E4008`; and that `Column { Item() }`, with `compose fun Column(content: compose () -> Unit)`, is clean

## 4. Declarations in statement position

- [x] 4.1 Add `parse_statement` for block and closure bodies, remove `val`/`var` from `parse_prefix`, give the unexpected-token path the "a declaration is only allowed as a statement" message, and document the statement-only invariant on `ExprKind::LocalVar` (design D2). Verify with parser tests that `g(val z = 1)`, `x = val y = 3`, `fun f() = val x = 1` and `if (c) val x = 1` report `E2001` with that message, while `if (c) { val x = 1 }` parses
- [x] 4.2 Parse local and top-level declaration initializers at `Precedence::Assign`, and report the existing chained-assignment message when `=` follows. Verify with parser tests that `val a = b = c`, inside a function and at top level, is rejected, and that `a = b = c` is still rejected
- [x] 4.3 Verify with a type-checker test that `fun f(c: Bool): Int {\nif (c) val x = 1\nreturn x\n}` fails to parse, so the scope leak can no longer reach the checker

## 5. Newlines and separators

- [x] 5.1 Add the newline-mode stack (design D3): `(` and `[` make newlines insignificant, and `{` makes them significant. In significant mode, a newline before any infix token other than `.`, `?.`, `&&`, `||` and `?:` ends the expression. Verify with parser tests that `val a = b\n-1` and `if (c) { foo() }\n-1` are two statements each; that `when (x) {\n0 -> a\n-1 -> b\nelse -> c\n}` has the patterns `0`, `-1` and `else`; that `a\n&& b` and `user\n?.name\n?: "anon"` continue; that `g(a\n- 1)` and `xs[i\n+ 1]` join; that `g({ a\n-1 })` has a two-statement closure body; and that `cleanup()\n(1 + 2).toString()` is still two statements
- [x] 5.2 Require a newline or `;` between statements, and a newline, `,` or `;` between struct fields, enum variants and `when` branches (design D4). Verify with parser tests that `{ val a = 1 val b = 2 }`, `struct P { val x: Int val y: Int }`, `enum E { A B }` and `when (x) { 0 -> a 1 -> b }` report `E2001`, while their `;`/`,` and multi-line forms parse

## 6. Integration

- [x] 6.1 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (0 warnings) and `cargo test --workspace`, and verify all pass
- [x] 6.2 Run `prism check` on every file under `examples/`, and verify the results match those before this change. Only files that need an unbuilt subsystem, such as `examples/demo/ui`, may fail, and only for the same reasons
- [x] 6.3 Run `openspec validate --all --strict` and verify it passes

## Workflow follow-up

- Rebase or apply `typechecker-soundness` after this change (design D8).
- Archive with `/opsx:archive`, merging the `syntax` and `type-system` deltas. In the same step, reword the existing "Trailing Lambda and Multi-Statement Closure Syntax" scenarios in `openspec/specs/syntax/spec.md` to say the trailing braces are a closure, and promote D1 to D4 and D6 into the syntax spec's `## Design Rationale`. The "Fail-fast errors" bullet stays, and D6 adds why recovery is still deferred.
- At archive, add these to `ROADMAP.md` as unchecked items naming `harden-parser-grammar`: a typed IR with node IDs, binding spans and typed closure parameters; a `Stmt` AST type; methods, `this` and loops; implicit `it`; and a lint for `fun f() = { … }`. The multi-error recovery item already exists.
