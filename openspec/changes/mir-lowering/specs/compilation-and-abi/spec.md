# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Phase 2 Lowers Each Typed File to MIR
Phase 2 MUST lower the typed tree of every file without type errors to MIR (the `mir` spec) from that tree alone, so that a file's MIR, like its typed tree, depends only on the file and the ABIs of its direct imports. A file with type errors MUST NOT be lowered.

#### Scenario: Lowering to MIR
- **WHEN** a file has no type errors
- **THEN** each function, top-level `val`, default value and closure body is lowered to a place-based control-flow graph of basic blocks: statements assign rvalues to places (a local plus projections) and mark storage, terminators branch, call, return and drop, and `&&`, `||`, `?.`, `?:`, `when` and `for` become explicit branches; `prism dump-mir <file.prism>` prints it

#### Scenario: MIR independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: &String): String = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-mir app/main.prism` prints the MIR of `app/main`, whose call names `lib/text::shout`, while `prism check app/main.prism` reports the `E4001` in `lib/text`
