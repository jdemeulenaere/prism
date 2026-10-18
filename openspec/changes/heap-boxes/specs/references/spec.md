# References Specification Delta

## ADDED Requirements

### Requirement: A Box Lends Its Contents to Reference Parameters
Where a call's parameter has type `&T`, an argument of type `Heap<T>`, behind any number of boxes and references, MUST lend its contents: a place MUST be read where it lives and its contents borrowed, without being consumed, and any other argument MUST be a temporary whose contents are borrowed until the end of the statement. A `Heap<T>?` argument MUST lend its contents to a `(&T)?` parameter likewise.

#### Scenario: Passing a box where its contents are borrowed
- **WHEN** `enum Expr { Lit(Int), Neg(Heap<Expr>) }` and `fun eval(e: &Expr): Int = 0` are declared, and a body of `fun f(h: Heap<Expr>, r: &Heap<Expr>, hh: Heap<Heap<Expr>>)` runs `eval(h)`, `eval(h)`, `eval(r)`, `eval(hh)` and `eval(Heap(Expr.Lit(1)))`
- **THEN** the body typechecks successfully, and `h` is not reported as moved

#### Scenario: Passing an optional box where an optional borrow is expected
- **WHEN** `struct Tree { val value: Int, val left: Heap<Tree>? }` and `fun depth(t: (&Tree)?): Int = 0` are declared and `fun f(t: &Tree): Int = depth(t.left)` is checked
- **THEN** the file typechecks successfully, and `t.left` is not reported as moved

#### Scenario: A box is not lent to a mutable parameter implicitly
- **WHEN** `fun bump(r: &mut Int) {}` is declared and `fun f(h: Heap<Int>) { var m = h; bump(m); bump(&mut m) }` is checked
- **THEN** the type checker emits `E4004` for `bump(m)`, with help suggesting `&mut m`, and accepts `bump(&mut m)`, which borrows the contents of `m` mutably

### Requirement: Borrowed Boxes Convert to Borrowed Contents
`&Heap<T>` MUST convert implicitly to `&T`, and `&mut Heap<T>` to `&mut T` or `&T`, through any number of boxes, wherever such a type is expected, as a borrow of the box's contents. A borrowed `Heap<T>?` MUST convert to `(&T)?` likewise. A value that already fits its expected type MUST NOT be converted.

#### Scenario: Returning a borrow of a box's contents
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` is declared, and `fun contents(h: &Heap<Node>): &Node = h` and `fun next(n: &Node): (&Node)? = &n.next` are checked
- **THEN** both typecheck successfully

#### Scenario: A generic parameter borrows the box itself
- **WHEN** `fun <T> same(x: &T): &T = x` is declared and `fun f(h: Heap<Int>): Int { val r = same(&h); return 1 }` is checked
- **THEN** the file typechecks, and `r` has type `&Heap<Int>`

### Requirement: Borrows of a Box's Contents Borrow the Box
A box MUST own its contents: a borrow of a box's contents, or of a part of them, MUST conflict with uses of the box as a borrow of a part of an owned value does, so the box MUST NOT be assigned, moved or dropped while its contents are borrowed. A borrow of the contents of a by-value box MUST NOT be returned.

#### Scenario: Using a box while its contents are borrowed mutably
- **WHEN** `struct P { var x: Int }` is declared and `fun f(h: Heap<P>): Int { var m = h; val q = &mut m.x; val z = m.x; *q = 1; return z }` is checked
- **THEN** the borrow checker emits `E5003` at `m.x` in `val z = m.x`

#### Scenario: Replacing or moving a box while its contents are borrowed
- **WHEN** `struct P { var x: Int }` and `fun take(h: Heap<P>) {}` are declared, and `fun f(h: Heap<P>): Int { var m = h; val r = &m.x; m = Heap(P(2)); return r }` and `fun g(h: Heap<P>): Int { val r = &h.x; take(h); return r }` are checked
- **THEN** the borrow checker emits `E5004` for the assignment to `m` and `E5005` for the move of `h`

#### Scenario: Returning a borrow of a by-value box's contents
- **WHEN** `struct P { var x: Int }` is declared and `fun f(h: Heap<P>): &Int = &h.x` and `fun g(h: &Heap<P>): &Int = &h.x` are checked
- **THEN** the borrow checker emits `E5008` for `f`, a borrow of the parameter `h`, and accepts `g`, whose result borrows what `h` refers to

## MODIFIED Requirements

### Requirement: Copy-Out From References
Where a value of type `T` is expected and the expression has a reference or box type (`&T`, `&mut T`, `Heap<T>` or nested borrows and boxes of `T`), a `Copy` value MUST be copied out implicitly, at every value position: initializers with a type, assigned values, returned values, by-value arguments, constructor fields and payloads, and default values. `(&T)?` MUST be accepted where `T?` is expected when `T` is `Copy`. An unannotated initializer MUST keep the reference or box type.

#### Scenario: Copy-out of a borrowed primitive
- **WHEN** `fun g(x: Int): Int` is declared and `fun f(a: &Int): Int = g(a)` is checked
- **THEN** the call typechecks, copying the `Int` out of the borrow

#### Scenario: Copy-out at every value position
- **WHEN** `struct Point : Copy { val x: Int }` and `struct Line { val start: Point }` are declared and a body of `fun f(a: &Int, p: &mut Point): Point` runs `val x: Int = a`, `var y = 0`, `y = a`, `val l = Line(p)` and `return p`
- **THEN** the body typechecks successfully

#### Scenario: Copy-out through nested and nullable borrows
- **WHEN** `fun f(a: &&Int): Int = a` and `fun g(a: (&Int)?): Int? = a` are checked
- **THEN** both typecheck successfully

#### Scenario: An unannotated initializer copies the borrow
- **WHEN** `fun f(a: &Int): &Int { val b = a; return b }` is checked
- **THEN** the file typechecks successfully, and `b` has type `&Int`

#### Scenario: Copy-out of a box's contents
- **WHEN** `struct Point : Copy { val x: Int }` is declared and `fun f(h: Heap<Int>, r: &Heap<Point>): Int { val n: Int = h; val p: Point = r; return h }` and `fun g(h: Heap<Int>): Heap<Int> { val k = h; return k }` are checked
- **THEN** both typecheck successfully, `h` stays usable in `f` after each copy, and `k` has type `Heap<Int>`, a move of the box

### Requirement: Writing Through a Mutable Reference
`*r = v` MUST write `v` into the place `r` refers to, and `*h = v` into the contents of the box `h`; each `*` peels exactly one reference or box layer. Every reference layer written through MUST be `&mut`, and every box layer MUST be a mutable place, or the assignment is rejected with `E4003`. A `*` applied to a value that is neither a reference nor a box MUST be rejected with `NotAReference` (`E4019`). The value MUST be checked against the referent type in a consuming position, and the binding holding a reference MAY be a `val`.

#### Scenario: Writing through a mutable borrow
- **WHEN** `fun bump(r: &mut Int) { *r = r + 1 }` is checked
- **THEN** it typechecks and writes through `r`; `r = 5` in the same body is rejected with `E4003`, because `r` is a `val` binding of reference type, with help suggesting `*r = 5`

#### Scenario: Writing through nested borrows
- **WHEN** `fun f(rr: &mut &mut Int) { **rr = 0 }` and `fun g(rr: &(&mut Int)) { **rr = 0 }` are checked
- **THEN** `f` typechecks successfully and the type checker emits `E4003` for `g`, because the outer borrow is shared

#### Scenario: Writing through a shared borrow rejected
- **WHEN** `fun f(r: &Int) { *r = 5 }` is checked
- **THEN** the type checker emits `E4003`, stating that a shared borrow cannot be written through

#### Scenario: Writing through a value that is not a reference
- **WHEN** `fun f() { var x = 1; *x = 5 }` is checked
- **THEN** the type checker emits `E4019` naming the type `Int`, saying that `*` needs a reference or a `Heap`

#### Scenario: The written value is checked and consumed
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(r: &mut String, a: &Int, n: &mut Int, s: String, u: User)` runs `*n = a`, `*r = s`, `*r = u.name` and `*n = "x"`
- **THEN** `*n = a` (a copy-out) and `*r = s` (a move of a whole parameter) are accepted, the type checker emits `E4017` for `u.name`, and it emits `E4002` for `"x"`

