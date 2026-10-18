# MIR Specification Delta

## ADDED Requirements

### Requirement: A Box's Contents Are Its Dereference
The contents of a `Heap<T>` place MUST be the place's dereference. Reading, borrowing and writing the contents MUST use that place, and writing it MUST drop the old contents first, as any assignment does. No operand MUST move out of it: contents MUST leave a box only through a call of the built-in `unbox`, which moves the whole box.

#### Scenario: Writing a box's contents drops the old contents
- **WHEN** `fun f(h: Heap<String>) { var m = h; *m = "b" }` is lowered
- **THEN** the dereference of `m` is dropped and then assigned the string `"b"`, and `m` is dropped where its scope ends

#### Scenario: Unboxing moves the whole box
- **WHEN** `fun f(h: Heap<String>): String = h.unbox()` is lowered
- **THEN** the MIR of `f` calls `builtin unbox` with a move of `h`, and the elaborated MIR drops `h` nowhere

### Requirement: Taking Apart Through a Box Unboxes It
Where a pattern takes its subject apart through a box, MIR MUST move the box part into a temporary as for any nested part, call the built-in `unbox` on it into a second temporary, and destructure that temporary.

#### Scenario: Destructuring through a box
- **WHEN** `enum Doc { Text(String), Wrapped(Heap<Doc>) }` is declared and `fun f(d: Doc): String = when (d) { Doc.Wrapped(Doc.Text(val s)) -> s; else -> "" }` is lowered
- **THEN** in the first branch, `d` is destructured into a temporary holding its `Heap<Doc>`, which is moved into a call of `builtin unbox` whose result is destructured into `s`

## MODIFIED Requirements

### Requirement: Calls Name Their Callee
A call MUST name its callee (a declaration path for an own or imported function or extension, the built-in `clone`, `length`, `get` (a string's character), `toString`, `Heap` (constructing a box), `unbox`, `print` or `println`, a function-typed place, or a default value's generator), pass one operand per parameter in parameter order, the receiver first, and be marked composable when its callee is composable.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public fun greet(n: Int): Int = n`, and a file importing `/models/user` declares `fun hello(): Int = greet(1)`
- **THEN** the MIR of `hello` calls `models/user::greet` with the constant `1`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the body of `Screen` is composable, its call of `Label` is marked composable, and its call of `log` is not

### Requirement: Receivers Lead the Operands of Calls
The first operand of a call of an extension or of `clone`, `length`, `get`, `toString` or `unbox`, and the first argument of an extension's body, MUST be the receiver, whose type the path or the built-in names; no other call has one.

#### Scenario: An extension call names its receiver
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }` and `public fun Team.captain(o: &String): &String = &lead`, and a file importing `/lib/team` declares `fun f(t: Team, s: String): Int { val c = t.captain(&s); return 1 }`
- **THEN** the MIR of `f` calls `lib/team::Team.captain`, whose path names the receiver type `Team`, with the receiver borrow of `t` first and the borrow of `s` second

#### Scenario: Output built-ins take no receiver
- **WHEN** `fun f(n: Int) { println(n); println() }` is lowered
- **THEN** the MIR of `f` calls `builtin println` with a copy of `n`, which is not a receiver, and then `builtin println` with no operand, and `prism dump-mir` prints both callees as `builtin println`

#### Scenario: String members are built-in calls
- **WHEN** `fun f(s: String, i: Int): Char = s[i]` and `fun g(s: &String): String = s.length.toString()` are lowered
- **THEN** the MIR of `f` calls `builtin get` with a shared receiver borrow of `s` and a copy of `i`, and no place of `f` has an index projection; the MIR of `g` calls `builtin length` with a receiver borrow of the referent of `s`, then `builtin toString` with a copy of its result

#### Scenario: Box built-ins are calls
- **WHEN** `fun f(n: Int): Int { val h = Heap(n); return h.unbox() }` is lowered
- **THEN** the MIR of `f` calls `builtin Heap<Int>` with the value of `n`, which is not a receiver, into `h`, then `builtin unbox` with a move of `h` as its receiver
