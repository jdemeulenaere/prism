# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Phase 3 Generates Each File's Code From Its Elaborated MIR
Code generation MUST run on each file only after the file passes Phase 2, and MUST generate the file's code from its elaborated MIR alone, never from another file's bodies or MIR, so that a file's code depends only on the file, the ABIs of its direct imports, the compiler and the target.

#### Scenario: Code generation independent of dependency bodies
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(2)) }`, and the body of `twice` is then changed to `n + n`
- **THEN** `prism dump-clif app/main.prism` prints the same text before and after the change, and `prism run app/main.prism` prints `4` both times

#### Scenario: A dependency with errors stops the run
- **WHEN** `lib/text.prism` declares `public fun shout(n: Int): Int = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun main() { println(shout(1)) }`
- **THEN** `prism dump-clif app/main.prism` prints the IR of `app/main`, while `prism run app/main.prism` reports the `E4001` in `lib/text`, runs nothing and exits with status 1
