# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or per form of a scenario with several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. `typed-tree` cases hold snapshots and executable cases `stdout.snap`/`stderr.snap` (`PRISM_BLESS=1`), each read against its scenario's THEN before committing. Formatter cases are golden files under `crates/prism-formatter/tests/golden/`.

## 1. Syntax tree and formatter (D2, D3)

- [x] 1.1 In `prism-ast`, add `arrow: Option<Span>` to `ExprKind::Closure` and print `{ -> ... }` for an empty header in `Display`. In `prism-parser`, store the arrow in `parse_closure`. In `prism-abi`, hash whether a closure has a header in `hash_expression_span_free`. Update every construction and pattern of `ExprKind::Closure`. Verify with parser tests for "it as a declared name", "A header-less closure has no parameters in the syntax tree" and "An empty parameter header is recorded", fixtures under `syntax/` for each (expecting `ok`), and that `cargo test -p prism-parser -p prism-abi` passes.
- [x] 1.2 In `prism-formatter`, print the header whenever `arrow` is set (including `{ -> }` for an empty body) and erase the arrow's span in `verify.rs`. Verify with golden files for "Empty headers and implicit it kept" and that `cargo test -p prism-formatter` passes, the corpus included.

## 2. Typing (D1, D3, D4, D6)

- [x] 2.1 In `prism-typechecker`, pass the header to `check_closure` from its three callers and bind a header-less closure's implicit `it` when exactly one parameter is expected (`LocalKind::ImplicitClosureParameter`, added in 3.1, span of the `{`, the expected `escaping` flag). Verify with fixtures under `type-system/` for every scenario of "Implicit Closure Parameter" and "Implicit Parameters Shadow Outer Names" (one case per form), and that `cargo test -p prism-typechecker` passes.
- [x] 2.2 Record the innermost closure's implicit-parameter context in `enter_closure`/`exit_closure`, report `TypeError::UnresolvedImplicitParameter` (`E4001`) from `resolve_ident` for an unbound `it` with D6's help, and replace the closure arity `CallSignatureMismatch` by `TypeError::ClosureParameterCount` (`E4004`) with D6's message and help. Render both helps in `prism-cli`. Verify with fixtures under `type-system/` for every scenario of "Closures Without an Implicit Parameter" and "Header-Less Closures Expecting Several Parameters" (one case per form), with the help lines checked by a CLI test or snapshot.
- [x] 2.3 In `openspec/specs/diagnostics/spec.md`, widen the `E4001` row (an unbound `it`, with its help) and the `E4004` row (a closure's parameter count). Verify that the fixture harness's catalog check passes.

## 3. Typed tree (D5)

- [x] 3.1 In `prism-typed-tree`, add `LocalKind::ImplicitClosureParameter`, its dump label `implicit closure parameter` (and `escaping implicit closure parameter`), and accept it where the validator accepts closure parameters. Verify with a fixture under `typed-tree/` with a snapshot for "Implicit parameter in the tree", and that every fixture's tree still validates.

## 4. Borrows and execution (D4)

- [x] 4.1 Confirm that MIR, `prism-borrowck` and `prism-codegen` need no change. Verify with fixtures under `references/` for every scenario of "Implicit Closure Parameters Borrow and Escape Like Declared Ones", and executable fixtures under `execution/` for every scenario of "Closures With an Implicit Parameter Run Like Declared Ones", run under `--check-allocations`.

## 5. Integration (D8)

- [x] 5.1 Add the D8 shapes to `crates/prism-cli/tests/compile_timing.rs` and run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`; verify that growth stays roughly linear.
- [x] 5.2 Use `it` in `examples/` where a one-parameter closure reads better with it. Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings), `cargo test --workspace`, `prism check`, `prism fmt --check` and `prism run --check-allocations examples/run/main.prism` on `examples/` (only `examples/demo/ui` may fail), and `openspec validate implicit-it --strict`. Verify all pass.

## Workflow follow-up

- On archive, promote D2 and D3 into `syntax`'s `## Design Rationale`, D1, D4 and D6 into `type-system`'s, D5 into `typed-tree`'s and D2's formatter half into `formatting`'s.
- In `ROADMAP.md`, tick "Implicit `it` parameter for single-parameter closures", and add as unchecked items, each naming `implicit-it`: destructuring closure parameters, parameter type annotations in closure headers, `_` for unused closure parameters, a lint for an `it` that shadows an outer `it`, the `E5007` help for a closure parameter (see Risks), and the super-linear checking and lowering of calls nested through their closure arguments (D8). Note in the `state-handles` follow-up that its handlers can now use `it`.
