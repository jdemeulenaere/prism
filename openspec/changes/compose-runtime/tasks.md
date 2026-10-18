# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/<scenario-slug>[--<form>]/`, as the header of `fixture_tests.rs` describes. Every `compose` and `execution` fixture of this change is executable (`// run: <status>`, run under `--check-allocations`) and starts with `import /prism/composer`. Scenarios whose WHEN names a Rust test or a benchmark are tested where their task says.

Prerequisites (design, "Baseline"): groups 1 to 6 need only `closures`' function value layout, and their tests build function values by hand. Groups 7 to 9 need `closures`, `escaping-closures` and `standard-library` archived. Before group 7, compare the MODIFIED blocks with the main specs as those changes left them, and reconcile any drift: `execution` "Runtime Errors End the Program" (from `hot-reload`) and `standard-library` "The Compiler Ships the Library". Also re-read `standard-library`'s `externals.rs` and `library_tests.rs` as implemented, since tasks 7.1 and 7.3 extend them. (Done at apply time: see design.md, "Reconciliation at apply time".)

## 1. Runtime support in `prism-runtime` (D2, D12, D13)

- [ ] 1.1 Add `crates/prism-runtime/src/function_value.rs`:
  - `#[repr(C)] FunctionValue { descriptor: *const Descriptor, environment: *mut u8 }` and `#[repr(C)] Descriptor { slot: *const AtomicUsize, drop_environment: Option<unsafe extern "C" fn(*mut u8)> }`, the layout of `closures` D2 and D3;
  - `unsafe fn call_with_word(&self, word: u64)`, which loads the slot with acquire ordering and calls the code as `extern "C" fn(*mut u8, u64)`;
  - `unsafe fn release(self)`, which calls the drop entry when it is not null;
  - `unsafe fn read_from(address: *const u8) -> FunctionValue`, for a value passed by address.

  Export them from `lib.rs`. Verify with unit tests that call a Rust `extern "C"` function through a hand-built descriptor and slot, see a repointed slot reach the new function, and see `release` call the drop entry once and skip a null one.
- [ ] 1.2 In `panic.rs`, add `PanicKind::CompositionMisuse` with the next free code, its message `composition misuse: <detail>`, and its entry in `PanicKind::ALL`. Verify with the existing `panic_message` tests extended to the new kind.
- [ ] 1.3 In `allocation.rs`, add `set_finish_report(report: fn() -> Option<String>)`, stored in a static. Make `finish_tracking` call it after dropping top-level values and before the leak report, and join a returned line before the leak line, so that either line makes `finish` return `ALLOCATION_CHECK_EXIT_STATUS`. Do nothing when checking is off. Verify with a unit test that installs a report returning a line and checks the combined report and the status.

## 2. The `prism-compose` crate: handles, thread state, entry table (D2, D3, D4)

- [ ] 2.1 Create `crates/prism-compose` (`Cargo.toml` depending on `prism-runtime` only) and add it to the workspace members. Write `src/lib.rs` with a crate comment summarizing the design's D1 to D4, citing the first-class compose and hot reload invariant by name. Verify with `cargo build -p prism-compose`.
- [ ] 2.2 Add `src/handle.rs`: `Handle(u64)` with `index()` and `generation()`, and `Arena<T>` with `insert`, `get`, `get_mut`, `remove` (bumping the generation) and a free list that retires an index whose generation reaches `u32::MAX`. Verify with unit tests: a removed handle is stale, a reused index has a new generation, and a retired index is never handed out again (test it with a generation preset near the maximum).
- [ ] 2.3 Add `src/thread.rs`: a `thread_local!` holding the composition arena, the group arena, the pass counter (from 1) and the active pass `Option<ActivePass { identifier, composition }>`, behind `with_state(|state| ...)`. Document the rule of D4 there: no reference into the state may be live while program code runs. Add `live_compositions_report() -> Option<String>`, giving `allocation check: compositions still live at exit: <n>` when n > 0. Verify with unit tests on two threads showing independent states.
- [ ] 2.4 Add `src/functions.rs` with `COMPOSE_FUNCTIONS: &[prism_runtime::RuntimeFunction]` and `compose_function(name)`. Add the test of `prism-runtime`'s `functions.rs`, reading `entry.rs` for `pub extern "C" fn` and `pub unsafe extern "C" fn` lines, so the table and the exported functions stay equal, and a test that reads `Cargo.toml` and finds no dependency but `prism-runtime`. Verify with the scenario "Every entry point is listed once" once `entry.rs` exists (task 6.1).

