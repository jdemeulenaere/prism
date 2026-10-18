# Prism Agent Guidelines (The Elephant & Goldfish Operating Manual)

Welcome to the **Prism** language repository. This document serves as the primary onboarding directive for AI agents and human contributors.

---

## 1. The Core Philosophy: The Elephant & Goldfish Model via OpenSpec
This project strictly follows the **Elephant-Goldfish model** (see [Rensin, 2024](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874)) using [OpenSpec](https://github.com/Fission-AI/openspec):
- **The Elephant (Durable Memory)**: All architectural invariants, grammar specifications, decisions, and roadmaps are permanently documented in [`openspec/specs/`](openspec/specs). When starting a task, **always consult the Elephant specs** to ensure alignment.
- **The Goldfish (Ephemeral Execution)**: Tasks are proposed and executed via [`openspec/changes/<change-name>/`](openspec/changes) work packets:
  - `proposal.md`: Motivation and scope boundary.
  - `design.md`: Technical approach, trade-offs, and design decisions (eliminates separate ADR drift).
  - `tasks.md`: Atomic, test-driven checklist.
  - `specs/`: Delta specifications with concrete scenarios (WHEN/THEN).
  - Once verified, the change is archived to `openspec/changes/archive/` and its delta specs merge into `openspec/specs/`.
- **Change packet or plain fix?** Not every change needs a change packet:
  - **Plain fix** (no change packet): the code is wrong and the spec already states, or obviously implies, the correct behaviour. Crashes always count. Commit the fix with a regression test that runs real `.prism` source (not a hand-built AST), named after the spec scenario it protects. If the spec was silent on that case, add the missing scenario to the main spec in the same commit; that pins down existing intent rather than changing behaviour.
  - **OpenSpec change**: the fix needs a decision someone could reasonably disagree with. That covers new semantics, a grammar rule, a new or reassigned error code (beyond taking the next free code for a new error kind, per [`diagnostics`](openspec/specs/diagnostics/spec.md)), changing an existing scenario, or an architectural choice.
  - Quick test: if you'd want to review the *design* before seeing code, propose a change; if you'd only want to review the *diff*, make a plain fix.
- **OpenSpec Workflows / Slash Commands**:
  - `/opsx-propose "change-name"`: Creates and populates the change packet.
  - `/opsx-apply`: Executes the tasks against the spec.
  - `/opsx-archive`: Merges deltas and archives the change.
  - `/opsx-sync`: Syncs delta specs from a change into main specs without archiving.
  - `/opsx-explore`: Interactive thinking partner for clarifying ideas before drafting specs.
  - `/opsx-update`: Revises existing planning artifacts to reconcile changes.
  - Claude Code uses a colon instead of a dash (`/opsx:propose`, `/opsx:apply`, ...); its skills and commands live in `.claude/`, while `.agents/` holds the Antigravity copies. Refresh both with `openspec update`; to change which tools are generated, run `openspec init --tools claude,antigravity`.

---

## 2. Architectural Invariants (Non-Negotiable)

Every code, grammar, or compiler change must uphold these. The normative text, with scenarios, is [`openspec/specs/system-invariants/spec.md`](openspec/specs/system-invariants/spec.md); keep this list as a one-line summary of it.

1. **Single-file compilation units with explicit export boundaries**: each `.prism` file compiles independently. Default visibility is `private`; `package` is visible within the package directory, `public` across packages. `public` and `package` declarations must declare explicit parameter and return types.
2. **Zero build files, zero marker files**: the workspace root comes from matching the file's path to its `package` declaration. Root-anchored imports use a leading slash (`import /core/fmt`), sibling imports a plain name (`import doo`).
3. **Deterministic hermetic builds and compilation avoidance**: a file's artifact depends only on its content, its direct imports' ABI hashes, the compiler version and the target. Phase 1 extracts ABI hashes, Phase 2 checks bodies; editing a non-generic body causes zero downstream recompilation.
4. **Kotlin-like syntax without ambiguity**: `fun`, `val`/`var`, `struct`, `enum`, `when`, `if` and `T?`, parsed deterministically.
5. **Zero-cost memory safety**: zero GC, zero hidden ARC, no lifetime annotations. Single-owner values, `Heap<T>` for recursion, stack-scoped borrows with elision, typed handles for graphs.
6. **First-class compose and hot reload**: `compose fun` is a native modifier; slot-table handles (`State<T>`, `NodeId`) are `Copy` IDs; the Tier 1 JIT swaps composable bodies while keeping UI state.

In code comments and diagnostics, cite an invariant by name ("the zero-marker invariant"), not by number.

---

## 3. Key Reference Documents ("The Elephant")

What is implemented and what is planned live in different places. Never treat a planned design as a description of the code.

- **Implemented behavior: [`openspec/specs/`](openspec/specs)**, the source of truth for what the compiler does today. Each spec's `## Design Rationale` section records why past decisions were made:
  - [`package-resolution`](openspec/specs/package-resolution/spec.md): Zero-marker root discovery, import resolution, and package scanning.
  - [`syntax`](openspec/specs/syntax/spec.md): Concrete grammar, tokens, and language constructs, string templates (`$name`, `${expression}`, `\$`) and their tokens, and the comment-collecting lexer mode tools use.
  - [`type-system`](openspec/specs/type-system/spec.md): Bidirectional typing, Nothing bottom type, nominal types, infinitely sized types (`E4023`), `Heap(value)` construction and invariant boxes, the built-in string members and `toString()`, the text forms of string template parts (`E4026`), and function values converting only without changing representation.
  - [`compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md): Phase 1 ABI extraction, deterministic ABI hashing, generic signatures and marker parameters, the definitions of reachable types, monomorphization hashes, the generic records instances are built from, and what an instance may depend on.
  - [`ownership`](openspec/specs/ownership/spec.md): `Copy` and `Clone` markers, explicit `.clone()`, whole-value moves (taking a value apart moves it whole; no partial moves), immutable top-level declarations, the borrow checker's use-after-move and uninitialized-read rules, closures never moving their captures, and moving a box's contents out with `unbox()` and consuming receivers.
  - [`patterns`](openspec/specs/patterns/spec.md): `when` patterns and destructuring declarations: constructor patterns, wildcards, `val`/`var` bindings, guards, alternatives, binding modes from the subject's type, patterns through boxes, exhaustiveness with a named missing value, and `val (name, email) = user`.
  - [`loops`](openspec/specs/loops/spec.md): `while`, `do`/`while` and `for` loops, and `break`/`continue`, and how they run.
  - [`references`](openspec/specs/references/spec.md): Implicit borrows at argument sites and of method receivers, copy-out, writing through references and into boxes (`*h = v`), reborrows, the borrowed-nullable view and boxes lending their contents; and the borrow checker's exclusivity rules: non-lexical borrows, conflicts, disjoint fields, closure captures and non-escaping closures; and the four lifetime elision rules: what a call's result keeps borrowed and what a body may return a borrow of; and template parts borrowed until the template is built.
  - [`extension-functions`](openspec/specs/extension-functions/spec.md): Methods as extension functions: declaration, resolution through the file and its imports, implicit `this`, and safe-navigation calls.
  - [`generics`](openspec/specs/generics/spec.md): Type parameters on functions, extensions, structs and enums: `Copy`/`Clone` bounds, move-only unbounded parameters, explicit and inferred type arguments, borrow-free type arguments, invariant generic types and generic extension receivers. Checked once at the declaration and run as instances; `?:` on a type parameter tests for null, and polymorphic recursion and non-regular generic types are rejected (`E4027`, `E6004` across files).
  - [`typed-tree`](openspec/specs/typed-tree/spec.md): The type checker's output: resolved bindings and declaration paths, a type on every node, explicit coercions, moves, copies and borrows, normalized calls, captures, templates and the `prism dump-typed-tree` command.
  - [`mir`](openspec/specs/mir/spec.md): The control-flow graph lowered from each typed file: typed locals, places, storage, drops, temporary scopes, branches, calls, default generators, closures, function names and variant constructors as adapter closures, templates, instances of generic bodies (substituted and flattened, with their closures) and the `prism dump-mir` command with `--instances`.
  - [`code-generation`](openspec/specs/code-generation/spec.md): The Tier 1 Cranelift backend: per-file code units, one code unit and slot per instance of a generic, symbols, calls through the function table, scalar representation, layouts of strings, structs, enums, nullables and boxes, string literals as static data, the runtime allocator, drop, clone and equality glue that checks the stack, templates as one runtime call with one allocation, function values as a descriptor and an environment (closure code with its own slot, borrowing environments in the creating frame, calls through the descriptor's slot, drop entries), passing aggregates by address, what it rejects (`E6001`), `prism dump-clif` (with `--instances`) and `prism dump-layout`.
  - [`execution`](openspec/specs/execution/spec.md): Running a program with `prism run`: the entry point, exit statuses, `print`/`println` output, `Int` and `Double` arithmetic, runtime errors, top-level value initialization, struct, enum, nullable and box values (copies, borrows, equality, `when`, `?.` and `?:`), strings (concatenation, equality, `length`, characters, `toString()`, templates), generic code (drops and clones of type-parameter values, flattened nullable arguments, runtime errors in an instance), closures and function values (captured variables read and written in place, function names and variant constructors as values, function values held in structs, nullables and top-level values), freeing values where their owner's scope ends, cloning, `--check-allocations` and the startup target.
  - [`build-cache`](openspec/specs/build-cache/spec.md): The per-user, content-addressed build cache through which `prism check` skips unchanged files, `--no-cache` and `--explain-cache`.
  - [`formatting`](openspec/specs/formatting/spec.md): `prism fmt`, its canonical layout, comment preservation, templates printed as written, and the meaning and idempotency guarantees.
  - [`diagnostics`](openspec/specs/diagnostics/spec.md): The catalog of every compiler error code. A change that adds an error adds its code here.
  - [`system-invariants`](openspec/specs/system-invariants/spec.md): The 6 non-negotiable invariants. These are design constraints, so they also cover subsystems not built yet.
- **Planned design: [`docs/architecture.md`](docs/architecture.md)**: escaping closures, caching instances, compilation avoidance, hot reload and Tier 2, the rest of the memory model, and the compose engine. A change that implements an item moves its requirements into `openspec/specs/` as delta specs and removes them from this document.
- **Progress: [`ROADMAP.md`](ROADMAP.md)**: current phase and milestones. [`README.md`](README.md) is the high-level vision.
- **History: `openspec/changes/archive/`**: completed change packets. Historical only and never authoritative: their delta specs and task lists are snapshots that may since have been superseded. Do not read them for current behavior. Consult them only to research why a past decision was made.

---

## 4. Current Milestone & Focus
- **Milestone 1**: Minimal Viable Bootstrap Compiler in Rust (`prismc` / `prism` CLI).
- **Milestone 2**: First-class Jetpack Compose UI engine, reactive tree runtime, and native hot-reload framework.
- **Milestone 3**: Self-hosting compiler written in Prism itself.

When writing or reviewing code, always ensure existing tests pass and new language features include executable test fixtures.

Naming: spell out new identifiers in full words (`Statement`, not `Stmt`); don't abbreviate for brevity's sake. Existing abbreviated names (`Expr`, `BlockExpr`, ...) stay until a change deliberately renames them.
