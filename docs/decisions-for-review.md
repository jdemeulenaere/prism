# Design Decisions for Review

On 2026-10-17 the user asked for autonomous iteration up to Milestone 2, with questions only for decisions that would be almost impossible to revert later. Every other design decision made on the user's behalf is logged here, so that the user can review them once Milestone 2 is complete.

- Each entry points at the change whose `design.md` holds the full reasoning (in `openspec/changes/<name>/` while the change is open, `openspec/changes/archive/<date>-<name>/` once archived) and, after archiving, at the main spec's `## Design Rationale`.
- **Revert cost** is a rough estimate of the work to change the decision later: *low* (local edit), *medium* (one change packet), *high* (touches several subsystems or much code).
- Decisions that simply follow from an existing spec or invariant are not listed.

## Process decisions

| # | Decision | Alternatives | Revert cost |
|---|----------|--------------|-------------|
| P1 | Each change runs as subagents: a proposer writes the packet (commit "Propose X"), an implementer applies it (commit "Implement X"), a reviewer checks the code against the spec, fixes gaps and archives (commit "Archive X"). Commits go to `main`, as in the existing history; nothing is pushed. | Feature branches or worktrees per change. | low |
| P2 | Proposal-time questions that the user would normally answer in `/opsx:explore` are decided by the proposer and marked **(autonomous)** in `design.md`. | Asking the user for each. | n/a |
| P3 | The order of changes after the agreed "road to the borrow checker" is set in `ROADMAP.md` under "Road to Milestone 2". | Other orders. | low |

## Language and compiler decisions

Entries are appended per change, in the order the changes are proposed.

| Change | # | Decision | Alternatives considered | Revert cost |
|--------|---|----------|-------------------------|-------------|
| extension-methods | D2 | Modifier order fixed: `[visibility] [compose] [mut \| consuming] fun T.name`; `consuming` is a contextual keyword like `compose`. | Free Kotlin-style modifier order; hard keyword `consuming`. | low |
| extension-methods | D3 | A receiver is a single type name; type arguments, `T?`, `&T` and function types are `E2001` for now. | Nullable receivers (`fun String?.orEmpty()`) and generic built-in receivers now. | low |
| extension-methods | D4 | `this` is its own AST node and a `val` binding in every mode (also `consuming`); outside an extension it is `E4001`. | `Ident("this")`; a mutable `this` in `consuming fun`; a new error code. | low |
| extension-methods | D5 | In extension bodies a bare call `f()` also resolves to an extension on the receiver (`this.f()`), and receiver members come before top-level declarations and imports. | Implicit `this` for fields only; module declarations first. | medium |
| extension-methods | D6 | Built-in `clone()` of a `Clone` type wins over an (imported) extension named `clone`: members before extensions. | Extension before built-in. | low |
| extension-methods | D7 | An extension named like a field of its receiver struct is rejected (`E4010`), and all rejected extension declarations reuse `E4010`. | Accept it silently (field wins at calls); a new error code. | low |
| extension-methods | D8 | Receivers auto-deref through any number of `&`/`&mut`/`Heap<T>` layers; `mut fun` on a temporary is accepted; `consuming` out of a `Heap<T>` is `E4017` until the `Heap<T>` change. | One layer only; exact static type; rejecting mutable temporaries. | low |
| extension-methods | D10 | `u?.f()` returns `R?`; a `?.` chain of fields over a place (`t?.lead?.greet()`) is borrowed where it lives (an optional place) for receivers only. | Treat the chain as a temporary (moves non-`Copy` fields, `E4017`), as `&u?.nick` still does. | medium |
| extension-methods | D11 | Calling an imported extension does not require importing its receiver type's module. | Require the receiver module too (`E4014`). | low |
| extension-methods | D12 | ABI: extensions are `ExportedSymbol::Function` with `receiver: Option<{mode, type}>`; the sort key includes the receiver. | A separate `Extension` symbol kind; extensions listed on the receiver's record. | low |
| extension-methods | D14 | No new error codes: `E4001`, `E4003`, `E4006`, `E4010`, `E4012`, `E4017` are widened. | New codes for shadowed extensions or `this` misuse. | low |
| extension-methods | D15 | Methods get a new `extension-functions` capability spec. | Put the rules in `type-system`. | low |
