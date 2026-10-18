# MIR Delta

## ADDED Requirements

### Requirement: Composable Bodies Mark Their Regions
The lowered MIR of a composable function or closure MUST hold a group start marker where each region of compose, "Conditional and Repeated Composable Code Runs in Its Own Groups", begins, and a group end marker on every path that leaves it, falling through or by `return`, `break` or `continue`. Each marker MUST name its region, its parent region and its construct. Markers MUST NOT read, write, move or drop anything.

#### Scenario: A branch with a composable call is marked
- **WHEN** `compose fun A() {}`, `fun log() {}` and `compose fun V(on: Bool) { if (on) { A() }; if (on) { log() } }` are lowered
- **THEN** `prism dump-mir` shows a group start at the beginning of the first `if`'s branch and a group end where it joins, and no marker for the second `if`

#### Scenario: A return leaves a loop's region
- **WHEN** `compose fun A() {}` and `compose fun F(n: Int) { while (true) { A(); if (n > 0) { return } } }` are lowered
- **THEN** the loop's region starts before its first iteration, and the path of the `return` ends it before returning

### Requirement: Composed MIR
After borrow checking, the MIR of every file MUST be composed. A composable body MUST take the composer and its change words as `Int` arguments after its declared ones, start its body group first and end it before every return, and every composable call MUST pass the caller's composer and change words for the callee. Composed MIR MUST hold no region marker and MUST be well formed.

#### Scenario: Composer passed to composable calls
- **WHEN** `compose fun Label(n: Int) {}` and `compose fun Screen() { Label(1) }` are compiled
- **THEN** `prism dump-mir --composed` shows the call of `Label` passing `Screen`'s composer and a change word marking its argument static

#### Scenario: A composable lambda takes the composer
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Screen() { Column { } }` are composed
- **THEN** the body `main::Screen#closure0` takes a composer and a change word, and `Column`'s call of `content` passes them

### Requirement: Skippable Bodies Keep, Compare and Skip
A skippable body MUST keep each argument in a slot of its restart group, comparing it there when its change bits are uncertain, test whether it may skip, and on skipping drop the arguments it owns and go to its group's end. It MUST end its restart group with the value of its restart closure, `<symbol>#restart`, which captures nothing, reads the kept arguments and calls the composable with the force bit.

#### Scenario: A skippable body and its restart closure
- **WHEN** `compose fun Badge(n: Int) { println(n) }` is composed
- **THEN** `prism dump-mir --composed` shows `main::Badge` compare its argument with its kept slot of type `Int`, a skip path that ends its restart group, and the body `main::Badge#restart`, without captures, which reads kept slot 0 as an `Int` and calls `main::Badge` with it and the force bit

### Requirement: Root Content Adapters
A body that calls `composition` or `setContent` MUST have one adapter closure, `<symbol>#root-content`, owning the content function value and calling it, when the runtime calls the adapter with a composer, with that composer and the force bit. Each such call MUST build its adapter from the content it is given.

#### Scenario: The adapter of a composition
- **WHEN** `compose fun Hello() {}` and `fun main() { val c = composition { Hello() } }` are composed
- **THEN** `prism dump-mir --composed` shows `main` building the closure value `main::main#root-content` from the content lambda, moved, and starting a composition with it

### Requirement: Composed MIR Dump
`prism dump-mir --composed <file.prism>` MUST print the composed MIR of the file as deterministic text, its composable bodies marked composed and the skippable ones marked skippable, each group key as 16 hexadecimal digits followed by its construct. With errors of any stage up to borrow checking it MUST report them and print no MIR.

#### Scenario: Dumping composed MIR
- **WHEN** `prism dump-mir --composed main.prism` runs twice on a file declaring `compose fun Label(n: Int) {}`
- **THEN** it prints the same text both times, with `main::Label` marked composed and skippable, and exits with status 0

### Requirement: Generic Composables Are Composed Once
A generic composable MUST be composed once, in its polymorphic MIR, and its record MUST hold the composed bodies, its restart closure included. Its instances MUST substitute the composed MIR, with the slot types of its kept and remembered values substituted too.

#### Scenario: Instances of a generic composable
- **WHEN** `compose fun <T : Copy> Show(x: T) { }` is declared and `fun main() { val c = composition { Show(1); Show(true) } }` runs through `prism dump-mir --instances main.prism`
- **THEN** it prints `main::Show<Bool>` and `main::Show<Int>`, each keeping its argument in a slot of its own type argument, each with its restart closure
