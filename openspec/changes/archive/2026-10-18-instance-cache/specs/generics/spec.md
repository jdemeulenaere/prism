# Generics Specification Delta

## RENAMED Requirements

- FROM: `### Requirement: Polymorphic Recursion Across Files Is Rejected When Compiled`
- TO: `### Requirement: Polymorphic Recursion Across Files Is Rejected`

## MODIFIED Requirements

### Requirement: Polymorphic Recursion Across Files Is Rejected
A cycle of generic function calls with a growing type argument that passes through generics of several files, which only files importing each other can form, MUST be reported as `PolymorphicRecursionAcrossFiles` (`E6004`) by `prism check` and `prism run` of any program holding the files of the cycle, whether or not anything instantiates them and whether or not their verdicts are reused, naming each generic on the cycle. The program MUST NOT run.

#### Scenario: Polymorphic recursion through two files
- **WHEN** `a/x.prism` imports `/a/y` and declares `public struct Box<T>(val v: T)` and `public fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)`, `a/y.prism` imports `/a/x` and declares `public fun <T> pong(x: T, n: Int): Int = ping(x, n)`, and `a/main.prism` imports `/a/x` and declares `fun main() { println(ping(1, 3)) }`
- **THEN** `prism check a/main.prism` and `prism run a/main.prism` both report `E6004` at `pong(Box(x), n - 1)`, naming `a/x::ping` and `a/y::pong`, and exit with status 1, and the run runs nothing

#### Scenario: Polymorphic recursion found from reused verdicts
- **WHEN** `prism check a/x.prism` runs twice with the cache enabled on the files `a/x.prism` and `a/y.prism` of that scenario
- **THEN** both runs report the same `E6004` and exit with status 1, and the second writes no entry, since each file alone passes Phase 2 and both verdicts are reused
