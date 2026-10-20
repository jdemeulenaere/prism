## ADDED Requirements

### Requirement: Glue Calls Hooks Through the Function Table
Drop and clone glue MUST call a type's hook as a call of a Prism function: through the slot of the hook's symbol, an extension's (`main::main.Session.drop`, its receiver named with its module) or, for a generic type, an instance's (`main::main.Owned.clone<String>`), never directly, whether the glue belongs to the type's own unit, an importer's or an instance's. Glue MUST run no user code other than hooks.

#### Scenario: Drop glue calls the hook through its slot
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct Session { val id: Int, val name: String }`, `operator mut fun Session.drop() {}` and `fun f(s: Session) {}`
- **THEN** it prints `function main#drop(main::Session)`, which loads the slot of `main::main.Session.drop` and calls it with the value's address before calling the runtime's string drop for `name`, and `f` calls `main#drop(main::Session)` directly

#### Scenario: An importer's glue calls the hook of an imported type
- **WHEN** `lib/net.prism` declares `public struct Session(val id: Int)` and `operator mut fun Session.drop() {}`, and `app/main.prism` imports `/lib/net` and declares `fun f(s: Session) {}`
- **THEN** `prism dump-clif app/main.prism` prints `app/main#drop(lib/net::Session)`, which calls `lib/net::lib/net.Session.drop` through its slot

#### Scenario: Clone glue calls a generic type's clone hook instance
- **WHEN** a file declares `struct Owned<T> : Clone { val v: T }`, `operator fun <T : Clone> Owned<T>.clone(): Owned<T> = Owned(v.clone())` and `fun f(o: &Owned<String>): Owned<String> = o.clone()`
- **THEN** `prism dump-clif main.prism` prints `main#clone(main::Owned<String>)`, which calls the instance `main::main.Owned.clone<String>` through its slot, and `prism dump-clif --instances main.prism` prints the instance's unit

## MODIFIED Requirements

### Requirement: Drops Run Drop Glue
Each drop that the elaborated MIR keeps, guarded by a drop flag or not, MUST run the drop glue of the dropped place's type on that place. A type that owns no storage (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) and reaches no drop hook through its own definition, fields, payloads or nullables MUST have no drop glue: its drops MUST run no code.

#### Scenario: A non-Copy struct is moved and dropped
- **WHEN** `struct Counter { var n: Int }`, `fun take(c: Counter): Int = c.n` and `fun flag(): Bool = false` are declared and `fun main() { val c = Counter(4); if (flag()) println(take(c)); println(1) }` runs
- **THEN** the program prints `1` and exits with status 0

#### Scenario: Dropping a struct frees its strings
- **WHEN** `struct User { val name: String, val nick: String?, val age: Int }` is declared and `fun main() { val u = User("A" + "da", "a".clone() + "da", 36); println(u.age) }` runs with `--check-allocations`
- **THEN** it prints `36`, reports no live allocation and exits with status 0

#### Scenario: A type of integers with a drop hook has drop glue
- **WHEN** `struct Handle { val id: Int }`, `operator mut fun Handle.drop() { println(id) }` and `struct Pair { val a: Handle, val n: Int }` are declared and `fun main() { val p = Pair(Handle(4), 1) }` runs
- **THEN** it prints `4`: `Pair` owns no storage but reaches a drop hook, so its drops run glue

### Requirement: Drop Glue Drops Each Owned Part
Dropping a `String` MUST free its buffer when it owns one. Dropping a struct or enum with a drop hook MUST first call the hook with the value's address. Dropping a struct MUST then drop its fields in declaration order, dropping an enum the payloads of its current variant in order, and dropping a nullable its payload when it is not `null`. A field or payload whose type owns no storage and reaches no drop hook MUST be skipped.

#### Scenario: Dropping an enum drops its current variant
- **WHEN** `enum Message { Text(String, Int), Empty }` is declared and `fun main() { val a = Message.Text(1.toString(), 2); val b = Message.Empty; println(1) }` runs with `--check-allocations`
- **THEN** it prints `1`, reports no live allocation and exits with status 0

#### Scenario: The hook runs before the parts are dropped
- **WHEN** `struct Inner { val name: String }`, `operator mut fun Inner.drop() { println("inner") }`, `struct Outer { val inner: Inner, val label: String }` and `operator mut fun Outer.drop() { println("outer " + inner.name) }` are declared and `fun main() { val o = Outer(Inner(1.toString()), 2.toString()) }` runs with `--check-allocations`
- **THEN** it prints `outer 1` and `inner`, reports no live allocation and exits with status 0

### Requirement: Clone Glue Builds Independent Values
The built-in `clone()` of a value whose type owns storage or reaches a clone hook MUST run the clone glue of its type: a `String` that owns a buffer MUST be cloned into a new buffer holding exactly its text, one that owns none copied; a struct or enum with a clone hook by calling the hook with the source's address and writing its result; a struct without one cloned field by field in declaration order; an enum by its current variant and that variant's payloads in order; a nullable as `null` or a clone of its payload. A part that owns no storage and reaches no clone hook MUST be copied bitwise.

#### Scenario: Cloning a struct of strings
- **WHEN** `struct User : Clone { val name: String, val id: Int }` is declared and `fun main() { val a = User(7.toString(), 7); val b = a.clone(); println(b.name); println(a.name) }` runs with `--check-allocations`
- **THEN** it prints `7` twice, reports no live allocation and exits with status 0

#### Scenario: A clone hook replaces the field-wise clone
- **WHEN** `struct Tag : Clone { val name: String, val id: Int }`, `operator fun Tag.clone(): Tag = Tag("copy of " + name, id + 1)` and `struct Labeled : Clone { val tag: Tag, val n: Int }` are declared and `fun main() { val a = Labeled(Tag(1.toString(), 1), 5); val b = a.clone(); println(b.tag.name); println(b.tag.id); println(b.n) }` runs with `--check-allocations`
- **THEN** it prints `copy of 1`, `2` and `5`, reports no live allocation and exits with status 0
