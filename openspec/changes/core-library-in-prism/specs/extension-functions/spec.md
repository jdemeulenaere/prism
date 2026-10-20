## ADDED Requirements

### Requirement: Extension Properties Are Read Like Fields
A read `value.name`, without a call, whose receiver's type after its `&`, `&mut` and `Heap<T>` layers has no field `name` MUST read a property: a member property of the type (`standard-library`, "Intrinsic Extensions Are Members"), else an extension property `name` of the file or of a direct import, found by receiver type constructor and name as an extension function is. The file's own MUST shadow imported ones; several imported MUST be `E4012`, and none `E4006`.

#### Scenario: Reading an extension property
- **WHEN** `struct Box : Copy { val w: Int, val h: Int }`, `val Box.area: Int get() = w * h` and `fun main() { val b = Box(2, 3); val r: &Box = &b; val none: Box? = null; println(b.area + r.area); println(none?.area ?: -1) }` run with `--check-allocations`
- **THEN** stdout is `12` then `-1`, and the exit status is 0

#### Scenario: An imported extension property
- **WHEN** `geo/shapes.prism` declares `package geo`, `public struct Box : Copy { val w: Int, val h: Int }` and `public val Box.area: Int get() = w * h`; a file importing `/geo/shapes` declares `fun f(b: Box): Int = b.area`; and a file importing it declares its own `val Box.area: Int get() = 0` and `fun g(b: Box): Int = b.area`
- **THEN** both files typecheck, the read in `f` calling `geo/shapes::Box.area` and the read in `g` the file's own property

#### Scenario: An unknown property
- **WHEN** a file declares `struct Box : Copy { val w: Int }` and `fun f(b: Box): Int = b.area`
- **THEN** the type checker emits `E4006` for `area`

### Requirement: An Extension Property Read Calls Its Getter
A read of an extension property MUST call its getter with a shared borrow of the receiver, as a method call with a shared receiver does, and have the getter's result as its value; `value?.name` MUST have type `R?`. The getter's type parameters MUST be solved from the receiver's type arguments alone. Assigning a property MUST be `E4003` (it has no setter), and calling it with parentheses MUST call the value read, as for a field.

#### Scenario: A generic extension property
- **WHEN** `struct Pair<T> { val a: T, val b: T }`, `val <T> Pair<T>.first: &T get() = &a` and `fun f(p: &Pair<String>): Int = p.first.length` are checked
- **THEN** the file typechecks, `p.first` having type `&String`, tied to `p` by the elision rules of a getter's shared receiver

#### Scenario: Assigning an extension property rejected
- **WHEN** `struct Box : Copy { val w: Int }`, `val Box.twice: Int get() = w * 2` and `fun f() { var b = Box(1); b.twice = 4 }` are checked
- **THEN** the type checker emits `E4003` for the assignment, saying that `twice` has no setter

#### Scenario: Calling an extension property
- **WHEN** `struct Box : Copy { val w: Int }`, `val Box.twice: Int get() = w * 2` and `fun f(b: Box): Int = b.twice()` are checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: Getter Bodies Are Extension Bodies
A getter's body MUST be checked as the body of an extension function with a shared receiver, the property's type parameters and no parameters, returning the property's type. `this`, a bare name reading the receiver's fields, members and extensions, `return`, and the elision rules for borrows a body returns apply as in such a function. A getter is not a composable context.

#### Scenario: A getter with a block body
- **WHEN** `struct Team { val members: List<String> }` and `val Team.label: String get() { if (members.isEmpty()) return "empty"; return members[0].clone() }` are declared and `fun main() { println(Team(listOf("ann", "bo")).label); println(Team(listOf()).label) }` runs with `--check-allocations`
- **THEN** stdout is `ann` then `empty`, and the exit status is 0

#### Scenario: A getter calling a composable rejected
- **WHEN** `compose fun Leaf() {}`, `struct Box : Copy { val w: Int }` and `val Box.leaf: Int get() { Leaf(); return 1 }` are checked
- **THEN** the type checker emits `E4008` for the call of `Leaf`

