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
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D2 | Modifier order fixed: `[visibility] [compose] [mut \| consuming] fun T.name`; `consuming` is a contextual keyword like `compose`. | Free Kotlin-style modifier order; hard keyword `consuming`. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D3 | A receiver is a single type name; type arguments, `T?`, `&T` and function types are `E2001` for now. | Nullable receivers (`fun String?.orEmpty()`) and generic built-in receivers now. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D4 | `this` is its own AST node and a `val` binding in every mode (also `consuming`); outside an extension it is `E4001`. | `Ident("this")`; a mutable `this` in `consuming fun`; a new error code. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D5 | In extension bodies a bare call `f()` also resolves to an extension on the receiver (`this.f()`), and receiver members come before top-level declarations and imports. | Implicit `this` for fields only; module declarations first. | medium |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D6 | Built-in `clone()` of a `Clone` type wins over an (imported) extension named `clone`: members before extensions. | Extension before built-in. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D7 | An extension named like a field of its receiver struct is rejected (`E4010`), and all rejected extension declarations reuse `E4010`. | Accept it silently (field wins at calls); a new error code. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D8 | Receivers auto-deref through any number of `&`/`&mut`/`Heap<T>` layers; `mut fun` on a temporary is accepted; `consuming` out of a `Heap<T>` is `E4017` until the `Heap<T>` change. | One layer only; exact static type; rejecting mutable temporaries. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D10 | `u?.f()` returns `R?`; a `?.` chain of fields over a place (`t?.lead?.greet()`) is borrowed where it lives (an optional place) for receivers only. | Treat the chain as a temporary (moves non-`Copy` fields, `E4017`), as `&u?.nick` still does. | medium |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D11 | Calling an imported extension does not require importing its receiver type's module. | Require the receiver module too (`E4014`). | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D12 | ABI: extensions are `ExportedSymbol::Function` with `receiver: Option<{mode, type}>`; the sort key includes the receiver. | A separate `Extension` symbol kind; extensions listed on the receiver's record. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D14 | No new error codes: `E4001`, `E4003`, `E4006`, `E4010`, `E4012`, `E4017` are widened. | New codes for shadowed extensions or `this` misuse. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D15 | Methods get a new `extension-functions` capability spec. | Put the rules in `type-system`. | low |
| [extension-methods](../openspec/changes/archive/2026-10-17-extension-methods/design.md) | D16 | Every `recv.name(...)` receiver is classified (place read in place, else consumed temporary) before its member is resolved, so field calls and `clone()` on a branch over fields now move them (`E4017`). | Speculative inference of the receiver; in-place reads for fields and `clone()` only. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D1 | The type checker builds the typed tree directly while checking (one change, staged in task groups); the double check of an `else`-less `if` is fixed. | A second pass from side tables keyed by AST ids (Rust HIR to THIR); annotating the AST in place (Swift). | high |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D2 | `Type`, `NominalId`, `TypeDef` move into `prism-typed-tree`; the checker keeps lowering and assignability (a `TypeRelations` trait). | Separate `prism-types` crate; interned type ids. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D3 | Per-body arenas with `u32` ids; parameter defaults are expressions of their function's body. | Boxed tree; one arena per file; a body per default. | medium |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D4 | Locals are `LocalId`s; declarations are module-qualified paths (module, receiver, name) as in the ABI; fields and variants by index. | Global numeric definition ids; names only. | medium |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D5 | Explicit place nodes; consumed places wrapped in `Move`/`Copy`; borrows over places with an origin; reborrow and copy-out are a borrow or copy of a `Deref`; borrowed non-places are `Temporary` places. | One node kind per checker `Adjustment`; move-versus-copy decided in MIR by type. | medium |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D6 | Every subtyping step is an explicit `Coerce` node (`WrapNullable`, `Diverge`, `Subtype`), so a node's type always equals its slot's type. | Type-directed lowering in MIR. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D7 | `?.` and nullable conversions are Swift-style chains (`SafeNavigation` + `NonNullPayload`); a `?.` on a chain joins it, so payloads stay places. | Desugaring to `when` with synthetic locals; a `safe` flag on nodes. | medium |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D8 | Calls: callee is a path, a `BuiltinFunction` (`Clone`) or a value; arguments in parameter order with `Default` slots and the source evaluation order; receiver first; `is_compose`; constructions are `Construct` nodes. | Source-order arguments; constructors as calls. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D9 | Control flow stays structured (MIR makes branches); a discarded block tail becomes a statement; `break`/`continue` name their loop; `when` records exhaustiveness. | Desugaring loops and `when` in the tree. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D11 | Closures live in their parent body's arenas and record captured locals in first-use order; capture modes are left to the borrow checker. | A separate body per closure; recording modes now. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D12 | Error nodes of the error type keep the checked subexpressions; the builder adds no diagnostics; error-free trees have no error node or type. | Dropping erroneous subtrees; checking more on error paths. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D13 | `prism dump-typed-tree <file>` (and later `prism dump-mir`) prints stable indented text; fixtures hold `typed-tree.snap`, re-blessed with `PRISM_BLESS=1`. | A `--dump-typed-tree` flag on `check`; JSON. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D14 | A validator checks the tree invariants of every passing fixture. | Snapshot fixtures only. | low |
| [typed-tree](../openspec/changes/typed-tree/design.md) | D16 | A new `typed-tree` capability; the pipeline step goes into `compilation-and-abi`. | Everything in `compilation-and-abi` or `type-system`. | low |