## 3. Groups, matching and slots (D5, D6)

- [ ] 3.1 Add `src/group.rs` with `GroupKind::{Restart, Replaceable, Movable, Node}` and the group record of D5, and `src/slot.rs` with `SlotType` (`#[repr(C)]`, D6), `Slot` (inline storage for values of at most 16 bytes and alignment 8, a `std::alloc` block otherwise), `WORD_SLOT_TYPE` and word slots with their tag. Matching compares record pointers, then fingerprints. Verify with unit tests: an inline and a boxed slot round-trip their bytes, and two records with equal fingerprints match.
- [ ] 3.2 Add `src/composer.rs` with the open-group stack and the operations `start_group(kind, key, data_key)` and `end_group(expected_kind)`. Use the reused scratch children vector and the lazily built `HashMap<(kind, key, data_key), VecDeque<usize>>` of D5. On a group's end, copy the new children, renumber `index_in_parent`, and push the unvisited old children to the release list as removed. Verify with Rust tests over a composition driven by Rust content: in-order reuse, the first unvisited match of equal keys, a reversal of 1,000 movable groups keeping every group, and removal of unvisited children. Also verify the misuse cases of "Groups Are Balanced" (`end of a group that was not started`, `group ended with the wrong kind`, `group left open`) through a test-only `Result` form of the operations, which the entry points turn into runtime errors.
- [ ] 3.3 In `composer.rs`, add the slot cursor and the operations `slot(type) -> *const u8`, `set_slot(type, value)`, `changed(type, value) -> bool`, and the word forms `has_word(tag)`, `word()`, `set_word(tag, value)` and `changed_word(tag, value)`. Forget the slots after the cursor at a group's end, and forget a mismatched slot when writing over it, both onto the release list. Verify with Rust tests for the scenario "A typed slot read with another slot type", a null equality entry comparing as changed, the new value being released when equal, and `word()` at no slot returning the `no slot to read` misuse.

## 4. Compositions, passes, releases, restart scopes (D4, D7, D8, D11)

- [ ] 4.1 Add `src/composition.rs` with `Composition::start(applier: Box<dyn Applier>, content: FunctionValue) -> Handle`, `set_content`, `recompose -> bool`, `dispose` and `root_scope`. Make every one of them check that no pass is active (`composition operation during a pass`). A pass sets the active pass, recomposes the queue, clears the active pass, and then releases. Verify with Rust tests for "Starting a Composition Composes It Once", "Recomposing Runs a Pass Only When a Scope Is Invalid" and "Setting the Content Recomposes the Root", using content function values built by `test_support.rs` (task 4.4).
- [ ] 4.2 Implement releases (D7): one release list per composition, an iterative post-order walk for removed subtrees (children last first, slots last first, restart function, node removal and release), groups returned to the arena with a new generation, and nothing held across calls of drop entries. Verify with the Rust scenarios "A removed group's slots are released after the pass" and "Release order of a removed subtree", using recording drop entries, plus a test removing a 100,000-deep chain of groups without overflowing the stack.
- [ ] 4.3 Implement restart scopes and invalidation (D8):
  - `end_restart_group(restart)`, which moves the restart function in and pushes the old one to the release list;
  - `current_scope()`, `skipping()` and `skip_to_group_end()`;
  - `invalidate(scope)`: O(1), ignoring stale handles, into the next pass's queue when a pass is active;
  - the restart frame with its one-group window;
  - the pass's ordering by `index_in_parent` paths in a reused buffer;
  - skipping scopes that were removed or are no longer invalid;
  - `Composition::invalidate_where(predicate on the restart descriptor's slot)` and the forced pass flag for `compose-hot-reload` (D15).

  Verify with Rust tests for "A restart function that starts another key", "A restart function reaches its closure's current code", tree order across unrelated scopes, a parent and child both invalid recomposing once each, and a removed scope dropped from the queue.
- [ ] 4.4 Add `src/test_support.rs` (public, documented as for tests): content and restart functions built from Rust closures (a boxed environment and a trampoline as the code, a drop entry freeing the box), function values with static descriptors and inline environments that allocate nothing, recording drop entries, and slot type records over Rust types. Use it in the tests of groups 3 to 5.

## 5. Node groups, the applier and dumps (D9, D14)

