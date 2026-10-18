# Patterns Specification Delta

## Purpose

Defines Prism's patterns: what a `when` branch and a destructuring declaration may match and bind, how a pattern is checked against its subject, how its bindings hold the subject's parts (copied, moved or borrowed), and when a `when` covers every value of its subject.

## ADDED Requirements

### Requirement: Variant Patterns Mirror Construction
A `when` branch pattern `Enum.Variant(p1, ..., pn)`, or `Variant(...)` when the bare name resolves as a variant pattern does today, MUST match a subject holding that variant whose payloads match the sub-patterns `p1` to `pn` in declaration order. There MUST be one sub-pattern per payload, and sub-patterns are positional, as payloads are.

#### Scenario: Binding a variant's payload
- **WHEN** `enum Shape { Circle(Double), Rect(Double, Double) }` is declared and `fun area(s: Shape): Double = when (s) { Shape.Circle(val r) -> r * r * 3.0; Shape.Rect(val w, val h) -> w * h }` is checked
- **THEN** the file typechecks, `r`, `w` and `h` have type `Double`, and the `when` is exhaustive without `else`

#### Scenario: Bare variant name in a constructor pattern
- **WHEN** with that `Shape`, `fun f(s: Shape): Double = when (s) { Circle(val r) -> r; Rect(_, val h) -> h }` is checked
- **THEN** the file typechecks, as `Shape.Circle(val r)` and `Shape.Rect(_, val h)` would

#### Scenario: Wrong number of payload patterns rejected
- **WHEN** with that `Shape`, `fun f(s: Shape): Double = when (s) { Shape.Rect(val w) -> w; else -> 0.0 }` is checked
- **THEN** the type checker emits `E4024`, stating that `Rect` has 2 payloads and the pattern gives 1

