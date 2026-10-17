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
