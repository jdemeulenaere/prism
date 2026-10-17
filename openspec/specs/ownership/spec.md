# Ownership Specification

## Purpose
The ownership specification defines Prism's single-owner value model as the type checker enforces it: which types are `Copy` or `Clone`, how `: Copy` and `: Clone` declarations are checked, explicit cloning, what a move may take its value from, and why top-level declarations are immutable. Flow-sensitive rules such as use after move belong to the borrow checker and are added here when it exists.

## Requirements

### Requirement: Copy and Clone Are Declared on Structs and Enums
A struct or enum MUST be `Copy` only when it declares `: Copy`, and `Clone` only when it declares `: Clone` or `: Copy`. Without a marker it MUST be move-only and not cloneable, whatever its fields. A `Copy` value is duplicated implicitly by a bitwise copy that runs no user code. A `Clone` value is duplicated only by an explicit `.clone()`. Every other use of a non-`Copy` value moves it.

#### Scenario: Opt-in Copy struct
- **WHEN** `struct Point : Copy { val x: Double, val y: Double }` and `struct Line { val start: Point, val end: Point }` are declared and `fun f(l: Line): Point = l.start` is checked
- **THEN** the file typechecks successfully, because reading a `Copy` field copies it

#### Scenario: Struct without a marker is move-only
- **WHEN** `struct Point { val x: Int, val y: Int }` (no marker, all fields `Copy`) and `struct Line { val start: Point }` are declared and `fun f(l: Line): Point = l.start` is checked
- **THEN** the type checker emits `E4017`, because `Point` is not `Copy`

#### Scenario: Copy enum with payloads
- **WHEN** `enum Shape : Copy { Circle(Float), Square(Float), Empty }` and `struct Slot { val shape: Shape }` are declared and `fun f(s: Slot): Shape = s.shape` is checked
- **THEN** the file typechecks successfully

#### Scenario: Clone struct is not Copy
- **WHEN** `struct User : Clone { val name: String }` and `struct Team { val lead: User }` are declared, and `fun f(t: &Team): User = t.lead.clone()` and `fun g(t: &Team): User = t.lead` are checked
- **THEN** `f` typechecks successfully and the type checker emits `E4017` for `g`

#### Scenario: Copy implies Clone
- **WHEN** `struct Point : Copy { val x: Int }` is declared and `fun f(p: &Point): Point = p.clone()` is checked
- **THEN** the file typechecks successfully

### Requirement: Built-In Copy Types
`Int`, `Float`, `Bool`, `Char`, `Unit`, `Nothing`, the built-in `Range` and shared borrows `&T` MUST be `Copy`, and `T?` MUST be `Copy` exactly when `T` is. `&mut T`, `String`, `Heap<T>`, function types (composable or not), type parameters and type names with no declaration in scope MUST NOT be `Copy`.

#### Scenario: Copy built-ins as fields of a Copy struct
- **WHEN** `struct S : Copy { val a: Int, val b: Float, val c: Bool, val d: Char, val e: Unit, val r: Range, val n: Int? }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: String is move-only
- **WHEN** `struct User : Copy { val name: String }` is declared
- **THEN** the type checker emits `E4016` naming the field `name` and the type `String`

#### Scenario: Non-Copy built-ins rejected in a Copy declaration
- **WHEN** a `: Copy` struct declares a field of type `Heap<Int>`, `() -> Unit`, `compose () -> Unit`, `String?` or a type parameter `T` (`struct Box<T> : Copy { val v: T }`)
- **THEN** the type checker emits `E4016` naming that field

#### Scenario: Unknown type name is not Copy
- **WHEN** `struct Id : Copy { val v: UInt64 }` is declared and no struct or enum `UInt64` is in scope
- **THEN** the type checker emits `E4016` naming the field `v`

#### Scenario: Borrows in enum payloads
- **WHEN** `enum A : Copy { X(&Int) }` and `enum B : Copy { Y(&mut Int) }` are declared
- **THEN** `A` typechecks successfully and the type checker emits `E4016` for `B`'s payload `&mut Int`

### Requirement: Built-In Clone Types
Every `Copy` type and `String` MUST be `Clone`, and `T?` and `Heap<T>` MUST be `Clone` exactly when `T` is. `&mut T`, function types, type parameters and type names with no declaration in scope MUST NOT be `Clone`.

#### Scenario: Clone built-ins as fields of a Clone struct
- **WHEN** `struct Doc : Clone { val title: String, val note: String?, val body: Heap<Int>, val size: Int }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: Non-Clone built-ins rejected in a Clone declaration
- **WHEN** a `: Clone` struct declares a field of type `() -> Unit` (`struct Task : Clone { val run: () -> Unit }`) or a type parameter `T`
- **THEN** the type checker emits `E4016` naming that field

