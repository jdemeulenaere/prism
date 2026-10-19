# Agent Workflow Playbook

Lessons from the autonomous run that took Prism from the borrow checker to Milestone 2 (2026-10-17 to 2026-10-19, about 500 commits, 40 changes). Read this before orchestrating several changes, or before running unattended. [`AGENTS.md`](../AGENTS.md) holds the rules; this document holds how to apply them at scale and the pitfalls already paid for.

---

## 1. The change loop

Each OpenSpec change ran as three separate agents, each starting from the files rather than from the previous agent's context:

1. **Propose** (`/opsx:propose`): proposal, design, tasks and delta specs, committed as "Propose <change>". The proposer studies the code closely enough that `tasks.md` names real files and functions.
2. **Implement** (`/opsx:apply`): one commit per task group ("Implement <change>: <group>"), each passing `cargo test --workspace`, so an interrupted agent loses at most one group.
3. **Review and archive**: a fresh agent hunts for bugs against the delta specs with small `.prism` probes, fixes what it confirms ("Fix <what> in <change>", each with a regression fixture), then archives ("Archive <change>").

**The fresh reviewer is not optional.** It found real defects in nearly every change, including memory-safety bugs (double drops, a use after free through a guard, segfaults from a borrowed temporary converted after being lent), unsound borrow-checker acceptances, wrong memoization in the compose transform and stale cache hits. Implementers' own tests rarely caught these. Brief the reviewer with the change's risk areas and ask for adversarial probes, not a reread of the diff.

**Plain fixes** (AGENTS.md §1) handled every bug where the spec already implied the answer, often found by a reviewer of an unrelated change. Give each its own commit and regression fixture.

## 2. Briefing subagents

Subagents start with no context. Every brief should contain:

- AGENTS.md first, then the relevant `.claude/skills/openspec-*/SKILL.md`. Say explicitly that the orchestrator authorises the step, because the skills tell the agent to stop and ask the user.
- The design principles in AGENTS.md §5, compile speed above all.
- How to record decisions: mark them **(autonomous)** in `design.md` with the rejected alternatives, and add a row to [`docs/decisions-for-review.md`](decisions-for-review.md) with a revert cost. Escalate only a decision that would be almost impossible to revert.
- Which files other running agents own, so this agent stages explicit paths only (§4).
- A short final report: commits, test status, decisions (one line each), and what is left or risky.

Keep the brief in the repository or in the prompt itself. A brief kept in `/tmp` was lost when the app restarted mid-run.

## 3. Proposing ahead without drifting

Proposing two or three changes ahead of implementation kept agents busy, but every packet drifted as earlier changes archived: requirements were renamed or split, scenarios were added, codes were taken. Therefore:

- **Reconcile before applying.** The first task of every apply is to re-read the main specs and the landed code, re-base each MODIFIED block on the current main text (a MODIFIED block must keep every scenario of the main requirement), turn ADDED blocks for requirements that now exist into MODIFIED, check every function the tasks name, and get `openspec validate <change> --strict` clean with no INFO lines. Record the reconciliation in `design.md`.
- When an implementation changes a name a later packet relies on, add a short "Baseline update" note to that packet's `design.md` instead of letting the next agent discover it.
- `openspec validate --strict` rejects requirement texts over 500 characters. Split a requirement that states two rules into two, moving scenarios without renaming them, because fixtures are named after scenarios.
- A later change may correct an earlier one's design (for example, compose-hot-reload D8 corrected compose-transform D22). Route the correction to the earlier packet's implementer if it has not landed yet.

## 4. Several agents in one checkout

Up to three agents worked on `main` at once (a proposer next to an implementer, or two implementers on disjoint crates). What made that safe:

