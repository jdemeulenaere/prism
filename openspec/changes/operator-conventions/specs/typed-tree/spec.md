## ADDED Requirements

### Requirement: Operator Uses Are Marked Calls
An operator use that calls an operator function MUST be a call of that function, normalized as an extension call (its receiver an explicit first argument, borrowed, moved or copied by its mode), marked with the operator it spells, which the dump shows (`call operator + main::Vector2.plus`). An ordering use MUST be the built-in comparison of that call with `0`, `!in` the negation of the `contains` call, and the call MUST record `a` as evaluated before the receiver for `in`.

#### Scenario: A marked operator call
- **WHEN** `struct Vector2 : Copy { val x: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = other` are declared and `fun f(a: Vector2, b: Vector2): Vector2 = a + b` is checked
- **THEN** the body's value is a call of `plus` marked with the operator `+`, whose arguments are a shared receiver borrow of the binding `a` and a copy of the binding `b`

#### Scenario: An ordering use compares the call with zero
- **WHEN** `struct Version : Copy { val major: Int }` and `operator fun Version.compareTo(other: &Version): Int = major - other.major` are declared and `fun f(a: Version, b: Version): Bool = a < b` is checked
- **THEN** the body's value is the built-in `<` of a call of `compareTo` marked with `<` and the literal `0`

### Requirement: Equality Stays an Equality Node
`==` and `!=` MUST stay binary equality nodes over operands read in place whether or not the compared type has an `equals` operator: which comparison runs is the code generator's (`code-generation`), so a type's `equals` reaches nested and element comparisons through one rule.

#### Scenario: Equality of a type with equals
- **WHEN** `struct User { val id: Int }` and `operator fun User.equals(other: &User): Bool = id == other.id` are declared and `fun f(a: &User, b: &User): Bool = a == b` is checked
- **THEN** the body's value is a binary `==` whose operands are the dereferences of the bindings `a` and `b`, read in place

### Requirement: Index Places Through Accessors
An index of a type with index operators MUST be a call of the chosen `get` (shared or `mut`) with the target as its receiver and the indices in order. When `get` returns a borrow, the element MUST be the dereference of a temporary holding that call, a place, which a borrow of the element (whole or a part) reborrows. A write through `set` MUST be a call statement.

#### Scenario: Index places in the tree
- **WHEN** `struct Item { var count: Int }`, `struct Bag { var items: List<Item> }`, `operator fun Bag.get(index: Int): &Item = &items[index]` and `operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]` are declared and `fun f(b: &mut Bag): Int { b[0].count = 1; val r: &Item = &b[1]; return b[2].count }` is checked
- **THEN** the assignment's target is field 0 of the dereference of a temporary holding a call of the `mut` `get` with a mutable receiver borrow of the dereferenced `b`; `r`'s initializer is a shared borrow of the dereference of a temporary holding a call of the shared `get`; and the result copies field 0 of the dereference of a temporary holding a call of the shared `get`

### Requirement: Compound Assignments Through Operator Functions
A compound assignment through an operator function MUST be a compound assignment statement recording the function as well as its operator, with the target appearing once. One through `get` and `set` MUST be a block of statements: a temporary per index, then the call of `set` whose value argument is the operator applied to `get`'s element and the value.

#### Scenario: A compound assignment recording its function
- **WHEN** `struct Vector2 : Copy { val x: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = other` are declared and a body of `fun f(v: Vector2)` runs `var t = v; t += v`
- **THEN** the second statement is a compound assignment `+` through `plus` whose target is the binding `t` and whose value is a copy of `v`

### Requirement: Cursor Loops Record Their Protocol
A `for` loop over a cursor MUST record the iteration as a cursor loop naming its `iterator`, `hasNext` and `next` functions (the `mut` `next` when iterating `&mut`), the cursor's type and the element type, and MUST hold its iterable as a borrow whose origin is the loop, shared or mutable.

#### Scenario: A cursor loop in the tree
- **WHEN** the `Bag` cursor protocol of `operators` is declared and `fun f(b: &Bag) { for (item in b) {} }` is checked
- **THEN** the loop iterates by cursor through a shared borrow of the dereferenced binding `b` whose origin is the loop, naming `iterator`, `hasNext` and `next` of `Bag`, the cursor type `Int`, and binds `item` of type `&Item`
