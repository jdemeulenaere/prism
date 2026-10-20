## ADDED Requirements

### Requirement: Drop Hooks Are Declared With Their Type
A struct or enum MAY have one drop hook, `operator mut fun T.drop()`, declared in its own file without a visibility modifier, with no parameter, a `Unit` result, and no type parameters but the type's own, in order and unbounded. Any other `operator` function named `drop` MUST be rejected with `InvalidOperatorDeclaration` (`E4035`), naming the rule it breaks. A drop hook runs whenever a value of its type is dropped (`execution`).

#### Scenario: Drop hooks accepted
- **WHEN** a file declares `struct Session { val id: Int }`, `operator mut fun Session.drop() { println(id) }`, `enum Slot { Full(String), Empty }`, `operator mut fun Slot.drop() {}`, `struct Owned<T>(val v: T)` and `operator mut fun <T> Owned<T>.drop() {}`
- **THEN** the file typechecks successfully

#### Scenario: Drop hook shapes rejected
- **WHEN** a file declaring `struct Session { val id: Int }` declares, one at a time, `operator fun Session.drop() {}`, `operator consuming fun Session.drop() {}`, `operator mut fun Session.drop(now: Bool) {}`, `operator mut fun Session.drop(): Int = 0`, `public operator mut fun Session.drop(): Unit {}`, `operator mut fun String.drop() {}` and `operator fun drop() {}`
- **THEN** the type checker emits `E4035` for each, saying that a drop hook is `operator mut fun Session.drop()` declared without a visibility modifier in the file of a struct or enum

#### Scenario: A drop hook outside its type's file rejected
- **WHEN** `lib/net.prism` declares `public struct Session(val id: Int)`, and `app/main.prism` imports `/lib/net` and declares `operator mut fun Session.drop() {}`
- **THEN** the type checker emits `E4035` for the hook in `app/main.prism`, saying that a drop hook belongs to the file declaring `Session`

#### Scenario: Generic drop hook shapes rejected
- **WHEN** a file declaring `struct Pair<A, B>(val a: A, val b: B)` declares, one at a time, `operator mut fun <A> Pair<A, Int>.drop() {}`, `operator mut fun <B, A> Pair<B, A>.drop() {}` and `operator mut fun <A : Clone, B> Pair<A, B>.drop() {}`
- **THEN** the type checker emits `E4035` for each, saying that a drop hook of `Pair` is declared for every `Pair<A, B>`, with its type parameters in order and unbounded

### Requirement: A Type With a Drop Hook Is Not Copy
A struct or enum declared `: Copy` MUST NOT have a drop hook: its declaration MUST be rejected with `E4035`, saying that a `Copy` value is copied bitwise, so a hook would run once per copy.

#### Scenario: A Copy type with a drop hook rejected
- **WHEN** a file declares `struct Handle : Copy { val id: Int }` and `operator mut fun Handle.drop() {}`
- **THEN** the type checker emits `E4035` at the hook, naming the `: Copy` marker of `Handle`

### Requirement: A Clone Type With a Drop Hook Has a Clone Hook
A struct or enum declared `: Clone` that has a drop hook MUST also have a clone hook, so that cloning never duplicates what its drop releases. Otherwise its drop hook MUST be rejected with `E4035`, with help suggesting `operator fun T.clone(): T`.

#### Scenario: A Clone type with a drop hook alone rejected
- **WHEN** a file declares `struct Session : Clone { val id: Int }` and `operator mut fun Session.drop() {}`
- **THEN** the type checker emits `E4035` at the hook, with help suggesting `operator fun Session.clone(): Session`

#### Scenario: A Clone type with both hooks accepted
- **WHEN** a file declares `fun open(): Int = 1`, `struct Session : Clone { val id: Int }`, `operator mut fun Session.drop() {}` and `operator fun Session.clone(): Session = Session(open())`
- **THEN** the file typechecks successfully

### Requirement: Drop Hooks Are Never Called by Name
A call of `drop()` on a value whose type has a drop hook, through any `&`, `&mut`, `Heap<T>` or `?.`, the hook's own `this` included, MUST be rejected with `DropHookCalled` (`E4036`), with help saying that a value is dropped where its owner's scope ends and that moving it into a function or a block drops it earlier. An extension named `drop` on such a type, in any file, MUST be rejected with `E4010`.

