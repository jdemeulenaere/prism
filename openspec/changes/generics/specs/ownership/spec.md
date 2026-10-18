# Ownership Specification Delta

## ADDED Requirements

### Requirement: Markers of Generic Types Are Conditional
A marker's parameters MUST be the type parameters that occur in a field or payload type of its declaration. An instantiation MUST be `Copy` when the marker is `Copy` and every marker parameter's argument is `Copy`, and `Clone` when the marker is `Copy` or `Clone` and every marker parameter's argument is `Clone`. The declaration check MUST treat each marker parameter as satisfying the marker.

#### Scenario: A Copy generic struct
- **WHEN** `struct Pair<A, B> : Copy { val a: A, val b: B }`, `struct User : Clone { val name: String }` and `struct Raw { val n: Int }` are declared, and `fun f(p: &Pair<Int, Int>): Pair<Int, Int> = p`, `fun g(p: &Pair<User, Int>): Pair<User, Int> = p.clone()`, `fun h(p: &Pair<String, Int>): Pair<String, Int> = p` and `fun k(p: &Pair<Raw, Int>): Pair<Raw, Int> = p.clone()` are checked
- **THEN** `f` and `g` typecheck successfully, the type checker emits `E4017` for `h`, because `Pair<String, Int>` is `Clone` but not `Copy`, and `E4018` for `k`

#### Scenario: A type parameter that no field uses does not condition the marker
- **WHEN** `struct Handle<T> : Copy { val id: Int }` is declared and `fun f(h: &Handle<String>): Handle<String> = h` is checked
- **THEN** the file typechecks successfully, because `Handle<String>` is `Copy`

#### Scenario: A marker parameter inside a non-Copy built-in
- **WHEN** `struct W<T> : Copy { val h: Heap<T> }` and `struct C<T> : Clone { val h: Heap<T>, val n: T? }` are declared
- **THEN** the type checker emits `E4016` for `W.h` and accepts `C`

## MODIFIED Requirements

### Requirement: Built-In Copy Types
`Int`, `Float`, `Bool`, `Char`, `Unit`, `Nothing`, the built-in `Range` and shared borrows `&T` MUST be `Copy`, and `T?` MUST be `Copy` exactly when `T` is. A type parameter MUST be `Copy` exactly when it is bounded by `Copy`. `&mut T`, `String`, `Heap<T>`, function types (composable or not), unbounded or `Clone`-bounded type parameters and type names with no declaration in scope MUST NOT be `Copy`.

#### Scenario: Copy built-ins as fields of a Copy struct
- **WHEN** `struct S : Copy { val a: Int, val b: Float, val c: Bool, val d: Char, val e: Unit, val r: Range, val n: Int? }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: String is move-only
- **WHEN** `struct User : Copy { val name: String }` is declared
- **THEN** the type checker emits `E4016` naming the field `name` and the type `String`

#### Scenario: Non-Copy built-ins rejected in a Copy declaration
- **WHEN** a `: Copy` struct declares a field of type `Heap<Int>`, `() -> Unit`, `compose () -> Unit` or `String?`
- **THEN** the type checker emits `E4016` naming that field

#### Scenario: Unknown type name is not Copy
- **WHEN** `struct Id : Copy { val v: UInt64 }` is declared and no struct or enum `UInt64` is in scope
- **THEN** the type checker emits `E4016` naming the field `v`

#### Scenario: Shared borrows are Copy
- **WHEN** a file declares `val LIMIT = 10`, `val REF: &Int = &LIMIT` and `fun f(): &Int = REF` at top level
- **THEN** the file typechecks successfully, because reading a `&Int` copies the borrow

#### Scenario: Copy-bounded type parameters are Copy
- **WHEN** `fun <T : Copy> get(x: &T): T = x` and `fun <T : Clone> take(x: &T): T = x` are checked
- **THEN** `get` typechecks successfully, copying `x` out of the borrow, and the type checker emits `E4017` for `take`

### Requirement: Built-In Clone Types
Every `Copy` type and `String` MUST be `Clone`, and `T?` and `Heap<T>` MUST be `Clone` exactly when `T` is. A type parameter MUST be `Clone` exactly when it is bounded by `Copy` or `Clone`. `&mut T`, function types, unbounded type parameters and type names with no declaration in scope MUST NOT be `Clone`.

#### Scenario: Clone built-ins as fields of a Clone struct
- **WHEN** `struct Doc : Clone { val title: String, val note: String?, val body: Heap<Int>, val size: Int }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: Non-Clone built-ins rejected in a Clone declaration
- **WHEN** a `: Clone` struct declares a field of type `() -> Unit` (`struct Task : Clone { val run: () -> Unit }`)
- **THEN** the type checker emits `E4016` naming that field

#### Scenario: Clone-bounded type parameters are Clone
- **WHEN** `fun <T : Clone> c(x: &Heap<T>): Heap<T> = x.clone()` is checked
- **THEN** the file typechecks successfully

### Requirement: Marker Declarations Are Checked Structurally
A `: Copy` declaration MUST be rejected with `InvalidMarkerDeclaration` (`E4016`) when any field or variant payload is not `Copy`, and a `: Clone` declaration when any is not `Clone`, where each of the declaration's marker parameters counts as satisfying the marker. The diagnostic MUST name the marker, the field (or the variant and payload position) and its type, once per offending field. A local or imported struct or enum is `Copy` or `Clone` as its marker and, for a generic type, its type arguments say.

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
