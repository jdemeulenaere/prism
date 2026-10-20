## ADDED Requirements

### Requirement: Extension Property Reads Lower to Calls
A read of an extension property written in Prism MUST lower to a `Call` terminator of its getter's declaration path, whose first operand is the receiver's shared borrow, as for an extension call with a shared receiver, writing the property's value to the call's destination. A getter's body MUST lower as the body of an extension function with no parameter.

#### Scenario: A unit property in MIR
- **WHEN** `prism dump-mir` runs on `fun f(n: Int): Dp = n.dp` with `import /prism/compose/ui`
- **THEN** the body of `f` calls `prism/compose/ui::Int.dp` with a shared receiver borrow of `n` into the return place, and builds no aggregate `Dp`

## MODIFIED Requirements

### Requirement: Calls Name Their Callee
A call MUST name its callee: a declaration path for an own or imported function, extension or extension property getter, the built-in `clone`, `length`, `get` (a string's character), `Heap` (constructing a box), `unbox`, `print` or `println`, a function-typed place, or a default value's generator. It MUST pass one operand per parameter in parameter order, the receiver first, and be marked composable when its callee is composable.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public fun greet(n: Int): Int = n`, and a file importing `/models/user` declares `fun hello(): Int = greet(1)`
- **THEN** the MIR of `hello` calls `models/user::greet` with the constant `1`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the body of `Screen` is composable, its call of `Label` is marked composable, and its call of `log` is not

### Requirement: Receivers Lead the Operands of Calls
The first operand of a call of an extension, of an extension property getter, or of `clone`, `length`, `get` or `unbox` MUST be the receiver, whose type the path or the built-in names. So MUST the first argument of an extension's or getter's body. No other call has one.

#### Scenario: An extension call names its receiver
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }` and `public fun Team.captain(o: &String): &String = &lead`, and a file importing `/lib/team` declares `fun f(t: Team, s: String): Int { val c = t.captain(&s); return 1 }`
- **THEN** the MIR of `f` calls `lib/team::Team.captain`, whose path names the receiver type `Team`, with the receiver borrow of `t` first and the borrow of `s` second

#### Scenario: Output built-ins take no receiver
- **WHEN** `fun f(n: Int) { println(n); println() }` is lowered
- **THEN** the MIR of `f` calls `builtin println` with a copy of `n`, which is not a receiver, and then `builtin println` with no operand, and `prism dump-mir` prints both callees as `builtin println`

#### Scenario: String members are built-in calls
- **WHEN** `fun f(s: String, i: Int): Char = s[i]` and `fun g(s: &String): String = s.length.toString()` are lowered
- **THEN** the MIR of `f` calls `builtin get` with a shared receiver borrow of `s` and a copy of `i`, and no place of `f` has an index projection; the MIR of `g` calls `builtin length` with a receiver borrow of the referent of `s` into a temporary, then `prism/core::Int.toString` with a shared receiver borrow of that temporary

#### Scenario: Box built-ins are calls
- **WHEN** `fun f(n: Int): Int { val h = Heap(n); return h.unbox() }` is lowered
- **THEN** the MIR of `f` calls `builtin Heap<Int>` with the value of `n`, which is not a receiver, into `h`, then `builtin unbox` with a move of `h` as its receiver

### Requirement: List Operations Are Built-In Calls
A vararg's list, `size` and each intrinsic list member MUST be `Call` terminators of a built-in callee, printed `builtin varargList`, `builtin size`, `builtin add` and so on, with the list's receiver borrow first (when it has one) and then the arguments, in the order `Calls Name Their Callee` and `Receivers Lead the Operands of Calls` give. `isEmpty`, `isNotEmpty` and `indices` MUST be calls of their declarations in `/prism/core`. `removeAt` and `removeLast` MUST write the removed element to their destination as a value the caller owns.

#### Scenario: Built-in list calls
- **WHEN** `fun f(): Int { var xs = listOf(1, 2); xs.add(3); if (xs.isNotEmpty()) return xs.removeAt(0); return 0 }` is lowered
- **THEN** the MIR calls `builtin varargList` with `1` and `2` into a temporary and `prism/collections::listOf` with a move of that temporary into `xs`, then `builtin add` with a mutable borrow of `xs` and `3`, `prism/core::List<T>.isNotEmpty<Int>` with a shared borrow of `xs`, and `builtin removeAt` with a mutable borrow of `xs` and `0` into the return place

## REMOVED Requirements

### Requirement: Unit Properties Lower to Constructions
**Reason**: `n.dp` and `n.sp` are ordinary extension properties of `/prism/compose/ui` written in Prism (`get() = Dp(this)`), so a read is a call of the getter, which builds the aggregate in its own body ("Extension Property Reads Lower to Calls").
**Migration**: The scenario "A unit in MIR" becomes "A unit property in MIR" of "Extension Property Reads Lower to Calls"; its fixture is renamed and re-blessed.
