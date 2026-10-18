# Typed Tree Specification Delta

## MODIFIED Requirements

### Requirement: Control Flow Stays Structured
Blocks MUST keep their statements and tail value, and a block whose value is discarded MUST hold its last expression as a statement. `if`, `when`, `&&`, `||` and `?:` MUST stay structured nodes. A `when` MUST read its subject in place, resolve each pattern to a variant or struct pattern with one sub-pattern per payload or field, a wildcard, a binding, a value, a range or a condition, keep each branch's guard, and record whether it is exhaustive.

#### Scenario: A discarded tail becomes a statement
- **WHEN** `fun g(): Int = 1` is declared and `fun f() { g() }` is checked
- **THEN** the block of `f` has the call of `g` as an expression statement and no tail, and has type `Unit`

#### Scenario: Discarded branches hold their tails as statements
- **WHEN** `fun g(): Int = 1` is declared and a body of `fun f(c: Bool)` runs `if (c) { g() }` and `when { c -> { g() }; else -> {} }` as statements
- **THEN** the `then` branch of the `if` and each branch of the `when`, whose values are discarded, is a block of type `Unit` with no tail, the first two holding the call of `g` as an expression statement

#### Scenario: When patterns resolved to variants
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color): Int = when (c) { Red -> 1; Green -> 2 }` is checked
- **THEN** the `when` reads the binding `c` in place, its patterns are variants 0 and 1 of `Color` without sub-patterns, and it is exhaustive

#### Scenario: Nested patterns and guards resolved
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `enum Event { Click(Point), Key(Char) }` are declared and `fun f(e: Event): Int = when (e) { Event.Click(Point(y = 0)) -> 0; Event.Click(Point(val x, _)) if x > 1 -> x; Event.Key -> 2; _ -> 3 }` is checked
- **THEN** the first pattern is variant 0 of `Event` holding a struct pattern of `Point` whose sub-patterns are a wildcard for `x` and the value `0` for `y`; the second holds a binding of `x` and a wildcard, and its branch has the guard `x > 1`; the third is variant 1 with a wildcard sub-pattern; the fourth is a wildcard

## ADDED Requirements

### Requirement: Pattern Bindings Record Their Mode
Each binding of a pattern MUST be a binding of the body, of kind pattern binding, recording its mode: a copy or a move of the part (the binding having the part's type), or a shared or mutable borrow of it (`&T` or `&mut T`). The patterns of one branch MUST share one binding per name.

#### Scenario: Binding modes in the tree
- **WHEN** `enum Msg { Text(String), Count(Int) }` is declared and `fun a(m: Msg): Int = when (m) { Msg.Text(val s) -> 1; Msg.Count(val n) -> n }` and `fun b(m: &Msg): Int = when (m) { Msg.Text(val s) -> 1; Msg.Count(val n) -> n }` are checked
- **THEN** in `a`, `s` is a moving binding of type `String` and `n` a copying binding of type `Int`; in `b`, `s` is a shared-borrow binding of type `&String` and `n` a copying binding of type `Int`

#### Scenario: Alternatives share a binding
- **WHEN** `enum Shape { Circle(Double), Square(Double) }` is declared and `fun f(s: Shape): Double = when (s) { Shape.Circle(val d), Shape.Square(val d) -> d }` is checked
- **THEN** both patterns bind the same binding `d`, which the branch body reads

### Requirement: Destructuring Declarations Are Statements
A destructuring declaration MUST be a statement of its block holding its value, read in place, and a struct pattern of the value's struct, whose sub-patterns are a binding for each named field and a wildcard for each other field, in declaration order.

#### Scenario: A destructuring statement
- **WHEN** `struct User { val name: String, val id: Int }` is declared and `fun f(u: User): Int { val (id as n, name) = u; return n }` is checked
- **THEN** the block of `f` starts with a destructuring statement reading `u` in place, whose struct pattern of `User` binds `name` by move and `n` by copy, in field order
