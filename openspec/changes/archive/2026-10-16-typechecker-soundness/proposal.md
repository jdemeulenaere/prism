# Proposal: typechecker-soundness

## Why

A read-only audit of `crates/prism-typechecker` found 23 soundness and correctness problems. Most are plain bugs where the spec already implies the right behavior, and those are being fixed as plain commits with regression tests (AGENTS.md, "Change packet or plain fix?"). The eight findings in this change each need a decision someone could reasonably disagree with: new semantics, new or reassigned error codes, or an architectural choice. The borrow checker is the next ROADMAP item, and two of these (nominal identity and reading through references) shape what it can rely on. That is why they are settled now. Each repro was re-verified against `main` at `8fb2a5e`, after the plain-fix commits.

## What Changes

The numbers are the audit finding numbers.

- **3. Canonical nominal identity.** A struct or enum is identified by its defining module (root-relative path without extension, such as `a/lib`) plus its name, not by its bare name. Same-named types from different files are no longer conflated, a struct never equals an enum, and the built-in `Range` is distinct from a user `Range`. Annotations bind to that identity. This supersedes the "nominal types resolve lazily" rationale. **BREAKING** for programs that relied on name-only matching.
- **6. Reading through references.** Operator operands and `if`/`when` conditions read through `&T` and `&mut T`, so `a + 1` and `a == b` with `a: &Int` typecheck. A `&T` is still not accepted where a `T` is expected (argument, initializer, return). *(Decision D1: auto-deref versus an explicit `*`.)*
- **7. Return and argument mismatch codes.** A value checked against a return type reports `E4005`, and one checked against a parameter type reports `E4004`, as the spec and catalog already say. Today both report `E4002`. *(Decision D2: change the code to match the spec, or narrow the spec.)* This changes the code reported in some existing tests and scenarios.
- **11 (first part). Closure parameters with no expected type** report a new cannot-infer error, `E4013`, instead of silently typing as `Error`. *(Decision D6.)*
- **12. Order-independent inference.** Unannotated private top-level functions and values are inferred on demand, so the result no longer depends on declaration order. A declaration whose inference depends on itself (a recursive inferred function) reports `E4013`. **BREAKING:** recursive private functions without a return type, accepted today, are rejected. *(Decision D5.)*
- **15. Ambiguous imports.** When two imports export the same name and the file uses it, the checker reports a new error, `E4012`, at the use site. Unused collisions stay legal. A local declaration that collides with an import keeps reporting `E4010`. *(Decision D7.)*
- **16. Discarded block values.** A block body of a `Unit` function, or of a closure expected to return `Unit`, may end in an expression of any type. That expression is checked, and its value is discarded. *(Decision D8.)*
- **20. Structured, module-qualified ABI types.** Exported signatures record types as a tree, not a string. Each nominal name is bound to its defining module, using only the file's own declarations and its direct imports' exported names. `Type::parse_str` is removed, so higher-order types such as `((Int) -> Int) -> Int` and `(Int) -> Int?` survive across files. Every ABI hash changes once. *(Decision D4.)*
- **21. Types reachable only through an import.** A value whose type is defined in a module the file does not import directly can be received and passed along, but reading its members reports a new error, `E4014`, that names the import to add. This keeps builds hermetic: the checker still reads only direct imports' ABIs. *(Decision D9.)*

**Non-goals**
- **Handled by plain-fix commits, not by this change:**
  - 1 and 2: the mutability holes (fixed in `e113b9b`);
  - 4: a type name usable as a value (fixed in `d1a7787`);
  - 5: `&User` not matching `&User` (fixed in `e113b9b`);
  - 8: `package` visibility leaking through annotations (fixed in `d1a7787`);
  - 9: the right side of Elvis not checked (fixed in `8fb2a5e`);
  - 10: default values not checked (fixed in `8fb2a5e`);
  - 11 (second part): duplicate closure parameters (fixed in `8fb2a5e`);
  - 13: exhaustiveness counting variants by name;
  - 14: duplicate named arguments (fixed in `8fb2a5e`);
  - 17: `A??` nullable flattening;
  - 18: joining `if`/`when` branches with `null`;
  - 19: a statement-position `when` without `else`, and `when` over a `&Enum` subject;
  - 22: same-named variants in two enums;
  - 23: error cascades through `&`, `&mut` and `?` (fixed in `e113b9b`).
- Borrow checking, moves, copies and lifetimes.
- A typed IR, HIR or node IDs.
- Generics: type parameters, generic structs, monomorphization.
- Rejecting unknown type names in annotations. They stay opaque (already on ROADMAP).
- Closure syntax: parameterless lambdas, function-type parsing, closure parameter annotations and composable closures. These belong to `harden-parser-grammar`.
- Writing through a `&mut` primitive (`*r = v` or an equivalent), and copying a value out of a borrow where a `T` is expected.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `type-system`: nominal identity, reading through references, return and argument mismatch codes, closure parameter inference, order-independent inference, ambiguous imports, discarded block values, and imported signatures keeping their declared types.
- `compilation-and-abi`: exported signatures record structured, module-qualified types, and the ABI records the file's module path.

`diagnostics` has no requirement changes. Its catalog table, which is not a requirement block, gains `E4012` and `E4013`, and the descriptions of `E4002`, `E4004` and `E4005` are narrowed. That edit is a task.

## Impact

- **Code:**
  - `crates/prism-typechecker` (`types.rs`, `scope.rs`, `checker.rs`, `error.rs`);
  - `crates/prism-abi` (`signature.rs`, `hasher.rs`): structured types and module paths;
  - `crates/prism-cli` (`workspace.rs`, `diagnostics.rs`): module paths, Phase 1b name binding, help lines for the new codes.
- **Concurrent work:** a plain-fix session is editing `crates/prism-typechecker` at the same time. This change is applied after those fixes land and rebases onto them.
- **Tests:** regressions go in `crates/prism-typechecker/tests/regressions.rs`, which runs real source through the parser, the ABI extractor and the checker. Multi-file cases that need module paths go in `crates/prism-cli/tests/`.
- **ABI hashes:** every hash changes once. There is no `.abi` cache yet, so nothing on disk is invalidated.
- **Docs:** `openspec/specs/diagnostics/spec.md` catalog, and `ROADMAP.md` follow-ups at archive.
- **Sequencing:** `harden-parser-grammar` changes the AST and should be applied first; this change rebases onto it (see design.md, Migration Plan).