#### Scenario: Calling a drop hook rejected
- **WHEN** a file declares `struct Session { val id: Int }` and `operator mut fun Session.drop() {}`, and `fun f(s: Session) { var t = s; t.drop() }` and `fun g(h: &mut Heap<Session>) { h.drop() }` are checked
- **THEN** the type checker emits `E4036` for each call

#### Scenario: An extension named drop on a type with a hook rejected
- **WHEN** `lib/net.prism` declares `public struct Session(val id: Int)` and `operator mut fun Session.drop() {}`, and `app/main.prism` imports `/lib/net` and declares `mut fun Session.drop() {}`
- **THEN** the type checker emits `E4010` for the extension, saying that the drop hook of `Session` always takes precedence and is never called by name

#### Scenario: Functions named drop stay ordinary
- **WHEN** a file declares `fun <T> drop(x: T) {}`, `struct Token { val id: Int }` and `mut fun Token.drop() {}`, and `fun f(t: Token) { var u = t; u.drop(); drop(u) }` is checked
- **THEN** the file typechecks successfully: `Token` has no drop hook, so `drop` is an ordinary extension, and the top-level `drop` an ordinary function

### Requirement: Values With Drop Hooks Are Not Taken Apart
A pattern or destructuring declaration that would move a part out of a value whose type has a drop hook MUST be rejected with `E4017`, saying that the type has a drop hook and cannot be taken apart, with help suggesting a borrow (`when (&s)`). Binding `Copy` parts, wildcards and borrowing patterns MUST stay accepted.

#### Scenario: Taking apart an enum with a drop hook rejected
- **WHEN** a file declares `enum Slot { Full(String), Empty }` and `operator mut fun Slot.drop() {}`, and `fun f(s: Slot): String = when (s) { Slot.Full(val t) -> t; Slot.Empty -> "" }` is checked
- **THEN** the type checker emits `E4017` for `val t`, saying that `Slot` has a drop hook and cannot be taken apart, with help suggesting `when (&s)`

#### Scenario: Destructuring a struct with a drop hook rejected
- **WHEN** a file declares `struct Named { val name: String, val id: Int }` and `operator mut fun Named.drop() {}`, and `fun f(n: Named): String { val (name) = n; return name }` and `consuming fun Named.take(): String { val (name) = this; return name }` are checked
- **THEN** the type checker emits `E4017` for both destructuring declarations

#### Scenario: Copy parts and borrows of a value with a drop hook
- **WHEN** with that `Named`, `fun f(n: Named): Int { val (id) = n; return id }` and `fun g(n: &Named): Int = when (n) { Named(name = val s, id = _) -> s.length }` are checked
- **THEN** the file typechecks successfully: `id` is copied and `s` borrows its field, so nothing is taken apart

#### Scenario: A field with a drop hook moves out with its holder taken apart
- **WHEN** a file declares `struct Session { val id: Int }`, `operator mut fun Session.drop() {}` and `struct Holder { val session: Session, val tag: Int }`, and `fun f(h: Holder): Session { val (session) = h; return session }` is checked
- **THEN** the file typechecks successfully: `Holder` has no drop hook, so it can be taken apart, and `session` keeps its hook

### Requirement: Clone Hooks Are Declared With Their Type
A struct or enum declared `: Clone` MAY have one clone hook, `operator fun T.clone(): T`, declared in its own file without a visibility modifier, with a shared receiver, no parameter, the result `T`, and no type parameters but the type's own, in order, each unbounded or, for a marker parameter, bounded by `Clone`. Any other `operator` function named `clone` MUST be rejected with `E4035`. The clone hook replaces the type's field-wise clone (`execution`).

#### Scenario: Clone hooks accepted
- **WHEN** a file declares `struct Ticket : Clone { val id: Int }`, `operator fun Ticket.clone(): Ticket = Ticket(id + 100)`, `struct Owned<T> : Clone { val v: T }` and `operator fun <T : Clone> Owned<T>.clone(): Owned<T> = Owned(v.clone())`
- **THEN** the file typechecks successfully

#### Scenario: Clone hook shapes rejected
- **WHEN** a file declaring `struct Ticket : Clone { val id: Int }` declares, one at a time, `operator mut fun Ticket.clone(): Ticket = Ticket(id)`, `operator fun Ticket.clone(): Int = id`, `operator fun Ticket.clone(deep: Bool): Ticket = Ticket(id)` and `public operator fun Ticket.clone(): Ticket = Ticket(id)`
- **THEN** the type checker emits `E4035` for each, saying that a clone hook is `operator fun Ticket.clone(): Ticket` declared without a visibility modifier

