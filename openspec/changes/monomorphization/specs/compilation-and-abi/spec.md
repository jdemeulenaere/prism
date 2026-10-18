# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Generic Records Carry What Instantiation Needs
For each generic function and extension of a file that passes Phase 2, private ones included, the compiler MUST keep a generic record: the elaborated polymorphic MIR of its body, its default value generators and its closures, and the definitions of the structs and enums those bodies mention. A record MUST NOT hold the body of any other declaration: the non-generic functions a generic body calls are called by symbol, and the generics it calls are instances of their own records.

#### Scenario: A generic body calls private helpers
- **WHEN** `lib/box.prism` declares `fun helper(n: Int): Int = n + 1`, `fun <T> inner(x: T): T = x` and `public fun <T> outer(x: T, n: Int): Int { val y = inner(x); return helper(n) }`, and `app/main.prism` imports `/lib/box` and declares `fun main() { println(outer("a", 1)) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `2`, the instance `lib/box::outer<String>` calling `lib/box::helper` through its slot and the instance `lib/box::inner<String>` through its own

### Requirement: Instances Depend Only on Their Record and Type Arguments
Phase 3 MUST build each instance from its generic's record and the definitions of the structs and enums its type arguments mention, never from another body of the generic's file or of the calling file, so that an instance is the same whichever file requests it first, and a file's own code never depends on a generic body.

#### Scenario: An instance of a generic from a module the caller does not import
- **WHEN** `lib/util.prism` declares `public struct Pair<A, B>(val a: A, val b: B)` and `public fun <T> twin(x: T): Pair<T, T> = Pair(x, x)`, `lib/box.prism` imports `/lib/util` and declares `public fun <T : Copy> first(x: T): T = twin(x).a`, and `app/main.prism` imports only `/lib/box` and declares `fun main() { println(first(7)) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `7`, compiling the instances `lib/box::first<Int>` and `lib/util::twin<Int>`

## MODIFIED Requirements

### Requirement: Phase 3 Generates Each File's Code From Its Elaborated MIR
Code generation MUST run on each file only after the file passes Phase 2, and MUST generate the file's code from its elaborated MIR alone, never from another file's bodies or MIR, so that a file's code depends only on the file, the ABIs of its direct imports, the compiler and the target. A file's code MUST NOT contain any instance of a generic, its own generics' included: it calls instances by symbol.

#### Scenario: Code generation independent of dependency bodies
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(2)) }`, and the body of `twice` is then changed to `n + n`
- **THEN** `prism dump-clif app/main.prism` prints the same text before and after the change, and `prism run app/main.prism` prints `4` both times

#### Scenario: A dependency with errors stops the run
- **WHEN** `lib/text.prism` declares `public fun shout(n: Int): Int = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun main() { println(shout(1)) }`
- **THEN** `prism dump-clif app/main.prism` prints the IR of `app/main`, while `prism run app/main.prism` reports the `E4001` in `lib/text`, runs nothing and exits with status 1

#### Scenario: A generic body change leaves its callers' code unchanged
- **WHEN** `lib/box.prism` declares `public fun <T> pick(a: T, b: T): T = a`, `app/main.prism` imports `/lib/box` and declares `fun main() { println(pick(1, 2)) }`, and the body of `pick` is then changed to `b`
- **THEN** `prism dump-clif app/main.prism` prints the same text before and after the change, while `prism run app/main.prism` prints `1` before it and `2` after it