### Requirement: Marker Declarations Are Checked Structurally
A `: Copy` declaration MUST be rejected with `InvalidMarkerDeclaration` (`E4016`) when any field or variant payload is not `Copy`, and a `: Clone` declaration when any is not `Clone`. The diagnostic MUST name the marker, the field (or the variant and payload position) and its type, once per offending field. A local or imported struct or enum is `Copy` or `Clone` as its marker says.

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

### Requirement: Explicit Clone
Every `Clone` type MUST have a built-in member `clone()` taking no arguments, unless the type has a field named `clone`, which takes precedence. It reads its receiver in place, looking through `&` and `&mut`, and returns an owned temporary of the receiver's type with those borrows removed. `x?.clone()` MUST work on nullables. On a type that is not `Clone`, `clone()` MUST be rejected with `NotCloneable` (`E4018`) naming the type.

#### Scenario: Cloning a field through a borrow
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning with safe navigation and elvis
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: &User): String = u.nick?.clone() ?: "anonymous"` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning a Heap
- **WHEN** `struct User : Clone { val name: String }` is declared and `fun f(h: &Heap<User>): Heap<User> = h.clone()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning a type without the marker rejected
- **WHEN** `struct Token { val id: Int }` is declared and `fun f(t: &Token): Token = t.clone()` is checked
- **THEN** the type checker emits `E4018` naming `Token`, with help suggesting `: Clone` on its declaration

#### Scenario: Cloning a function value rejected
- **WHEN** `fun f(g: () -> Unit) { val h = g.clone() }` is checked
- **THEN** the type checker emits `E4018`

#### Scenario: Clone takes no arguments
- **WHEN** `fun f(s: &String): String = s.clone(1)` is checked
- **THEN** the type checker emits `E4004`

#### Scenario: A field named clone wins over the built-in
- **WHEN** `struct S { val clone: () -> Int }` is declared and `fun f(s: &S): Int = s.clone()` is checked
- **THEN** the file typechecks successfully: the call invokes the field, because a member of the receiver's type takes precedence over the built-in `clone()`

### Requirement: Consuming Positions
A value MUST be consumed when it initializes a local, a parameter default or a top-level `val`; is assigned; is passed to a by-value parameter, struct field or enum payload; or is returned (a `return` operand, an expression body, a block tail or closure result). A branch, block tail, parenthesized expression or `?:` operand in a consuming position is consuming. Every other use reads in place.

#### Scenario: Branch results in a consuming position
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User, c: Bool): String { val n = if (c) u.name else "x"; return n }` is checked
- **THEN** the type checker emits `E4017` for `u.name`

#### Scenario: Elvis operands in a consuming position
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: User): String = u.nick ?: "anonymous"` is checked
- **THEN** the type checker emits `E4017` for `u.nick`, with help suggesting `u.nick?.clone()`