- **Stage explicit paths only.** Never `git add -A` or `git add .`. Commit only your own hunks of a shared file.
- **Edit shared files with small targeted edits.** `docs/decisions-for-review.md`, `ROADMAP.md`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock`. Never rewrite them from an earlier read, and never `git checkout`, `git restore` or `git stash` a file that may hold another agent's uncommitted edits.
- Run two implementers in parallel only when their crates are disjoint. Building a new crate alongside a front-end change worked; two type-checker changes did not.
- A red `cargo test --workspace` may be another agent's half-finished edit. Test your own crates, wait and retry, and don't "fix" files you don't own.
- A worktree (`isolation: worktree`) gives full isolation, but it costs a full `target/` per worktree and a merge afterwards. On this machine disk space made it a last resort.

## 5. Measurement and timing

- **Measure on an idle machine.** Concurrent `cargo` builds inflated a warm-start measurement from 1.5 ms to 6.3 ms and nearly triggered a needless fix. Rerun before believing a regression.
- Timing tests are ignored benchmarks that assert only under `PRISM_ASSERT_TIMING=1`, take the best of several runs, and assert growth ratios (×4.6 per ×4 for near-linear stages) rather than absolute times. `crates/prism-cli/tests/compile_timing.rs` covers every compiler stage. Add shapes there for any new stage or analysis (openspec/config.yaml).
- Super-linear compile time hid in places no one expected: scoped name lookup, symbol building per closure, the borrow checker's loan flow (cubic with handlers kept to the end of a body), inline structural equality, Cranelift's egraph and register allocation on deep dominator trees, and directory re-reads in the resolver. Generated inputs at 256, 1024 and 4096 found each of them.

## 6. The environment

- **`target/` grows without bound.** `target/debug/deps` reached about 600,000 files and tens of GiB of stale `*.rcgu.o` objects. Besides filling the disk, this made macOS's app check-in list the folder, so a debug test binary took 12–15 s to create its window event loop. Clean stale artifacts regularly (`cargo clean -p <crate>`, or delete `deps` entries older than a few hours), and don't create extra `CARGO_TARGET_DIR`s or scratch Cargo projects.
- **Test in real windows when it adds coverage, deliberately.** Headless runs (`--headless --events`) cover composition, layout, input semantics and draw lists. Only a real window covers:
  - the launch path (`prism run` → the main-thread window server → winit/AppKit), where a run that returned at once with status 0 was seen and never explained;
  - the window's Inter metrics and HiDPI scaling;
  - rasterized pixels (`--screenshot`);
  - platform focus, occlusion, resize and close events;
  - input-to-present timing.

  `prism run` without `--headless` opens a window on the user's real screen, so if the user is around, say so in one line first. Batch the runs, keep them short, close every window you open, and prefer the opt-in tests (`PRISM_WINDOW_TESTS=1 cargo test -p prism-cli --test window_input`, `--test app_windows`) over ad-hoc launches. Unannounced relaunches, eight in two minutes on 2026-10-19, look to the user like a misbehaving app.
- **Real OS input needs a permission.** The window tests feed input through the window's own event handling, which covers the mapping code but not the OS's event delivery. Posting real mouse and keyboard events (CGEventPost) needs the Accessibility permission for the process running the tests. That is the user's security setting to grant: ask, never work around it.
- **The display may be locked** during unattended runs. The window tests above still run, and checks that need a person and real hardware go to the user under a "Needs the user" item in `ROADMAP.md`.
- **Watch tests spawn `prism run --watch`.** Their helpers must kill and reap the child on drop, or a failed test leaves the program spinning forever.
- **Rate limits and crashes** interrupted agents several times. A per-group commit plus resuming the same agent (SendMessage with its id) lost no work. A long silence from a fresh agent was usually a slow start, not a hang: ask for a status line before restarting it.

## 7. Working with the user

- Log every decision taken on the user's behalf in [`docs/decisions-for-review.md`](decisions-for-review.md), with a revert cost, and keep its top summary current. The user reviews it at the end of the milestone.
- User directions given mid-run (compile speed first; Compose's APIs and module split; no `Modifier.composed`) became design principles (AGENTS.md §5). Save new ones there too, not only in a decisions row.
- Relay a user direction to running agents verbatim, and have them record it as "(user direction <date>)".