#### Scenario: Writing through the result of a call
- **WHEN** `fun pick(a: &mut Int, b: &mut Int): &mut Int` is declared and a body declares `var a = 1` and `var b = 2` and runs `*pick(&mut a, &mut b) = 0`
- **THEN** the body typechecks successfully

#### Scenario: Writing a box's contents
- **WHEN** `fun f(h: Heap<Int>, r: &mut Heap<Int>): Int { var m = h; *m = 2; *m += 1; **r = 3; *r = Heap(4); return m }` is checked
- **THEN** the body typechecks successfully: `*m = 2` and `*m += 1` write the contents of `m`, `**r = 3` the contents of the box `r` refers to, and `*r = Heap(4)` replaces that box

#### Scenario: Writing through a box that is not mutable rejected
- **WHEN** `struct Holder { val h: Heap<Int> }` is declared and `fun f(h: Heap<Int>) { *h = 1 }`, `fun g(r: &Heap<Int>) { **r = 1 }` and `fun k(o: Holder) { var p = o; *p.h = 1 }` are checked
- **THEN** the type checker emits `E4003` for each, with help suggesting a `var` binding for `f`, a `&mut` borrow for `g`, and a `var` field for `k`

#### Scenario: Assigning the contents' type to a box rejected
- **WHEN** `fun f(h: Heap<Int>) { var m = h; m = 5 }` is checked
- **THEN** the type checker emits `E4002`, with help suggesting `*m = 5`

### Requirement: Capturing Closures Do Not Escape
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT escape: being returned from a function, closure or default value, initializing a top-level `val`, being stored in a struct field, an enum payload or a box, being written through a reference, or being assigned to a captured variable MUST be rejected by the borrow checker with `EscapingClosure` (`E5007`). A closure without captures MAY escape. Passing any closure to a function-typed parameter is not one.

#### Scenario: Capturing closures do not escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }`, `fun k(p: () -> Int): () -> Int = p` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the borrow checker emits `E5007` for each: for `f` saying that a closure that captures `s` cannot escape and labelling where it is returned, for `g`, `h` and `m` likewise, and for `k` saying that the function-typed parameter `p` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

#### Scenario: Boxing a capturing closure
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(s: String): Int { val h = Heap({ greet(s) }); return 1 }` and `fun g(): Heap<() -> Int> = Heap({ 2 })` are checked
- **THEN** the borrow checker emits `E5007` for `f`, labelling the closure "stored in a box here", and accepts `g`
