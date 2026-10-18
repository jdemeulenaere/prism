# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Structs Are Laid Out Inline
A struct value MUST be stored inline, its fields in its own storage without heap indirection. Its fields MUST be placed in order of decreasing alignment, fields of equal alignment in declaration order, each at the next multiple of its alignment; the struct's alignment MUST be its largest field's (1 without fields) and its size the end of its last field rounded up to that alignment.

#### Scenario: Value struct definition
- **WHEN** `struct Point { val x: Double, val y: Double }` is declared and used in `fun main() { val p = Point(1.0, 2.0); println(p.y) }`
- **THEN** `prism dump-layout main.prism` shows `Point` with size 16 and alignment 8, `x` at offset 0 and `y` at offset 8, and `prism run main.prism` prints `2.0`

#### Scenario: Fields ordered by alignment
- **WHEN** `struct S { val a: Bool, val n: Int, val b: Bool }` is declared
- **THEN** `prism dump-layout` shows `S` with size 16, `n` at offset 0, `a` at 8 and `b` at 9

### Requirement: Enums Are Laid Out as a Tag and Payloads
An enum value MUST store its variant's index, in declaration order, in the smallest unsigned integer of 8, 16 or 32 bits that numbers every variant, at offset 0, followed by the variant's payload, laid out as a struct's fields are, starting after the tag. All variants MUST share the storage after the tag; the enum's size MUST be its largest variant's end rounded up to its alignment.

#### Scenario: Payload-free enum
- **WHEN** `enum Color { Red, Green, Blue }` is declared
- **THEN** `prism dump-layout` shows `Color` with size 1, alignment 1 and an 8-bit tag at offset 0

#### Scenario: Recursive enum with Heap indirection
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` is declared
- **THEN** `prism dump-layout` shows a finite layout for `Expr`: size 24, an 8-bit tag at offset 0, the payload of `Literal` at 8, and the payloads of `Add` at 8 and 16

#### Scenario: A small payload follows the tag
- **WHEN** `enum Flag { On(Bool), Off }` is declared
- **THEN** `prism dump-layout` shows `Flag` with size 2, its tag at offset 0 and the payload of `On` at 1

### Requirement: Nullables Are Laid Out With a Tag or as a Null Pointer
A `T?` MUST be laid out as an 8-bit tag at offset 0, 0 for `null` and 1 otherwise, followed by `T` at the next multiple of its alignment, except that `(&T)?`, `(&mut T)?` and `Heap<T>?` MUST be a single pointer whose value is 0 exactly when it is `null`.

#### Scenario: A nullable integer
- **WHEN** a body holds a value of type `Int?`
- **THEN** `prism dump-layout` shows `Int?` with size 16, its tag at offset 0 and its payload at 8

#### Scenario: An optional borrow is one pointer
- **WHEN** `struct Point : Copy { val x: Int }` is declared and a function takes `a: (&Point)?` and `b: (&mut Point)?`
- **THEN** `prism dump-layout` shows both nullable types with the size and alignment of a pointer and no tag

### Requirement: Instances of Generic Types Are Laid Out by Substitution
A generic struct or enum used at concrete type arguments outside a generic body MUST be laid out as its declaration with the arguments substituted for its type parameters, and values of it MUST be compiled like those of any struct or enum.

#### Scenario: A generic struct at concrete types
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)` is declared and `fun main() { val p = Pair(1, true); println(p.b) }` runs
- **THEN** `prism dump-layout main.prism` shows `Pair<Int, Bool>` with size 16, `a` at 0 and `b` at 8, and the program prints `true`

### Requirement: Layouts of Types From Modules Not Imported
A file MUST lay out every value it can hold, including values of private types and of types defined in modules it does not import, from its own declarations and the ABIs of its direct imports alone.

#### Scenario: A type reached through an imported struct
- **WHEN** `geo/core.prism` declares `public struct Vec : Copy { val x: Int }`, `geo/shapes.prism` imports `/geo/core` and declares `public struct Circle : Copy { val center: Vec, val r: Int }`, `public fun unit(): Circle = Circle(Vec(3), 1)` and `public fun norm(v: Vec): Int = v.x`, and `app/main.prism` imports only `/geo/shapes` and declares `fun main() { val c = unit(); val d = c; println(d.r); println(norm(c.center)) }`
- **THEN** `prism run app/main.prism` prints `1` and then `3`, and `prism dump-layout app/main.prism` shows `geo/shapes::Circle` with size 16