#### Scenario: Reads in place are not moves
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(u: User): Bool` uses `u.name` as an operand (`u.name == "a"`, `u.name + "!"`), a `when` subject, the operand of `&` (`val r = &u.name`), the receiver of `clone()` or a discarded expression statement
- **THEN** the body typechecks successfully

### Requirement: Moves Take Whole Values Only
A non-`Copy` value in a consuming position MUST be a whole owned local or by-value parameter, or a temporary (a call, constructor, literal or variant). Taking it from a field (also through a borrow, `Heap<T>` or a temporary), a safe navigation or a top-level `val` MUST be rejected with `CannotMoveOut` (`E4017`), naming the source. Its help MUST suggest a borrow and, for a `Clone` type, `.clone()`. The error type is never reported.

#### Scenario: Moving a non-Copy field rejected
- **WHEN** `struct User { val id: Int, val name: String }` is declared and a body of `fun f(u: User)` declares `val n = u.name`
- **THEN** the type checker emits `E4017` stating that `name` cannot be moved out of `u`, with help suggesting `&u.name` or `u.name.clone()`, while `val i = u.id` in the same body is accepted

#### Scenario: Help for a field that cannot be cloned
- **WHEN** `struct Token { val id: Int }` and `struct Session { val token: Token }` are declared and `fun f(s: Session): Token = s.token` is checked
- **THEN** the type checker emits `E4017`, with help suggesting `&s.token` and no `.clone()`

#### Scenario: Field moves at every consuming site
- **WHEN** with `fun take(s: String)` and `struct Tag { val s: String }` declared, `u.name` for `u: User` is passed to `take`, returned from a function returning `String`, passed to the constructor `Tag(u.name)`, or assigned to a `var` local
- **THEN** the type checker emits `E4017` at each site, and `u.name.clone()` is accepted at each site

#### Scenario: Moving a field through a borrow or Heap
- **WHEN** `fun f(u: &User): String = u.name`, `fun g(u: &mut User): String = u.name` or `fun h(u: Heap<User>): String = u.name` is checked
- **THEN** the type checker emits `E4017` for each

#### Scenario: Moving a field out of a temporary
- **WHEN** `fun make(): User` is declared and `fun f(): String = make().name` is checked
- **THEN** the type checker emits `E4017`

#### Scenario: Moving through a safe navigation
- **WHEN** `fun f(u: User?): String? = u?.name` is checked
- **THEN** the type checker emits `E4017`, while `fun g(u: User?): Int? = u?.id` and `fun h(u: User?): String? = u?.name?.clone()` typecheck successfully

#### Scenario: Moving out of a top-level val
- **WHEN** a file declares `val GREETING = "hi"` and `val LIMIT = 10` at top level, and `fun f(): String = GREETING` and `fun g(): Int = LIMIT`
- **THEN** the type checker emits `E4017` for `GREETING`, with help suggesting `&GREETING` or `GREETING.clone()`, and accepts `g`

#### Scenario: Whole values and temporaries accepted
- **WHEN** `fun take(u: User)` is declared and a body of `fun f(u: User)` runs `val v = u`, `take(v)`, `take(User(1, "a"))`, `take(make())` and `val h = helper` for a top-level `fun helper(): Int`
- **THEN** the body typechecks successfully; whether `u` or `v` is used after it moved is checked by the borrow checker, not by this rule

#### Scenario: Field of a type from an unimported module
- **WHEN** `shop/models` exports `public struct Money : Copy { val cents: Int }` and `public struct Note : Clone { val text: String }`, `shop/api` exports `public struct Order { val price: Money, val note: Note }`, and a file that imports only `/shop/api` declares `fun f(o: Order) { val p = o.price; val c = o.note.clone(); val n = o.note }`
- **THEN** the type checker accepts `o.price` and `o.note.clone()` and emits `E4017` for `val n = o.note`, without reporting `E4014`

### Requirement: No Mutable Top-Level State
Top-level declarations MUST be immutable: a top-level `var` MUST be rejected by the parser with `E2001`, whatever its visibility, with a message saying that mutable state belongs in locals and parameters (and, in UI code, in state handles). Top-level `val` declarations and local `var` declarations MUST be unaffected.

#### Scenario: Top-level var rejected
- **WHEN** a file declares `var counter = 0`, `public var counter: Int = 0` or `package var counter: Int = 0` at top level
- **THEN** the parser rejects the declaration with `E2001`, pointing at `var`

#### Scenario: Top-level val and local var accepted
- **WHEN** a file declares `val LIMIT = 10` at top level and `fun f(): Int { var x = LIMIT; x = x + 1; return x }`
- **THEN** the file typechecks successfully

## Design Rationale
Decisions from `value-semantics` (agreed with the user in the borrow-checker roadmap exploration and while proposing the change):

- **`Copy` and `Clone` are opt-in markers, and `Copy` implies `Clone`.** Each is a deliberate API promise, visible in the declaration and the ABI: adding a field never silently changes what a public type can do, and a unique token or a type that will own a resource is never duplicable by accident. Structural derivation (Swift's model) was rejected because it would need opt-out markers later. `Copy` means an implicit bitwise copy that never allocates; `Clone` means an explicit `.clone()` that may allocate, which is why `String` is `Clone` but not `Copy`: an implicit deep copy would be a hidden allocation, and a hidden reference count is forbidden by the zero-cost memory-safety invariant. Rust's requirement to write both was rejected as buying nothing without custom clone bodies.
- **The predicates are shallow.** A struct or enum is `Copy` or `Clone` exactly as its marker says, whatever its fields, so neither the predicates nor the structural check recurse through type definitions or need a cycle guard. The structural check runs only in the defining file; importers trust the marker recorded in the ABI.
- **The built-in tables.** `&mut T` and function types are neither: a closure holds inferred borrows of its captures, which a copy or clone would duplicate. Type parameters are neither until bounds exist. Unknown (opaque) type names are neither, because nothing has promised that they can be duplicated; this row disappears once unknown names are rejected. The error type counts as both, so one mistake is not reported twice.
- **One marker map in the checker.** The marker of every nominal type comes from the file's own declarations, its imports' exported types, and every nominal reference in an ingested ABI, which covers types whose module the file does not import. Putting the marker on `Type` was rejected: two values of one type could disagree.
- **`clone()`, not `copy()`.** Kotlin's data-class `copy()` is a shallow copy with named overrides, so a deep `copy()` would mislead Kotlin users and would take the name from a possible struct-update feature. `clone` is a built-in member rather than a library function, because methods do not exist yet and a user-written `clone` could not be called generically without traits. Its receiver is read in place through `&` and `&mut`, and its result is a temporary. A user type's clone clones each field; custom clone bodies wait for traits. Deferring cloning until traits was rejected because whole-value moves would leave no way to get an owned copy of a field.
- **A field named `clone` wins over the built-in** (user). This matches Kotlin, where members, including a property of function type, take precedence over extensions. Making the built-in always win was rejected: the parser drops parentheses, so `(x.clone)()` cannot be told apart from `x.clone()`, and preserving them would add an expression node for an edge case. `extension-methods` can apply the same rule to every field-versus-extension clash, and must reject a user extension named `clone` on a `Clone` type.
- **Moves take whole values only.** A value is either whole or gone, which is simpler to explain and to drop, and keeps the future move checker to whole locals. Fields of temporaries are rejected too (`make().name`), so a temporary never needs a partial drop, which matters once destructors exist (Rust forbids it for `Drop` types). `?:` and safe navigation consume, since the result is the field's value. Partial moves and moves out of temporaries remain possible relaxations that break no program. Every case is decided by the expression's shape and type, so it is a type-checker rule.
- **A consuming flag, not a separate pass.** A flag set at consuming sites (initializers, assignments, arguments, returns) and handed through branches, block tails and `?:` operands is taken by the next expression checked, so the leaf check runs where the leaf's type and scope are known. A post-pass would re-infer expressions or resolve names after their scope has closed.
- **One move-out code for every source.** `E4017` names the source ("field 'name' of 'u'", "top-level value 'GREETING'"), and `reference-access` adds "borrow" to it rather than taking a new code, as Rust uses one E0507 for every place behind a reference. Its help suggests a borrow and, for a `Clone` type, `.clone()`.
- **Top-level `var` is removed in the parser.** A borrow of a mutable global created in one function can be invalidated by an assignment in a callee, which no single function's analysis sees; Rust makes `static mut` unsafe for this reason. Rejecting it in the parser leaves no `var` flag in the AST or ABI, following the precedent of borrowed struct fields; a type-checker code would have kept a flag that no valid program sets.
