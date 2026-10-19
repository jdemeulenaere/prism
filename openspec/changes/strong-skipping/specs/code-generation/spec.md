## ADDED Requirements

### Requirement: Reading a State's Version Is One Call
`state version` MUST be a direct call of `prism_compose_state_version` with the state's handle word and the slot type record of its value type, giving the version as a 64-bit integer. When the entry reports a failure, the generated code MUST raise the error the runtime recorded at the operation's location, as for the other state operations.

#### Scenario: A version read in the IR dump
- **WHEN** `compose fun Show(s: &String) {}` and `compose fun Screen(items: &State<List<String>>) { Show(items.value[0]) }` are declared and `prism dump-clif main.prism` runs
- **THEN** `main::Screen` calls `prism_compose_state_version` with the handle and the record of `List<String>`, and passes its result as the last argument of its call of `main::Show`

## MODIFIED Requirements

### Requirement: Composables Take the Composer and Change Words
The machine signature of a composable function, and the signature of the code of a composable closure, MUST be that of its declared parameters followed by the composer, its change words and one version word per borrowed parameter, each a 64-bit integer. A call of a composable function value MUST pass the environment, the arguments, the composer, the change words and the version words, in that order.

#### Scenario: Signature of a composable
- **WHEN** `compose fun Label(n: Int) {}` is declared and `prism dump-clif main.prism` runs
- **THEN** `main::Label` takes three 64-bit integers: `n`, the composer and one change word

#### Scenario: A call of a composable function value
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` is declared and `prism dump-clif main.prism` runs
- **THEN** the call of `content` in `main::Column` passes the environment, then `Column`'s composer and a change word

#### Scenario: Signature of a composable taking a borrow
- **WHEN** `compose fun Show(s: &String, n: Int) {}` is declared and `prism dump-clif main.prism` runs
- **THEN** `main::Show` takes five 64-bit integers: the address of `s`, `n`, the composer, one change word and one version word

### Requirement: Slot Type Records
Each code unit MUST define one slot type record per type that its composed MIR keeps or remembers, named `<unit>#slot-type(<type>)`: the type's 256-bit type fingerprint, its size and alignment, the address of its drop glue or null when it needs no drop, the address of its equality glue, and its name as the slot dump prints it. The record of a kept borrow's reference type MUST describe its identity: 16 bytes, no drop glue, and glue comparing both words.

#### Scenario: Records of a unit
- **WHEN** `struct Point : Copy { val x: Int }`, `compose fun Badge(n: Int) {}` and `compose fun P(n: Int) { val p = remember { Point(n) } }` are declared and `prism dump-clif main.prism` runs
- **THEN** it defines the data `main#slot-type(Int)` and `main#slot-type(main::Point)`, once each

#### Scenario: The record of a kept borrow
- **WHEN** `compose fun Show(s: &String) {}` is declared and `prism dump-clif main.prism` runs
- **THEN** it defines the data `main#slot-type(&String)` with the size 16, a null drop entry and an equality entry comparing the two words of its values

### Requirement: Kept Values Are Compared by Equality Glue
Every type with a slot type record, other than a kept borrow's reference type, MUST have equality glue taking the two values' addresses and returning a `Bool`, scalars included, that gives `==` of the type. The equality glue of a function type MUST compare two values as equal exactly when they have the same descriptor, its environment equality entry is not null, and the entry finds their environments equal. Keeping a borrow MUST call no glue of its referent type.

#### Scenario: A kept value that is never equal to itself
- **WHEN** `struct D : Copy { val v: Double }`, `compose fun Show(d: D) { println("show") }`, `fun screen(v: Double): compose () -> Unit = { Show(D(v)) }` and `fun main() { val nan = 0.0 / 0.0; val c = composition(screen(nan)); c.setContent(screen(nan)); c.setContent(screen(1.0)); c.setContent(screen(1.0)) }` run
- **THEN** `show` is printed three times: on both passes with `NaN`, and on the first pass with `1.0`
