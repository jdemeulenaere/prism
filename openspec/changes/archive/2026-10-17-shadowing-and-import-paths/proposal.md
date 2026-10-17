## Why

The user settled four questions left open after `typechecker-soundness` and the spec fixtures (2026-10-17). A local may shadow a parameter; a top-level declaration may shadow an imported name; naming a type known only through an import's signature reports `E4014`; and the stage that rejects `.` and `..` import segments was left to us. The specs and code disagree with these decisions or don't state them, and the `.`/`..` scenario (`E1006` from the resolver) is unreachable from source.

## What Changes

- **A local may shadow a parameter.** `fun f(a: Int): Int { val a = 2; return a }` is accepted, and `a` then refers to the local. Today's behavior; the spec now states it.
- **A top-level declaration shadows an imported symbol.** Declaring `fun helper(...)` in a file that imports a module exporting `helper` is accepted, and `helper` in that file refers to the local declaration, for values, callees and type names alike. **BREAKING:** this was `E4010` before.
- **Naming an unimported type in an annotation reports `E4014`.** If a file only knows `Order` through an import's signature (`shop/api` returns an `Order` from `shop/models`), writing `o: Order` reports `E4014` at the annotation with the import to add, instead of an opaque type whose first member access reported a misleading `E4006`. Same as Kotlin, where naming a type requires importing it.
- **The parser rejects `.` and `..` import segments** with a syntax error (`E2001`) whose message names the segment and points to root-anchored imports. The resolver keeps `E1006` for the import paths the parser accepts but resolution rejects: a file importing itself, and a sibling import with more than one segment. The resolver's own `.`/`..` check stays as a defensive guard.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `type-system`: "Lexical Scope and Symbol Resolution" (parameter shadowing), "Ambiguous Imported Names Rejected at Use" (local declarations shadow imports), and "Members of a Type Require Importing Its Module" (annotations).
- `package-resolution`: "Import Resolution and Workspace Anchoring" (which stage rejects `.`/`..`).

## Impact

- `crates/prism-typechecker/src/checker.rs`: `define_top_level` no longer reports `E4010` for a name that an import also exports.
- `crates/prism-parser/src/parser.rs`: `parse_import_decl` reports `.`/`..` segments with a dedicated message.
- `openspec/specs/diagnostics/spec.md`: the `E1006` catalog row no longer claims `.`/`..` segments.
- Fixtures under `crates/prism-cli/tests/fixtures/` that pin today's behavior with `FIXME(decision)` become ordinary `expect:` fixtures; regression tests in `crates/prism-typechecker/tests/regressions.rs` that expect `E4010` for an import collision change.
- `ROADMAP.md`: the three open decision items are ticked.
