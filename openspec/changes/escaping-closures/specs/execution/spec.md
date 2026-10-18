# Execution Specification Delta

## ADDED Requirements

### Requirement: Closures That Own Their Captures Keep Them
A closure that owns its captures MUST keep the values it captured when it was built for as long as it lives, and each call MUST read those values, however many times it is called, wherever it is stored and after the function that built it has returned.

#### Scenario: A returned closure keeps what it captured
- **WHEN** `fun greeter(name: String): (String) -> String = { greeting -> greeting + ", " + name }` and `fun main() { val hello = greeter("Ada"); println(hello("Hello")); println(hello("Bye")) }` run with `--check-allocations`
- **THEN** it prints `Hello, Ada` and then `Bye, Ada`, reports no live allocation and exits with status 0

#### Scenario: A handler stored in a struct
- **WHEN** `struct Button { val label: String, val onClick: () -> Unit }`, `fun button(label: String, onClick: escaping () -> Unit): Button = Button(label, onClick)`, `fun click(b: &Button) { b.onClick() }` and `fun main() { val name = "Ada"; val b = button("Greet") { println("Hi " + name) }; click(b); click(b); println(b.label) }` run with `--check-allocations`
- **THEN** it prints `Hi Ada` twice and then `Greet`, reports no live allocation and exits with status 0

#### Scenario: A closure owning another closure
- **WHEN** `fun make(s: String): () -> Int = { s.length }`, `fun twice(g: escaping () -> Int): () -> Int = { g() * 2 }` and `fun main() { val h = twice(make("abc")); println(h()); println(h()) }` run with `--check-allocations`
- **THEN** it prints `6` twice, reports no live allocation and exits with status 0

#### Scenario: A closure in a default value
- **WHEN** `fun pick(a: Int, f: () -> Int = { a * 2 }): Int = f()` and `fun main() { println(pick(4)); println(pick(4) { 1 }) }` run with `--check-allocations`
- **THEN** it prints `8` and then `1`, reports no live allocation and exits with status 0

#### Scenario: Closures in generic and boxed values
- **WHEN** `struct Box<T>(val v: T)`, `fun <T> keep(x: T): T = x` and `fun main() { val s = "abc"; val b = Box(keep({ s.length })); println(b.v()); val t = "de"; val h = Heap({ t.length }); println(1) }` run with `--check-allocations`
- **THEN** it prints `3` and then `1`, reports no live allocation and exits with status 0

### Requirement: Owned Environments Are Freed Exactly Once
An owned environment MUST be freed exactly once, when the function value that owns it is dropped: at the end of its owner's scope, when the variable holding it is assigned another value, or with the struct, enum, box or nullable holding it, and its owned captures MUST be dropped with it.

#### Scenario: Reassigning a variable frees the old environment
- **WHEN** `fun make(s: String): () -> Int = { s.length }` and `fun main() { var h = make("a" + "b"); println(h()); h = make("cde"); println(h()) }` run with `--check-allocations`
- **THEN** it prints `2` and then `3`, reports no live allocation and exits with status 0

#### Scenario: A closure kept past its binding's block
- **WHEN** `fun main() { var h: () -> Int = { 0 }; var i = 0; while (i < 3) { val s = "a" + i.toString(); h = { s.length + 10 }; i += 1 }; println(h()) }` runs with `--check-allocations`
- **THEN** it prints `12`, reports no live allocation and exits with status 0

#### Scenario: A dropped struct frees its handler
- **WHEN** `struct Item { val name: String, val onSelect: () -> Int }` and `fun item(name: String): Item { val n = name.clone(); return Item(name, { n.length }) }` are declared and `fun main() { val a = item("x" + "yz"); println(a.onSelect()); val b = item("w"); println(b.name) }` run with `--check-allocations`
- **THEN** it prints `3` and then `w`, reports no live allocation and exits with status 0
