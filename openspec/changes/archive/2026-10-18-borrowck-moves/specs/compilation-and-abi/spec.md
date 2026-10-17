# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Phase 2 Borrow-Checks Each File's MIR
Phase 2 MUST borrow-check the MIR of every file that typechecks from that MIR alone, never another file's bodies, MIR or ABI, reporting `E5xxx` diagnostics, and MUST elaborate the drops of each file that passes. `prism check` MUST fail on a file with borrow-checker errors as it does on one with type errors.

#### Scenario: Borrow checking on MIR
- **WHEN** a file's MIR has been built
- **THEN** the borrow checker checks every body of it for uses of moved and uninitialized values, reporting `E5xxx` diagnostics; when it reports none, the file's drops are elaborated, and the elaborated MIR is what later stages consume

#### Scenario: Borrow checking independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: String): String { val t = s; return s }`, which uses `s` after moving it, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-mir --elaborated app/main.prism` prints the elaborated MIR of `app/main`, while `prism check app/main.prism` reports the `E5001` in `lib/text`