#### Scenario: Arguments on a payload-free variant rejected
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color): Int = when (c) { Color.Red(_) -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4024`

#### Scenario: Named sub-patterns on a variant rejected
- **WHEN** with that `Shape`, `fun f(s: Shape): Double = when (s) { Shape.Circle(r = val r) -> r; else -> 0.0 }` is checked
- **THEN** the type checker emits `E4024`, because payloads have no names

### Requirement: Struct Patterns Mirror Construction
A pattern naming a struct, `Point(p1, ..., pn)` or `Point(x = p1, ...)`, MUST match a struct subject whose fields match its sub-patterns. Positional sub-patterns MUST give every field in declaration order. Named sub-patterns, which may follow positional ones as in a call, MUST each name a field once, and every field they leave out is matched by a wildcard.

#### Scenario: Positional and named struct patterns
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` is declared and `fun f(p: Point): Int = when (p) { Point(0, val y) -> y; Point(y = 0) -> 1; Point(val x, _) -> x }` is checked
- **THEN** the file typechecks and the `when` is exhaustive

#### Scenario: Struct pattern errors
- **WHEN** with that `Point`, the patterns `Point(val x)`, `Point(x = 1, x = 2)`, `Point(z = 1)` and `Point(x = 1, 2)` are checked in a `when (p)`
- **THEN** the type checker emits `E4024` for the first (2 fields, 1 pattern), `E4024` for the second (field `x` named twice), `E4006` for the third, and `E4024` for the fourth (a positional pattern after a named one)

#### Scenario: Struct pattern of an imported struct
- **WHEN** `shop/models` exports `public struct Point : Copy { val x: Int, val y: Int }` and a file importing `/shop/models` declares `fun f(p: Point): Int = when (p) { Point(0, val y) -> y; Point(x = val x) -> x }`
- **THEN** the file typechecks and the `when` is exhaustive, as for a struct of the same file

#### Scenario: Struct pattern nested in a variant pattern
- **WHEN** with that `Point`, `enum Event { Click(Point), Key(Char) }` is declared and `fun f(e: Event): Int = when (e) { Event.Click(Point(val x, _)) -> x; Event.Key(_) -> 0 }` is checked
- **THEN** the file typechecks and the `when` is exhaustive

### Requirement: Sub-Patterns
A sub-pattern MUST be a constructor pattern, a wildcard `_` matching anything, a binding `val name` or `var name` matching anything and binding it, an `in a..b` or `in a..<b` range, or a value expression matching a part equal to it under `==`. These forms other than value expressions MUST also be accepted at the top of a `when` branch with a subject.

#### Scenario: Value and range sub-patterns
- **WHEN** `enum Shape { Circle(Double), Named(String) }` is declared and `fun f(s: Shape, limit: Double): Int = when (s) { Shape.Circle(0.0) -> 0; Shape.Circle(limit) -> 1; Shape.Circle(in 0.0..1.0) -> 2; Shape.Named("x") -> 3; else -> 4 }` is checked
- **THEN** the file typechecks, each sub-pattern being compared with the payload of its position

#### Scenario: Top-level wildcard and binding
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 1; _ -> 2 }` and `fun g(x: Int): Int = when (x) { 0 -> 1; val n -> n * 2 }` are checked
- **THEN** both typecheck and are exhaustive, `_` and `val n` covering every `Int`

#### Scenario: Patterns need a subject
- **WHEN** parsing `fun f(c: Bool): Int = when { _ -> 1 }` or `fun g(c: Bool): Int = when { val x -> 1; else -> 2 }`
- **THEN** the parser reports `E2001`, stating that a branch of a `when` without a subject is a condition

### Requirement: A Payload Variant Named Alone Matches the Variant
In pattern position, a variant that has payloads, written without parentheses (`Shape.Circle`, or a bare `Circle`), MUST match a subject holding that variant whatever its payloads, and bind nothing.

#### Scenario: Matching a variant without its payloads
- **WHEN** `enum Shape { Circle(Double), Rect(Double, Double) }` is declared and `fun isRound(s: &Shape): Bool = when (s) { Shape.Circle -> true; Shape.Rect -> false }` is checked
- **THEN** the file typechecks, and the `when` is exhaustive

### Requirement: Constructor Patterns Are Told Apart From Values
A branch pattern written as a dotted name followed by `(` MUST be a constructor pattern unless a binary operator, `.`, `?.`, `[` or `{` follows its closing `)`, in which case the whole pattern is a value expression. A constructor pattern whose name is neither a variant nor a struct MUST be a value expression when all its arguments are values, and otherwise `E4024`.

#### Scenario: A call followed by an operator is a value
- **WHEN** `fun g(n: Int): Int = n` is declared and `fun f(x: Int): Int = when (x) { g(1) + 1 -> 1; g(3) -> 2; else -> 3 }` is checked
- **THEN** the file typechecks, `g(1) + 1` and `g(3)` being values compared with `x`

#### Scenario: Binding inside a function call rejected
- **WHEN** `fun g(n: Int): Int = n` is declared and `fun f(x: Int): Int = when (x) { g(val n) -> n; else -> 0 }` is checked
- **THEN** the type checker emits `E4024`, stating that only a variant or a struct can be matched by its parts

### Requirement: Bindings Are Declared With val or var
A pattern MUST bind a name only through `val name` or `var name`. A bare identifier in a pattern MUST stay a value compared with `==`, as in Kotlin's `when`, and an unresolved one MUST be `E4001` with help suggesting `val name`. A `var` binding MUST be a mutable local of the binding's type.

#### Scenario: A bare identifier compares
- **WHEN** `enum Box { Of(Int) }` is declared and `fun f(b: Box, n: Int): Int = when (b) { Box.Of(n) -> 1; else -> 2 }` is checked
- **THEN** the file typechecks, `n` being compared with the payload, and the `when` is not exhaustive without its `else`

#### Scenario: An unresolved bare identifier suggests a binding
- **WHEN** with that `Box`, `fun f(b: Box): Int = when (b) { Box.Of(n) -> n }` is checked
- **THEN** the type checker emits `E4001` for `n`, with help suggesting `val n`

#### Scenario: A var binding is mutable
- **WHEN** with that `Box`, `fun f(b: Box): Int = when (b) { Box.Of(var n) -> { n += 1; n } }` is checked
- **THEN** the file typechecks

### Requirement: Bindings Are Scoped to Their Branch
A branch's bindings MUST be visible in its guard and its body only, not in the patterns that declare them nor in other branches. They may shadow outer locals. One pattern binding a name twice MUST be `E4010`.

#### Scenario: Duplicate binding in one pattern
- **WHEN** `enum P { Two(Int, Int) }` is declared and `fun f(p: P): Int = when (p) { P.Two(val a, val a) -> a }` is checked
- **THEN** the type checker emits `E4010`

#### Scenario: A binding shadows an outer local
- **WHEN** `enum Box { Of(Int) }` is declared and `fun f(b: Box, n: Int): Int = when (b) { Box.Of(val n) -> n }` is checked
- **THEN** the file typechecks, and `n` in the branch body is the binding

### Requirement: Binding Modes Follow the Subject's Type
A binding MUST take its part by value when the subject's type is not a reference (a `Copy` part is copied, any other part moved), as a shared borrow `&T` of a non-`Copy` part and a copy of a `Copy` part when the subject is `&E`, and as a mutable borrow `&mut T` of any part when the subject is `&mut E`. Nested patterns keep the mode of their subject.

#### Scenario: Owned subject binds by value
- **WHEN** `enum Msg { Text(String), Count(Int) }` is declared and `fun f(m: Msg): String = when (m) { Msg.Text(val s) -> s; Msg.Count(_) -> "n" }` is checked
- **THEN** the file typechecks, and `s` has type `String`

#### Scenario: Shared subject binds by shared borrow
- **WHEN** with that `Msg`, `fun len(s: &String): Int = 1` and `fun f(m: &Msg): Int = when (m) { Msg.Text(val s) -> len(s); Msg.Count(val n) -> n }` are checked
- **THEN** the file typechecks, `s` has type `&String` and `n` has type `Int`

#### Scenario: Mutable subject binds by mutable borrow
- **WHEN** with that `Msg`, `fun bump(m: &mut Msg) { when (m) { Msg.Count(val n) -> *n += 1; Msg.Text(val s) -> *s = "x" } }` is checked
- **THEN** the file typechecks, `n` having type `&mut Int` and `s` type `&mut String`

#### Scenario: Receiver modes choose the binding mode
- **WHEN** with that `Msg`, `fun Msg.peek(): Int = when (this) { Msg.Count(val n) -> n; Msg.Text(val s) -> 0 }` and `consuming fun Msg.text(): String = when (this) { Msg.Text(val s) -> s; Msg.Count(_) -> "" }` are checked
- **THEN** both typecheck, `s` having type `&String` in `peek` and `String` in `text`

#### Scenario: Borrowing from an owned local
- **WHEN** with that `Msg`, `fun len(s: &String): Int = 1` and `fun f(m: Msg): Int { val n = when (&m) { Msg.Text(val s) -> len(s); Msg.Count(_) -> 0 }; return n }` are checked
- **THEN** the file typechecks, `s` has type `&String`, and `m` is not moved

### Requirement: Guards
A branch of a `when` with a subject MUST accept a guard, `if condition`, after its patterns and before `->`, without parentheses. The guard MUST typecheck as `Bool` (`E4002` otherwise) and sees the branch's bindings. A guarded branch matches only when one of its patterns matches and the guard is true for that pattern's bindings.

#### Scenario: Guard over a binding
- **WHEN** `enum Shape { Circle(Double), Square(Double) }` is declared and `fun f(s: Shape): Int = when (s) { Shape.Circle(val r) if r > 10.0 -> 1; Shape.Circle(_) -> 2; Shape.Square(_) -> 3 }` is checked
- **THEN** the file typechecks and is exhaustive

#### Scenario: Non-Bool guard rejected
- **WHEN** with that `Shape`, `fun f(s: Shape): Int = when (s) { Shape.Circle(val r) if r -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Guard on else or without a subject rejected
- **WHEN** parsing `fun f(x: Int): Int = when (x) { else if x > 0 -> 1 }` or `fun g(c: Bool): Int = when { c if c -> 1; else -> 2 }`
- **THEN** the parser reports `E2001` for each

### Requirement: Guards Do Not Move or Mutate Bindings
Inside a guard, every binding of its branch MUST be read-only and MUST NOT be moved. A move of a by-value binding there MUST be `E4017` naming the binding "in a guard", and an assignment, `&mut` or `mut fun` receiver through any binding MUST be `E4003`, since the guard may fail and later branches still test the subject.

#### Scenario: Moving a binding in a guard rejected
- **WHEN** `enum Msg { Text(String) }` and `fun take(s: String): Bool = true` are declared and `fun f(m: Msg): Int = when (m) { Msg.Text(val s) if take(s) -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4017` for `s`, stating that a binding cannot be moved in a guard

#### Scenario: Mutating a binding in a guard rejected
- **WHEN** `enum Box { Of(Int) }` and `fun set(n: &mut Int): Bool = true` are declared and `fun f(b: &mut Box): Int = when (b) { Box.Of(val n) if set(n) -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4003` for `n`, stating that a binding cannot be mutated in a guard

#### Scenario: Reading a binding in a guard
- **WHEN** with that `Msg`, `fun short(s: &String): Bool = true` and `fun f(m: Msg): String = when (m) { Msg.Text(val s) if short(s) -> s; Msg.Text(_) -> "long" }` are checked
- **THEN** the file typechecks: the guard borrows the payload, and the body moves it

### Requirement: Guards Do Not Change a Subject They Take Apart
A guard MUST NOT assign, move or mutably borrow the subject of a pattern that takes it apart once the guard is true, since the branch would then take apart a value that may no longer hold the matched variant. The borrow checker MUST report it as a conflict with a shared borrow of the subject held from the pattern to the take-apart (`E5003` to `E5005`).

#### Scenario: Mutating a subject the branch takes apart rejected
- **WHEN** `enum Msg { Text(String) }` and `fun clear(m: &mut Msg): Bool = true` are declared and `fun f(m: Msg): String { var v = m; return when (v) { Msg.Text(val s) if clear(&mut v) -> s; else -> "" } }` is checked
- **THEN** the borrow checker emits `E5003` at `&mut v`, labelling `val s` as where the pattern takes `v` apart after the guard

### Requirement: Alternatives Bind the Same Names
In a branch with several comma-separated patterns, every pattern MUST bind the same names, each with the same type and binding mode in every pattern; otherwise the type checker MUST report `InconsistentAlternativeBindings` (`E4025`) naming the binding.

#### Scenario: Alternatives binding one name
- **WHEN** `enum Shape { Circle(Double), Square(Double), Empty }` is declared and `fun size(s: Shape): Double = when (s) { Shape.Circle(val d), Shape.Square(val d) -> d; Shape.Empty -> 0.0 }` is checked
- **THEN** the file typechecks and is exhaustive

#### Scenario: Alternatives binding different names
- **WHEN** with that `Shape`, `fun f(s: Shape): Double = when (s) { Shape.Circle(val d), Shape.Square(val e) -> 1.0; else -> 0.0 }` is checked
- **THEN** the type checker emits `E4025` for `d`, which the second pattern does not bind

#### Scenario: Alternatives binding different types
- **WHEN** `enum V { I(Int), D(Double) }` is declared and `fun f(v: V): Int = when (v) { V.I(val x), V.D(val x) -> 1 }` is checked
- **THEN** the type checker emits `E4025`, `x` being `Int` in one pattern and `Double` in the other

### Requirement: Exhaustiveness Over Nested Patterns
A `when` in value position MUST be exhaustive: every value of its subject's type matches some unguarded pattern. Enums, structs, `Bool` and nullables are covered by their constructors, nested to any depth. A wildcard or binding covers anything. A guarded branch, a range, a value that is neither a literal nor a payload-free variant, and a literal other than `true`, `false` and `null` cover nothing on their own.

#### Scenario: Nested coverage
- **WHEN** `enum Inner { A, B }` and `enum Outer { Wrap(Inner), Empty }` are declared and `fun f(o: Outer): Int = when (o) { Outer.Wrap(Inner.A) -> 1; Outer.Wrap(Inner.B) -> 2; Outer.Empty -> 3 }` is checked
- **THEN** the `when` is exhaustive without `else`

#### Scenario: A literal sub-pattern does not cover its variant
- **WHEN** `enum Box { Of(Int), Empty }` is declared and `fun f(b: Box): Int = when (b) { Box.Of(0) -> 1; Box.Empty -> 2 }` is checked
- **THEN** the type checker emits `E4009`, stating that `Box.Of(_)` is not covered

#### Scenario: A guarded branch covers nothing
- **WHEN** `enum Box { Of(Int), Empty }` is declared and `fun f(b: Box): Int = when (b) { Box.Of(val n) if n > 0 -> 1; Box.Empty -> 2 }` is checked
- **THEN** the type checker emits `E4009`, stating that `Box.Of(_)` is not covered

#### Scenario: Boolean payloads are covered by both literals
- **WHEN** `enum Flag { On(Bool), Off }` is declared and `fun f(x: Flag): Int = when (x) { Flag.On(true) -> 1; Flag.On(false) -> 2; Flag.Off -> 3 }` is checked
- **THEN** the `when` is exhaustive without `else`

#### Scenario: A when statement needs no coverage
- **WHEN** `enum Box { Of(Int), Empty }` is declared and `fun f(b: Box) { when (b) { Box.Of(0) -> println(0) } }` is checked
- **THEN** the file typechecks

### Requirement: Nullable Subjects Match Patterns of Their Payload
For a subject of type `T?`, `null` MUST match `null`, and a constructor pattern, a variant name or a literal of `T` MUST match a non-null subject whose payload matches it, so `null` together with patterns covering `T` covers `T?`. A binding at the top binds the whole `T?`.

#### Scenario: Nullable enum subject
- **WHEN** `enum Shape { Circle(Double), Square(Double) }` is declared and `fun f(s: Shape?): Double = when (s) { null -> 0.0; Shape.Circle(val r) -> r; Shape.Square(val d) -> d }` is checked
- **THEN** the file typechecks and is exhaustive

#### Scenario: Variant names over a nullable enum
- **WHEN** `enum Color { Red, Green }` is declared and `fun f(c: Color?): Int = when (c) { null -> 0; Red -> 1; Color.Green -> 2 }` is checked
- **THEN** the file typechecks and is exhaustive

#### Scenario: Nullable payload
- **WHEN** `enum Slot { Of(Int?) }` is declared and `fun f(s: Slot): Int = when (s) { Slot.Of(null) -> 0; Slot.Of(1) -> 1; Slot.Of(val n) -> 2 }` is checked
- **THEN** the file typechecks and is exhaustive, `n` having type `Int?`

### Requirement: Uncovered Values Are Named
A non-exhaustive `when` in value position MUST be reported as `NonExhaustiveWhen` (`E4009`), naming one value that no branch covers as a pattern (`Shape.Rect(_, _)`, `Outer.Wrap(Inner.B)`, `null`, `false`, `_` for an open type), with help suggesting a branch for it or an `else` branch.

#### Scenario: The missing variant is named
- **WHEN** `enum Shape { Circle(Double), Rect(Double, Double) }` is declared and `fun f(s: Shape): Double = when (s) { Shape.Circle(val r) -> r }` is checked
- **THEN** the type checker emits `E4009`, stating that `Shape.Rect(_, _)` is not covered

#### Scenario: An open type needs a catch-all
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 1; 1 -> 2 }` is checked
- **THEN** the type checker emits `E4009`, stating that `_` is not covered

### Requirement: Destructuring Declaration Syntax
The grammar MUST accept `val (entries) = value` and `var (entries) = value` wherever a local declaration is accepted, with one or more comma-separated entries, each an identifier optionally followed by `as` and an identifier, and an optional trailing comma. A type annotation, an empty list or any other entry MUST be `E2001`. `as` is a keyword only inside the list.

#### Scenario: Parsing destructuring declarations
- **WHEN** parsing `fun f(u: User) { val (name as n, email,) = u; var (age) = u; val as = 1 }`
- **THEN** the body holds two destructuring declarations, the first binding `n` and `email`, the second a `var` binding `age`, and a local named `as`

#### Scenario: Malformed destructuring declarations rejected
- **WHEN** parsing `val (a, b): Pair = p`, `val () = p`, `val (a.b) = p` or `val (_) = p` inside a function body
- **THEN** the parser reports `E2001` for each

### Requirement: Destructuring Declarations Name Fields
`val (e1, ..., en) = value` and `var (...) = value` MUST declare one local per entry. An entry `field` binds the struct's field `field` to a local of that name, and `field as name` binds it to `name`. Fields not named are not bound. An unknown field MUST be `E4006`, a field named twice `E4024`, and two entries binding one name `E4010`.

#### Scenario: Destructuring a struct by field names
- **WHEN** `struct User { val name: String, val email: String, val age: Int }` is declared and `fun f(u: User): String { val (email, name) = u; return name }` is checked
- **THEN** the file typechecks, `name` and `email` having type `String` and the field of the same name

#### Scenario: Renaming an entry
- **WHEN** with that `User`, `fun f(a: User, b: User): Int { val (age) = a; val (age as other) = b; return age + other }` is checked
- **THEN** the file typechecks

#### Scenario: Destructuring errors
- **WHEN** with that `User`, `val (nick) = u`, `val (age, age as a) = u` and `val (name, email as name) = u` are checked in a body of `fun f(u: User)`
- **THEN** the type checker emits `E4006`, `E4024` and `E4010` respectively

#### Scenario: Positional names are not fields
- **WHEN** `struct Pair(val first: Int, val second: Int)` is declared and `fun f(p: Pair): Int { val (a, b) = p; return a }` is checked
- **THEN** the type checker emits `E4006` for `a`, with help suggesting `first as a`

### Requirement: Destructuring Takes Only Structs
The value of a destructuring declaration MUST be a struct, or a reference to one. Any other value MUST be `E4024`, with help suggesting a `when` for an enum.

#### Scenario: Destructuring an enum rejected
- **WHEN** `enum Box { Of(Int) }` is declared and `fun f(b: Box) { val (x) = b }` is checked
- **THEN** the type checker emits `E4024`

### Requirement: Destructuring Follows the Binding Modes
A destructuring declaration MUST bind its entries as a pattern does over its value. An owned value's `Copy` fields are copied and its other fields moved, which takes the value apart. A `&S` value binds non-`Copy` fields as `&T` and copies the rest. A `&mut S` value binds every field as `&mut T`.

#### Scenario: Taking a consuming receiver apart
- **WHEN** `struct Account { val name: String, val id: Int }`, `struct User { val name: String }` and `consuming fun Account.toUser(): User { val (name) = this; return User(name) }` are checked
- **THEN** the file typechecks: `name` has type `String`, moved out of `this`

#### Scenario: Destructuring a borrow
- **WHEN** with that `Account`, `fun len(s: &String): Int = 1` and `fun f(a: &Account): Int { val (name, id) = a; return len(name) + id }` are checked
- **THEN** the file typechecks, `name` having type `&String` and `id` type `Int`

#### Scenario: Copy fields do not consume
- **WHEN** with that `Account`, `fun f(a: Account): Int { val (id) = a; val (name) = a; return id }` is checked
- **THEN** the file typechecks, since copying `id` leaves `a` whole
