# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Phase 2 Produces a Typed Tree per File
Phase 2 MUST check each file into a typed tree (the `typed-tree` spec) using only the file's own AST and the ABIs of its direct imports, never another file's bodies or typed tree, so that a file's tree is unaffected by edits to the bodies of the files it imports.

#### Scenario: Typed tree from the type checker
- **WHEN** the type checker checks a file
- **THEN** it produces a typed tree in which every node carries its type and span, names are resolved to bindings or declaration identities, calls are normalized to positional order with explicit default slots, and every implicit copy, move, borrow, copy-out and reborrow is an explicit node; the tree is produced even when the file has type errors

#### Scenario: Typed tree independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: &String): String = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-typed-tree app/main.prism` prints the tree of `app/main`, whose call names `lib/text::shout`, while `prism check app/main.prism` reports the `E4001` in `lib/text`
