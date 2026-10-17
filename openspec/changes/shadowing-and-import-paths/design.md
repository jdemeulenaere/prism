## Context

- `ScopeStack` already orders lookups innermost scope first, then module scope, then the import layer (`scope.rs`). A top-level declaration therefore already wins over an import at every use; `define_top_level` (`checker.rs`) adds an explicit `E4010` on top of that, which `typechecker-soundness` D7 kept pending this decision.
- Parameters and the outermost block of a function body live in different scope levels, so `val a = 2` shadowing parameter `a` is accepted today; the fixture `type-system/duplicate-symbol-declaration-rejected--local-duplicating-a-parameter` pins it with `FIXME(decision)`.
- `parse_import_decl` expects an identifier after `import`, `/` or `.`, so `import ../x`, `import ./x` and `import /a/../x` fail with a generic "expected identifier" `E2001`. The resolver's `E1006` check for `.`/`..` components is unreachable from source.

## Goals / Non-Goals

**Goals:** encode the user's two shadowing decisions and give `.`/`..` imports one owner with a helpful message.

**Non-Goals:**
- Shadowing warnings. Prism has no warnings channel (ROADMAP).
- Shadowing *within* one scope level: two declarations of the same name in the same block, the same parameter list or the same file still report `E4010`.
- Changing which imports the resolver accepts.

## Decisions

### D1. Locals may shadow parameters (user decision)
Parameters stay one scope level above the body's outermost block. A body-level `val`/`var` with a parameter's name shadows it from its declaration onward. Same as Rust; Kotlin accepts it with a warning, which Prism doesn't have.

### D2. Top-level declarations shadow imports (user decision)
`define_top_level` stops checking `is_imported`. The scope order already makes the local declaration win for values, callees and type names (`lookup`, `resolve`, `lookup_type`). A local declaration also removes any ambiguity: if two imports export `helper` and the file declares its own `helper`, uses resolve to the local one and `E4012` is not reported. This matches Kotlin, where a declaration in the file wins over a star import.

### D3. The parser owns `.` and `..` import segments
Diagnostic codes are grouped by stage (`diagnostics`: "Code range identifies the stage"), so a parser rejection must be `E2xxx`, not `E1006`. `parse_import_decl` checks for a `.`, `..` or `/`-led relative segment where a path segment is expected and reports `E2001` with: `import paths cannot contain '.' or '..'; use a root-anchored import such as 'import /core/fmt'`, spanning the offending segment.

`E1006` keeps the cases only resolution can see: a file importing itself, and a sibling import with several segments (`import doo.util`). The resolver's component check for `.`/`..` remains as a guard for import declarations built outside the parser.

**Alternative:** let the parser accept `.`/`..` as segments so the resolver reports `E1006`. Rejected: the grammar would accept paths the language forbids, only to reject them one stage later.

## Risks / Trade-offs

- **[Risk] Silent shadowing of an import can hide a mistake** (a local `helper` unintentionally replacing an imported one). → Accepted by the user; a future warnings channel can flag it.
