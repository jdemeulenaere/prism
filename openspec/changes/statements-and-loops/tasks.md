# Tasks

## 1. Statement AST refactor, no new syntax (D1, D2)

- [ ] 1.1 In `prism-ast`, add `Statement` (with `Local`, `Assign`, `Expression`) and `Block { statements, tail, span }`, replacing `BlockExpr`, and remove `ExprKind::LocalVar` and `ExprKind::Assign`. Verify `cargo build -p prism-ast`.
- [ ] 1.2 In `prism-parser`, build `Block`s. The last expression item becomes the tail, with a trailing `;` ignored; unbraced `if`/`else` and `when` bodies holding an assignment become a one-statement block with no tail; a parenthesized or otherwise nested assignment reports `E2001`. Verify with parser tests and fixtures for the new `syntax` scenarios "Block ending in an expression", "Block ending in an assignment", "Trailing semicolon keeps the tail", "Assignments as unbraced branch bodies" and "Parenthesized assignment as a value rejected".
- [ ] 1.3 In `prism-typechecker`, check blocks as statements plus an optional tail, keeping the `Unit`-expected statement rule for a tail `when`, and move `LocalVar` and `Assign` handling to statements. Verify that every existing unit test and fixture passes, and add a fixture for the reworded "Block statement evaluation and Unit typing" scenario.
- [ ] 1.4 Update the remaining crates and tests that build or walk blocks (`prism-abi` if it touches bodies, `prism-cli`, hand-built ASTs in unit tests). Verify `cargo test --workspace` is green with no new syntax yet.

## 2. Half-open ranges (D5)

- [ ] 2.1 Lex `..<` as `DotDotLess`, keeping `4..10`, `4.10` and `4..<10` distinct. Verify with lexer tests and a fixture for "Half-open range tokens".
- [ ] 2.2 Parse `..<` at `Range` precedence as `BinaryOp::HalfOpenRange`, and `in a..<b` as `WhenPattern::InRange { end_inclusive: false, .. }`; a newline before `..<` ends the expression. Verify with fixtures for "Half-open range precedence" and "Half-open range in a when pattern".
- [ ] 2.3 Type `..<` and `in a..<b` like their closed forms. Verify with fixtures for the three `type-system` "Half-Open Ranges" scenarios.

## 3. Loops (D3, D6)

- [ ] 3.1 Add the `while`, `do`, `for`, `break` and `continue` keyword tokens. Verify with a fixture for "Loop keyword as a variable name rejected" and a lexer test for all five.
- [ ] 3.2 Parse `while`, `do`/`while` and `for (name in expr)` as statements with mandatory parentheses. Bodies are a braced block or, for `while`/`for`, an unbraced expression or assignment; a declaration body reports `E2001`; a loop in expression position reports `E2001`. Verify with fixtures for "While condition without parentheses rejected", "Unbraced declaration body rejected", "Declaration as an unbraced loop body rejected" and "Loop as an initializer rejected".
- [ ] 3.3 Typecheck loops: `Bool` conditions read through references; the `do`/`while` condition is checked inside the body's scope; the `for` iterable must be the built-in `Range`; the loop variable is an immutable `Int` in its own scope level; loops are `Unit` statements. Verify with fixtures for every scenario of "While Loops", "Do-While Loops", "For Loops Over Ranges", "Loop Bodies" and "Loops Are Unit Statements".

## 4. Break and continue (D4)

- [ ] 4.1 Parse `break` and `continue` as operand-free expressions. Verify with a fixture for "Break takes no value".
- [ ] 4.2 Type them as `Nothing`, and add a loop-depth counter to the body context that loops increment and closure bodies reset. A jump at depth zero reports `BreakOutsideLoop` (`E4015`) naming the keyword. Verify with fixtures for every scenario of "Break and Continue" and "Jumps Outside a Loop Rejected".
- [ ] 4.3 Add `E4015` `BreakOutsideLoop` to the catalog in `openspec/specs/diagnostics/spec.md`, and render it in `prism-cli` with a help line. Verify the fixture harness's catalog check passes (every live code has a fixture).

## 5. Planned runtime semantics

- [ ] 5.1 In `docs/architecture.md` §4 ("Runtime Semantics Awaiting Code Generation"), add the planned evaluation of loops, `break`, `continue` and `a..<b` (excludes `b`; empty when `b <= a`) as requirements with scenarios. Verify the section reads as planned, not implemented.

## 6. Integration

- [ ] 6.1 Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `prism check` on every file under `examples/`, and `openspec validate --all --strict`. Verify all pass.

## Workflow follow-up

- On archive, promote D1–D6 into the `## Design Rationale` sections of `syntax` and `type-system`, and into the new `loops` spec. Replace the `harden-parser-grammar` bullet "Declarations are statements, without a separate statement type" with D1.
- Tick step 1 of "the road to the borrow checker" in `ROADMAP.md`, and add the non-goals as unchecked items: diverging `while (true)` and iteration beyond `Range`. Loop labels are already listed.
