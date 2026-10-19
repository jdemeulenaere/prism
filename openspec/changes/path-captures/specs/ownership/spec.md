## ADDED Requirements

### Requirement: Owning Closures Copy the Paths They Can
A closure that owns its captures MUST copy each captured path that has projections, where the closure is built, when the path's type is `Copy`, its binding is a `val`, it dereferences no `&mut` reference, and every field it reads after dereferencing a reference is a `val` field. Otherwise the closure MUST own the path's whole binding, merged with the binding's other paths, by the rules for capturing a binding by value.

#### Scenario: Copying a field leaves its binding usable
- **WHEN** `struct User { val id: Int, val name: String }`, `struct Button { val onClick: () -> Int }` and `fun take(u: User) {}` are declared and `fun f(u: User): Button { val b = Button({ u.id + 1 }); take(u); return b }` is checked
- **THEN** the file passes `prism check`: the closure copies `u.id`, and `u` is moved only by `take(u)`

#### Scenario: A non-Copy field takes the whole binding
- **WHEN** `struct User { val id: Int, val name: String }`, `struct Button { val onClick: () -> Int }` and `fun take(u: User) {}` are declared and `fun f(u: User): Button { val b = Button({ u.name.length + u.id }); take(u); return b }` is checked
- **THEN** the borrow checker emits `E5001` at `u` in `take(u)`, with a related location at the closure labelled "value moved into closure here", because the closure owns the whole binding `u` for its path `u.name`, and `u.id` with it

#### Scenario: A var field of a val binding is copied
- **WHEN** `struct Counter { var n: Int, val label: String }` and `struct Button { val onClick: () -> Int }` are declared and `fun f(c: Counter): Button { val b = Button({ c.n }); println(c.label); return b }` is checked
- **THEN** the file passes `prism check`: no write can reach a field of the `val` binding `c`, so the closure copies `c.n` and `c` stays usable

## MODIFIED Requirements

### Requirement: Captures by Value Are Values the Closure Can Take
A closure that owns a capture MUST capture only a `val`, and, for a non-`Copy` binding, only one its builder owns. Capturing by value a `var`, or a non-`Copy` capture of an enclosing closure or earlier parameter read in a default value, MUST be rejected by the type checker with `CannotCaptureByValue` (`E4028`), labelling the capture and why the closure owns it, with help suggesting a fix that applies (a `val` copy or clone of the path the closure reads, or, for a written `var`, not escaping).

#### Scenario: Capturing a var by value
- **WHEN** `struct Button { val onClick: () -> Int }` is declared and `fun f(): Button { var n = 1; val b = Button({ n }); n = 2; return b }` is checked
- **THEN** the type checker emits `E4028` at `n` in the closure, saying that the variable `n` cannot be captured by value, with a related location at the closure's use as a field and help suggesting `val m = n` before the closure

#### Scenario: Capturing what the closure cannot take
- **WHEN** `fun store(f: escaping () -> Int): Int = 1`, `fun run(f: () -> Int): Int = f()` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): Int = run { store { greet(s) } }`, `fun g(a: String, p: () -> Int = { greet(a) }): Int = p()` and `fun h(n: Int): Int = run { store { n } }` are checked
- **THEN** the type checker emits `E4028` at `s` in `f`, noting that `s` is captured by the enclosing closure, and at `a` in `g`, noting that a default value only borrows earlier parameters, each with help suggesting to clone it into a `val` first, and accepts `h`, whose `n` is copied

#### Scenario: Capturing a written var by value
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(): Button { var n = 0; return Button({ n += 1; n }) }` and `fun h(): Button { var n = 0; return Button({ run { n += 1; n } }) }` are checked
- **THEN** the type checker emits `E4028` at `n` in each, with help saying that the closure writes `n` and a closure cannot write what it owns, and suggesting to keep it from escaping, rather than a `val` copy it could not write

#### Scenario: A borrowed var capture is unaffected
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun f(): Int { var n = 1; run { n += 1; n }; return n }` is checked
- **THEN** the file passes `prism check`, because the closure borrows `n`

#### Scenario: Capturing a field of a var by value
- **WHEN** `struct P { val x: Int }` and `struct Button { val onClick: () -> Int }` are declared and `fun f(): Button { var p = P(1); val b = Button({ p.x }); p = P(2); return b }` is checked
- **THEN** the type checker emits `E4028` at `p.x` in the closure, saying that the variable `p` cannot be captured by value, with help suggesting `val x = p.x` before the closure

#### Scenario: A field of an enclosing closure's capture is copied
- **WHEN** `struct User { val id: Int, val name: String }`, `fun store(f: escaping () -> Int): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(u: User): Int = run { store { u.id } }` is checked
- **THEN** the file passes `prism check`: the inner closure copies `u.id` from the outer closure's borrow of it, and gives nothing of `u` away