#### Scenario: A clone hook needs the Clone marker
- **WHEN** a file declares `struct Token { val id: Int }` and `operator fun Token.clone(): Token = Token(id)`, or `struct Point : Copy { val x: Int }` and `operator fun Point.clone(): Point = Point(x)`
- **THEN** the type checker emits `E4035` for each hook: with help suggesting `: Clone` on `Token`, and saying that a `Copy` value is copied bitwise for `Point`

### Requirement: Hooks Cost Constant Compile Time per Declaration and Use
Checking a hook declaration, a call that resolves to a drop hook, a pattern's subject and a marker declaration MUST each take constant time beyond what they take today, and planning hook instances MUST take time linear in the distinct types a program drops, clones and instantiates, so that files and programs using hooks check in time linear in their size.

#### Scenario: Many hooked types check in linear time
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` runs the `drop-hooks` and `generic-hook-instances` shapes at their three sizes
- **THEN** each shape's time grows at most 4.6 times for each fourfold size

## MODIFIED Requirements

### Requirement: Marker Declarations Are Checked Structurally
A `: Copy` declaration MUST be rejected with `InvalidMarkerDeclaration` (`E4016`) when any field or variant payload is not `Copy`, and a `: Clone` declaration when any is not `Clone`, unless the type has a clone hook, which builds its clones; its marker parameters, if generic, count as satisfying it. The diagnostic MUST name the marker, the field (or the variant and payload position) and its type, once per offending field. A local or imported struct or enum is `Copy` or `Clone` as its marker and, for a generic type, its type arguments say.

#### Scenario: Copy declaration with a non-Copy field rejected
- **WHEN** `struct User : Copy { val id: Int, val name: String, val tags: Heap<Int> }` is declared
- **THEN** the type checker emits `E4016` once for `name` and once for `tags`, and accepts `id`

#### Scenario: Copy enum with a non-Copy payload rejected
- **WHEN** `enum Event : Copy { Click(Int, Int), Text(String) }` is declared
- **THEN** the type checker emits `E4016` naming variant `Text`, its first payload and the type `String`

#### Scenario: Nested Copy types accepted
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `struct Rect : Copy { val origin: Point, val size: Point? }` are declared
- **THEN** both declarations typecheck successfully

#### Scenario: Field of a non-Copy struct rejected
- **WHEN** `struct Point { val x: Int }` (no marker) and `struct Rect : Copy { val origin: Point }` are declared
- **THEN** the type checker emits `E4016` naming `origin` and `Point`

#### Scenario: Clone declaration over Clone and Copy fields
- **WHEN** `struct Point : Copy { val x: Int }`, `struct User : Clone { val name: String }`, `struct Team : Clone { val lead: User, val at: Point }` and `struct Bad : Clone { val p: Raw }` with `struct Raw { val n: Int }` are declared
- **THEN** `Team` typechecks successfully and the type checker emits `E4016` for `Bad.p`

#### Scenario: Field of an imported marked type
- **WHEN** `geo/types` exports `public struct Point : Copy { val x: Int }`, `public struct Label : Clone { val text: String }` and `public struct Path { val name: String }`, and a file importing `/geo/types` declares `struct Seg : Copy { val a: Point }`, `struct Pin : Clone { val l: Label }` and `struct Bad : Copy { val l: Label }`
- **THEN** `Seg` and `Pin` typecheck successfully and the type checker emits `E4016` for `Bad.l`

#### Scenario: Generic fields of a marked declaration
- **WHEN** `struct Box<T> : Copy { val v: T }`, `struct Cell<T> : Clone { val v: T, val next: Heap<T>? }` and `struct Bad<T> : Copy { val b: Box<String> }` are declared
- **THEN** `Box` and `Cell` typecheck successfully, and the type checker emits `E4016` for `Bad.b`, because `Box<String>` is not `Copy`

#### Scenario: A clone hook waives the field check
- **WHEN** `struct Task : Clone { val name: String, val run: () -> Unit }` and `operator fun Task.clone(): Task = Task(name.clone(), { println(1) })` are declared, and `fun f(t: &Task): Task = t.clone()` is checked
- **THEN** the file typechecks successfully, although the field `run` is not `Clone`