- [ ] 5.1 Add `src/applier.rs` with the `Applier` trait of D9 and `NodeTree` (an arena of linked nodes, creation numbers from 1, never reused). Verify with unit tests of `place` (insert, move, unchanged), `remove` and `release`, and with numbers not reused after a release.
- [ ] 5.2 In `composer.rs`, add node groups: `start_node_group(key, kind) -> NodeHandle`, the per-node-group "last placed" marks, placement during composition, the restart frame's previous node (D8), a new node for a kind change, and node removal and release at the end of the pass. Verify with a recording applier for the scenario "Removed node groups remove their nodes after the pass", a keyed reorder placing each moved node once, a kind change creating a new node, and a restarted scope placing its node after the previous scope's last node.
- [ ] 5.3 Add `src/dump.rs` with `Composition::dump() -> String` in the format of "Dumping a Composition", followed by the applier's `dump`. Make `NodeTree::dump` follow "Dumping the Node Tree". Verify with unit tests of both formats, including an invalid scope's ` invalid` suffix and a typed slot's `slot <type name>` line.

## 6. Entry points, misuse errors and measurements (D2, D3, D12, D16)

- [ ] 6.1 Add `src/entry.rs` with every `#[no_mangle] pub extern "C" fn prism_compose_*` of D6, D10 and D11, each validating its handles and turning a misuse into `panic_without_location(PanicKind::CompositionMisuse, detail)`:
  - the protocol functions: `start(content: *mut FunctionValue) -> u64`, `set_content`, `recompose -> u8`, `dispose`, `root_scope`, `dump(destination: *mut PrismString, composition)`, `start_group`, `start_movable_group`, `start_restart_group`, `start_node_group -> u64`, `end_group`, `end_restart_group(composer, restart: *mut FunctionValue)`, `skipping -> u8`, `skip_to_group_end`, `current_scope`, `invalidate`, `has_word -> u8`, `word -> i64`, `set_word` and `changed_word -> u8`;
  - the typed slot functions `slot`, `set_slot` and `changed`.

  A composer is the active pass's identifier (D3). Function values passed by address are read and owned (`FunctionValue::read_from`). List every one in `COMPOSE_FUNCTIONS` with its `RuntimeType`s (`I64` for words, `I8` for `Bool`, `Pointer` for addresses). Verify with the task 2.4 tests, and with Rust tests calling the entry points through their C signatures. A child-process test (spawning the test binary with an environment variable, as `prism-runtime`'s exit tests do) checks the scenario "Reading a slot that is not there" and its status 101.
- [ ] 6.2 Add `crates/prism-compose/tests/allocations.rs` with a counting `#[global_allocator]` and the scenario "A steady-state pass allocates nothing", with restart functions from `test_support.rs` that allocate nothing. Verify with `cargo test -p prism-compose --test allocations`.
- [ ] 6.3 Add `crates/prism-compose/tests/runtime_timing.rs`, `#[ignore]`d, measuring as D16 describes and asserting the bounds of "Recomposing one scope does not depend on the composition's size" under `PRISM_ASSERT_TIMING=1`. Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture` and record the medians in design.md, D16.

## 7. Backend and library (D2, D10, D13)

- [ ] 7.1 Make `crates/prism-codegen` depend on `prism-compose`. Extend `externals::entry_point(symbol) -> Option<&'static RuntimeFunction>` to search `RUNTIME_FUNCTIONS` then `COMPOSE_FUNCTIONS`, and use it in `UnitContext::runtime_function` too (the external check, `externals::check_externals`, and the lowering of external calls already use it). Make `link.rs::Linker::new` register both tables' symbols. Verify with a `lowering_tests.rs` test over an injected library module for "A compose entry point declared with the wrong signature" (`E6005`), and a link test that every compose entry point resolves.
- [ ] 7.2 In `program.rs::Program::run`, call `prism_runtime::set_finish_report(prism_compose::live_compositions_report)` before `prism_runtime::start`. Verify with the `execution` fixture of task 8.4.
- [ ] 7.3 Add `crates/prism-library/prism/composer.prism` (`package prism`): a comment saying it is the composer protocol that composables are lowered to, and not an application API; one `private external fun prism_compose_*` per protocol entry point, with `escaping` on `content` and `restart`; and the public wrappers of "Composition Functions of the Protocol", "Group Functions of the Protocol", "Scope Functions of the Protocol" and "Slot Functions of the Protocol", each with a comment. Write `rememberInt` as `if (prism_compose_has_word(composer, typeTag)) prism_compose_word(composer) else { val value = calc(); prism_compose_set_word(composer, typeTag, value); value }` (or the equivalent statement form). Add it to `MODULES`, not to `PRELUDE`. Verify with `library_tests.rs`, which now checks, formats and compiles it, for the scenario "The protocol module's externals match the entry points", and with `cargo run -q -p prism-cli -- fmt --check crates/prism-library/prism`.
- [ ] 7.4 Add a CLI test in `cli_workspace_tests.rs` for "The composer module is not in the prelude": `prism check --explain-cache` on a file without imports names no `prism/composer.prism`, and names it after `import /prism/composer`.

