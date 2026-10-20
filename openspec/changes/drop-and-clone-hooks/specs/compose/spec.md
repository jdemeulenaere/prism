## ADDED Requirements

### Requirement: Hooks of Values a Composition Releases Run After the Pass
A remembered or kept value with a drop hook that a pass releases MUST run its hook when the pass releases it, after the pass has ended, in the release order. A state's value with a drop hook MUST run it when the state's cell is freed, not when the state is retired. Such hooks run outside composition, as handlers do: a state they write invalidates its readers for the next pass.

#### Scenario: A remembered value's hook runs when its group leaves
- **WHEN** `struct Noisy { val name: String }`, `operator mut fun Noisy.drop() { println("drop " + name) }`, `val Visible: MutableState<Bool> = mutableStateOf(true)` and `compose fun Kept() { val n = remember { Noisy("kept") }; println("composed " + n.name) }` are declared, and `fun main() { val c = composition { if (Visible.value) { Kept() } }; Visible.value = false; c.recompose(); println("done") }` runs with `--check-allocations`
- **THEN** it prints `composed kept`, `drop kept` and `done`, and exits with status 0

#### Scenario: A state's value runs its hook when its cell is freed
- **WHEN** with that `Noisy` and `Visible`, `compose fun Held() { val s = remember { mutableStateOf(Noisy("held")) }; println("composed") }` and `fun dispose(c: Composition) {}` are declared, and `fun main() { val c = composition { if (Visible.value) { Held() } }; Visible.value = false; c.recompose(); println("removed"); dispose(c); println("done") }` runs with `--check-allocations`
- **THEN** it prints `composed`, `removed`, `drop held` and `done`: the state retired by the pass is freed, and its value dropped, when the last composition is disposed

### Requirement: Hooks Run During a Pass Follow the Pass's Rules
A hook that runs while a pass runs, for a value dropped in a composable body or in a function it calls, MUST obey the rules of the pass: writing a state there MUST end the program with `state written during composition`, located at the write in the hook.

#### Scenario: A hook writing state during composition
- **WHEN** `val Count: MutableState<Int> = mutableStateOf(0)`, `struct Bump { val n: Int }`, `operator mut fun Bump.drop() { Count.value += n }` and `compose fun Content() { val b = Bump(1) }` are declared, and `fun main() { val c = composition { Content() } }` runs
- **THEN** stderr is `panic: state written during composition` and the location of `Count.value += n` in the hook, and the exit status is 101