#### Scenario: A private field type
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `public struct Outer : Copy { val s: Secret, val n: Int }` and `public fun make(): Outer = Outer(Secret(1), 2)`, and `app/main.prism` imports `/lib/a` and declares `fun main() { val o = make(); println(o.n) }`
- **THEN** `prism run app/main.prism` prints `2`

### Requirement: Aggregates Are Passed by Address
A parameter whose type is a struct, an enum or a nullable laid out with a tag MUST be passed as the address of memory the caller owns for the duration of the call, and such a result MUST be written to memory whose address the caller passes before the other arguments. A function's machine signature MUST NOT depend on the layout of any type.

#### Scenario: Signature of a function over structs
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun f(p: Point, n: Int): Point = p` are declared and `prism dump-clif main.prism` runs
- **THEN** `f` takes three machine values, the result's address, the address of `p` and the 64-bit `n`, and returns nothing

#### Scenario: A field added keeps the signature
- **WHEN** `Point` of that file gains a field `val z: Int`
- **THEN** `prism dump-clif main.prism` shows the same signature for `f`

### Requirement: Drops of Values That Own Nothing Do Nothing
A value whose type owns no storage (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) MUST be dropped without running any code. A struct or enum that owns storage MUST be rejected with `E6001` naming what it holds, until drop glue exists.

#### Scenario: A non-Copy struct is moved and dropped
- **WHEN** `struct Counter { var n: Int }`, `fun take(c: Counter): Int = c.n` and `fun flag(): Bool = false` are declared and `fun main() { val c = Counter(4); if (flag()) println(take(c)); println(1) }` runs
- **THEN** the program prints `1` and exits with status 0

#### Scenario: A struct holding a string
- **WHEN** `struct User { val name: String }`, `fun age(u: &User): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `age`, naming strings, and exits with status 1

### Requirement: Layout Dump
`prism dump-layout <file.prism>` MUST print, as deterministic text, the layout of each struct and enum the file declares, in declaration order, then of every other struct, enum and nullable type that a compiled body of the file holds, by rendered name: size, alignment, and the offset of every field, tag and payload. A type without a layout yet MUST be listed with the construct it waits for. With type or borrow errors it MUST report them, print nothing and exit with a failure status.

#### Scenario: Dumping a file's layouts
- **WHEN** `prism dump-layout main.prism` runs twice on a file that `prism check` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: A type without a layout yet
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct User { val name: String }` and `struct Box<T>(val v: T)`
- **THEN** it lists `User` as having no layout yet because of strings, and `Box` as laid out per instance, and exits with status 0

#### Scenario: Dumping a file with errors
- **WHEN** `prism dump-layout main.prism` runs on `struct P { val x: Int }` and `fun f(): Int = nope`
- **THEN** it reports `E4001`, prints no layout, and exits with a failure status

## MODIFIED Requirements

### Requirement: Unsupported Constructs Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val name: String }` and `fun main() { val p = P("a"); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `P("a")`, naming strings, prints nothing to stdout and exits with status 1

#### Scenario: An enum holding a heap value is rejected
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Literal(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `Expr.Literal(1)`, naming `'Heap' values`, prints nothing to stdout and exits with status 1

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `println("hi")`, or `val h: Heap<Int>? = null`, or `val f = { 1 }`, or `println(id(1))` for `fun <T> id(x: T): T = x`, or the program declares `compose fun Label() {}` or `fun first(s: String): Int = 1`
- **THEN** `prism run` reports `E6001` naming strings at `"hi"`, `'Heap' values` at `null`, closures at `{ 1 }`, calls of generic functions at `id(1)`, composable functions at `Label`, and strings at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(): Int { val a = "x"; val b = "y"; return 1 }` and `fun g(): Int { val c = "z"; return 2 }` are declared with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at `"x"` in `f` and one at `"z"` in `g`
