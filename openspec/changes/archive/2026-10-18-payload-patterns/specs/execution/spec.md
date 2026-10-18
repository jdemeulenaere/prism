# Execution Specification Delta

## ADDED Requirements

### Requirement: Patterns Match Payloads at Run Time
A constructor pattern MUST match exactly when the subject holds its variant, or is its struct, and every sub-pattern matches the part at its position. A binding MUST hold the matched part, a copy of it, or a borrow of it, as its mode says.

#### Scenario: Reading payloads
- **WHEN** `enum Shape { Circle(Int), Rect(Int, Int), Empty }` and `fun area(s: Shape): Int = when (s) { Shape.Circle(val r) -> 3 * r * r; Shape.Rect(val w, val h) -> w * h; Shape.Empty -> 0 }` are declared and a program prints `area(Shape.Circle(2))`, `area(Shape.Rect(3, 4))` and `area(Shape.Empty)`
- **THEN** it prints `12`, `12` and `0`

#### Scenario: Nested and literal sub-patterns
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Event : Copy { Click(Point), Key(Char) }` and `fun f(e: Event): Int = when (e) { Event.Click(Point(0, val y)) -> y; Event.Click(Point(val x, _)) -> x * 10; Event.Key('q') -> -1; Event.Key(_) -> -2 }` are declared and a program prints `f(Event.Click(Point(0, 7)))`, `f(Event.Click(Point(2, 7)))`, `f(Event.Key('q'))` and `f(Event.Key('w'))`
- **THEN** it prints `7`, `20`, `-1` and `-2`

#### Scenario: Writing a payload through a mutable binding
- **WHEN** `enum Counter { Running(Int), Stopped }`, `fun tick(c: &mut Counter) { when (c) { Counter.Running(val n) -> *n += 1; Counter.Stopped -> {} } }` and `fun value(c: &Counter): Int = when (c) { Counter.Running(val n) -> n; Counter.Stopped -> -1 }` are declared and `fun main() { var c = Counter.Running(1); tick(&mut c); tick(&mut c); println(value(&c)) }` runs
- **THEN** it prints `3`

#### Scenario: Binding the whole subject
- **WHEN** `enum Note { Text(String), Empty }`, `fun keep(n: Note): Note = when (n) { Note.Empty -> Note.Empty; val other -> other }` and `fun text(n: &Note): String = when (n) { Note.Text(val s) -> s.clone(); Note.Empty -> "" }` are declared and `fun main() { println(text(&keep(Note.Text("kept")))) }` runs with `prism run --check-allocations`
- **THEN** it prints `kept` and exits with status 0, reporting no leak and no invalid free

### Requirement: Guards Run After Their Pattern Matches
A guard MUST run only when its pattern matched, after the tests of that pattern and before the next pattern is tested. A false guard MUST let testing continue with the next pattern, the subject unchanged.

#### Scenario: Guard order and fallthrough
- **WHEN** `enum Box { Of(Int), Empty }`, `fun big(n: Int): Bool { println(n); return n > 9 }` and `fun f(b: Box): Int = when (b) { Box.Empty -> 0; Box.Of(val n) if big(n) -> 1; Box.Of(_) -> 2 }` are declared and a program prints `f(Box.Empty)`, `f(Box.Of(10))` and `f(Box.Of(3))`
- **THEN** it prints `0`; then `10` and `1`; then `3` and `2`

#### Scenario: Alternatives with a guard
- **WHEN** `enum Shape { Circle(Int), Square(Int) }` and `fun f(s: Shape): Int = when (s) { Shape.Circle(val d), Shape.Square(val d) if d > 1 -> d; else -> 0 }` are declared and a program prints `f(Shape.Circle(5))`, `f(Shape.Square(5))` and `f(Shape.Square(1))`
- **THEN** it prints `5`, `5` and `0`

### Requirement: Taken-Apart Values Release Each Part Once
When a value is taken apart, each moved part MUST be owned by its binding from then on, and each part no binding takes MUST be dropped where the branch or the enclosing block ends, so every allocation the value owned is freed exactly once.

#### Scenario: Taking strings apart
- **WHEN** `enum Note { Pair(String, String), Empty }` and `fun first(n: Note): String = when (n) { Note.Pair(val a, _) -> a; Note.Empty -> "" }` are declared and `fun main() { println(first(Note.Pair("left", "right"))) }` runs with `prism run --check-allocations`
- **THEN** it prints `left` and exits with status 0, reporting no leak and no invalid free

#### Scenario: Destructuring a consuming receiver
- **WHEN** `struct Account { val name: String, val bio: String, val id: Int }` and `consuming fun Account.label(): String { val (name, id) = this; return name }` are declared and `fun main() { val a = Account("ada", "math", 1); println(a.label()) }` runs with `prism run --check-allocations`
- **THEN** it prints `ada` and exits with status 0, reporting no leak and no invalid free

#### Scenario: Borrowing leaves the subject whole
- **WHEN** `enum Note { Text(String), Empty }`, `fun size(s: &String): Int = s.length` and `fun main() { val n = Note.Text("abc"); val k = when (&n) { Note.Text(val s) -> size(s); Note.Empty -> 0 }; println(k); val m = n }` run with `prism run --check-allocations`
- **THEN** it prints `3` and exits with status 0, reporting no leak and no invalid free