### Requirement: Extension Property Declarations Are Checked
An extension property MUST be rejected with `E4010` when the file declares another extension property or extension function of the same name on the same receiver type constructor, when it is named like a field of its receiver struct, or when it is named like a member of its receiver type (a member of a built-in type, or an intrinsic extension of a library type). An extension property and a top-level function or value MAY share a name.

#### Scenario: A property and a function of one name rejected
- **WHEN** a file declares `struct Box : Copy { val w: Int }`, `val Box.size: Int get() = w` and `fun Box.size(): Int = w`
- **THEN** the type checker emits `E4010` for the second declaration

#### Scenario: A property named like a field rejected
- **WHEN** a file declares `struct Box : Copy { val w: Int }` and `val Box.w: Int get() = 0`
- **THEN** the type checker emits `E4010`, saying that the field `w` always takes precedence

#### Scenario: A property named like a member rejected
- **WHEN** a file declares `val String.length: Int get() = 0` and `val <T> List<T>.indices: Range get() = 0..<1`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

### Requirement: Extension Property Reads Cost a Lookup Each
Checking a read of an extension property MUST cost what checking a method call with no arguments costs, at most two table lookups beyond the field and member tests, with no analysis of the getter's body at the read. Checking a file MUST grow near-linearly with its number of property reads and declarations.

#### Scenario: Many property reads check in linear time
- **WHEN** the `extension-properties` shape of `crates/prism-cli/tests/compile_timing.rs` checks files of 256, 1,024 and 4,096 functions, each reading a generic and a non-generic extension property of user types and a member property of `/prism/core`, with one declaration of each property per file, under `PRISM_ASSERT_TIMING=1`
- **THEN** each stage grows by at most ×4.6 per ×4 in size

## MODIFIED Requirements

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field `name` of the receiver's struct. Failing that, it MUST invoke the built-in `clone()` of a `Clone` receiver or `unbox()` of a `Heap<T>` behind its references. Failing that, it MUST invoke a member `name` of its type (`standard-library`, "Intrinsic Extensions Are Members"), such as a scalar's `toString()` or a `List<T>` member, intrinsic or written in Prism, whatever extensions are visible. A member property (`length`, `size`, `indices`) counts as a field. Then it MUST call a receiver function value `name`, and only then an extension.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the member `toString()` of `/prism/core`, `prism/core::Int.toString`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents

### Requirement: Extension Declarations Are Checked
Two extensions in one file with the same receiver type constructor (`Box` for `Box<Int>` and `Box<T>`), name and context MUST be rejected with `E4010`. So MUST an extension that could never be called: one named like a field of its receiver struct, one named `clone` on a `Clone` type, or one named like a member of its receiver type (`standard-library`, "Intrinsic Extensions Are Members"), such as `toString` on `Int`, `Double`, `Bool` or `Char`, `length` on `String`, or `isEmpty` on `List`. Extensions on different receivers or with different contexts, and an extension and a top-level function, MAY share a name.

#### Scenario: Duplicate extension rejected
- **WHEN** a file declares `fun User.greet(): String = "a"` and `fun User.greet(x: Int): String = "b"`
- **THEN** the type checker emits `E4010` for the second declaration

#### Scenario: Extension shadowed by a field rejected
- **WHEN** a file declares `struct User { val name: String }` and `fun User.name(): String = "x"`
- **THEN** the type checker emits `E4010`, saying that the field `name` always takes precedence

#### Scenario: Extension named clone on a Clone type rejected
- **WHEN** a file declares `struct User : Clone { val name: String }`, `fun User.clone(): User = User("x")` and `fun String.clone(): String = "x"`
- **THEN** the type checker emits `E4010` for each, saying that the built-in `clone()` always takes precedence

#### Scenario: Extensions named like built-in members rejected
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun Char.toString(): String = "c"`, `fun String.length(): Int = 0` and `fun <T> List<T>.isEmpty(): Bool = true`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

#### Scenario: Use before an inferred extension declaration
- **WHEN** `fun g(u: User): String { val s: String = u.size(); return s }` precedes `fun User.size() = 42`
- **THEN** the type checker emits `E4002`, exactly as when the declarations are swapped