## 8. Executable fixtures

- [ ] 8.1 Add `compose` fixtures, one executable case per scenario, with `stdout.snap` and, for errors, `stderr.snap`. Groups and slots:
  - "Starting a composition composes its content once";
  - "A remembered value survives recomposition";
  - "New content recomposes the root";
  - "Group kinds in a dump";
  - "Reordered movable groups keep their slots";
  - "Equal keys match in order";
  - "Branches get their own groups";
  - "Slots past the cursor are forgotten";
  - "A slot of another type is recreated";
  - "A calculation runs only for a new slot";
  - "Comparing a value stores it";
  - "Two tags at two positions".

  Read each snapshot against its scenario's THEN. Verify with `cargo test -p prism-cli --test fixture_tests`.
- [ ] 8.2 Add the `compose` fixtures of scopes and nodes:
  - "An invalidated scope recomposes alone";
  - "The root is the current scope outside restart groups";
  - "Equal arguments skip a restart scope";
  - "Skipping is false on the first pass";
  - "Invalidating a stale scope does nothing";
  - "Parents recompose before their children";
  - "A scope removed by its parent is not recomposed";
  - "The root scope of a composition";
  - "Moved node groups move their nodes";
  - "Node numbers are not reused";
  - "A node group returns its node";
  - "Nested nodes in a dump";
  - "A dump of groups, slots and nodes";
  - "Disposing a composition frees what it holds".

  Verify as in 8.1.
- [ ] 8.3 Add the `compose` fixtures of misuse, each with `// run: 101` and an exact `stderr.snap`: "A composer used after its pass", "Disposing a composition twice", "Ending a group that was not started", "A group left open" and "A composition operation during a pass". Add both forms of "The protocol module is imported": `--without-import` (`// expect: E4001`) and `--with-import` (`// run: 0`). Verify as in 8.1.
- [ ] 8.4 Add the `execution` fixtures "A composition that is never disposed" (`// run: 102`, with the exact `stderr.snap`) and "A composition misuse has no location" (`// run: 101`). Verify as in 8.1.

## 9. Integration

- [ ] 9.1 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings) and `cargo test --workspace`. Run `cargo run -q -p prism-cli -- check` on every file under `examples/`, `cargo run -q -p prism-cli -- fmt --check examples/` and `cargo run -q -p prism-cli -- run --check-allocations examples/run/main.prism`, which must behave as before. No compiler stage is added, so `compile_timing.rs` gets no new shape (design D16).
- [ ] 9.2 Re-run the startup benchmark of `crates/prism-cli/tests/run_timing.rs`, cold and warm, to check that registering the compose table and installing the finish report leave startup under its 15 ms target. Record the medians in design.md, D16.

## Workflow follow-up

- At archive, remove from `docs/architecture.md` §5 what this change implements:
  - from "Compose Runtime Crates Behind C Entry Points", the `prism-compose` half;
  - "Slot Table of Groups and Owned Slots", except its scenario about a state read earlier in the pass, which moves to the `state-handles` plan;
  - from "Restart Scopes and Invalidation", the tree order, the at-most-once rule and dropping removed scopes;
  - from "Compositions Are Owned Values", the runtime lifecycle, leaving the owned Prism `Composition` to `compose-transform`;
  - from "UI Node Tree Owned by the Runtime", the sentence on node insertions, moves and removals;
  - from "Groups Keyed by Declaration Path and Lexical Position", the sentence on type fingerprints.

  In "Composable Calling Convention" and D4, replace "the address of the current composer" with "the current composer word" (D3).
- At archive, promote D1 to D10 and D15 into a "Design Rationale" section of the new `compose` spec, and add to `ROADMAP.md` the follow-ups: deciding whether `/prism/composer`'s wrappers stay public (Open Questions, for `compose-transform`), nested passes for subcompositions (D4), counting live groups and nodes in the allocation check (D13, for `reactive-app-verification`), and a size-class allocator for slots larger than 16 bytes if measurements ask for it (D6).
