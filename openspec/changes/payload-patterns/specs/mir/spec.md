# MIR Specification Delta

## MODIFIED Requirements

### Requirement: Statements Assign Rvalues to Places
Every place MUST be a local followed by projections: dereferences, fields by index, nullable payloads, payloads of a variant by index, and indices. A consumed place MUST be a move or copy operand, and every borrow MUST be an rvalue over a place recording whether it is shared or mutable and why it exists.

#### Scenario: Field read through references
- **WHEN** `struct User { val id: Int }` is declared and `fun f(u: &&User): Int = u.id` is lowered
- **THEN** the return place is assigned a copy of field `id` of the place reached by two dereferences of the argument `u`

#### Scenario: Moves, copies and borrows
- **WHEN** `fun greet(n: &String): Int = 1` is declared and `fun f(s: String, n: Int): Int { val a = n; val b = s; return greet(b) }` is lowered
- **THEN** `a` is assigned a copy of `n`, `b` a move of `s`, and `greet` is called with a shared argument borrow of `b`

### Requirement: When Branches Test Patterns in Order
A `when` MUST evaluate its subject once. A `when` without guards whose patterns are all variant patterns with only wildcard and binding sub-patterns, or `else`, MUST switch on the subject's discriminant; any other `when` MUST test its patterns in order, each constructor pattern by its tag and then its sub-patterns from left to right. When no pattern matches, an exhaustive `when` without `else` MUST reach an unreachable terminator.

#### Scenario: When over variants switches on the discriminant
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color): Int = when (c) { Red -> 1; Green -> 2 }` is lowered
- **THEN** the discriminant of `c` is read once and switched on, `0` leading to the branch giving `1`, `1` to the branch giving `2`, and any other value to an unreachable terminator

#### Scenario: When patterns tested in order
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 1; in 1..10 -> 2; else -> 3 }` is lowered
- **THEN** `x` is first compared with `0`, then tested against `1` and `10` only when it is not `0`, and the `else` branch is reached when both tests fail

#### Scenario: Payload bindings switch on the discriminant
- **WHEN** `enum Shape { Circle(Double), Rect(Double, Double) }` is declared and `fun f(s: Shape): Double = when (s) { Shape.Circle(val r) -> r; Shape.Rect(val w, _) -> w }` is lowered
- **THEN** the discriminant of `s` is switched on once; the block of variant 0 copies payload 0 of `Circle` into `r`, the block of variant 1 copies payload 0 of `Rect` into `w`, and any other value reaches an unreachable terminator

#### Scenario: Nested sub-patterns are tested in order
- **WHEN** `enum Box { Of(Int), Empty }` is declared and `fun f(b: Box): Int = when (b) { Box.Of(0) -> 1; Box.Of(val n) -> n; Box.Empty -> 2 }` is lowered
- **THEN** the discriminant of `b` is compared with `0`, then only on that edge its payload is compared with `0`; when either test fails the second pattern's discriminant test follows

### Requirement: MIR Is Well Formed
The MIR of a file without type errors MUST satisfy its structural invariants: every block ends in a terminator and is reachable, every local used is declared, every assignment's value has its place's type except for an explicit subtyping conversion, projections and call operands are well typed, no operand moves a place through a dereference or a place rooted at a capture local, only values whose type needs a drop are dropped, every local is storage-live wherever it is used, and each destructuring moves a whole local into one local per non-`Copy` part of its variant or struct.

#### Scenario: Error-free MIR
- **WHEN** a file typechecks successfully
- **THEN** its MIR satisfies every structural invariant, and so does its elaborated MIR when it passes the borrow checker

## ADDED Requirements

### Requirement: Variant Payloads Are Places
Payload `i` of variant `v` of an enum place MUST be a place of its own, of the payload's type with the enum's type arguments substituted, which a pattern reads, copies or borrows in place. `prism dump-mir` MUST print it as `(place as Variant).i`.

#### Scenario: Borrowing a payload
- **WHEN** `enum Msg { Text(String), Empty }` and `fun len(s: &String): Int = 1` are declared and `fun f(m: &Msg): Int = when (m) { Msg.Text(val s) -> len(s); Msg.Empty -> 0 }` is lowered
- **THEN** in the block of variant 0, `s` is assigned a shared borrow of payload 0 of `Text` of the referent of `m`, printed `&((*_1) as Text).0`, and `m` is not moved

### Requirement: Taking a Value Apart Moves It Into Its Parts
Where a pattern takes its subject apart, MIR MUST move the whole subject local in one destructuring statement into one local per non-`Copy` part of the matched variant or struct: the part's binding, or a temporary of the branch when the part is not bound. A non-whole subject (a nullable's payload) MUST first be moved into a temporary. `Copy` parts MUST be copied before it.

#### Scenario: Destructuring with an unbound part
- **WHEN** `enum Pair { Of(String, String, Int) }` is declared and `fun f(p: Pair): String = when (p) { Pair.Of(val a, _, val n) -> a }` is lowered
- **THEN** in the branch, `n` is copied from payload 2 of `Of`, then one destructuring statement moves `p` into `a` (payload 0) and a temporary (payload 1); the temporary is dropped where the branch ends, and the elaborated MIR drops `p` nowhere

#### Scenario: Nested take-apart
- **WHEN** `struct User { val name: String, val bio: String }` and `enum Event { Joined(User), Left }` are declared and `fun f(e: Event): String = when (e) { Event.Joined(User(val name, _)) -> name; Event.Left -> "" }` is lowered
- **THEN** `e` is destructured into a temporary holding its `User`, which is destructured into `name` and a temporary for `bio`

### Requirement: Bindings Are Made After the Tests and the Guard
A branch MUST make its copying and borrowing bindings after its pattern's tests succeed and before its guard. Its moving bindings MUST be made only once the guard is true. While the guard runs, a moving binding MUST read its part in place, and a shared borrow of the subject (origin `guard`) MUST be held from before the guard until the subject is taken apart. A guard MUST be lowered once per pattern of its branch, and a false guard MUST continue with the next pattern.

#### Scenario: A guard reads a moving binding in place
- **WHEN** `enum Msg { Text(String), Empty }` and `fun short(s: &String): Bool = true` are declared and `fun f(m: Msg): String = when (m) { Msg.Text(val s) if short(s) -> s; else -> "long" }` is lowered
- **THEN** `m` is borrowed with origin `guard` before the guard, which calls `short` with a shared borrow of payload 0 of `Text` of `m`; on its `true` edge that borrow is read and `m` is destructured into `s`, and on its `false` edge control reaches the `else` branch with `m` whole

### Requirement: Destructuring Declarations Lower Like Patterns
A destructuring declaration MUST evaluate its value once, as a place when it is one and otherwise into a temporary, then bind its entries as an irrefutable struct pattern would, its bindings and the temporaries of unbound non-`Copy` fields being locals of the enclosing block.

#### Scenario: Destructuring a parameter
- **WHEN** `struct User { val name: String, val email: String, val id: Int }` is declared and `fun f(u: User): Int { val (id, name) = u; return id }` is lowered
- **THEN** `id` is copied from field `id` of `u`, then `u` is destructured into `name` and a temporary for `email`, both dropped where the body's block ends, and `u` is not dropped
