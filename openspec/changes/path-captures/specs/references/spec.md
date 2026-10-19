## ADDED Requirements

### Requirement: Closures Capture Paths
A closure MUST capture, for each use of a binding of an enclosing scope, the path that use reads: the binding, then each field access and `Heap<T>` dereference the use goes through, and a first dereference of the binding itself when it is a `val` reference. A path MUST stop at an index, a safe navigation, a state's `value`, a call and any other dereference of a reference. When one captured path is a prefix of another, the closure MUST capture the prefix only.

#### Scenario: A closure captures the fields it uses
- **WHEN** `struct P { var x: Int, var y: Int }` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(): Int { var p = P(1, 2); return run { p.x + p.y + p.x } }`
- **THEN** the closure captures the paths `p.x` and `p.y`, each once, both borrowed, and not the binding `p`

#### Scenario: A use of the whole binding subsumes its paths
- **WHEN** `struct P { var x: Int, var y: Int }`, `struct Q { val p: P, val n: Int }`, `fun size(p: &P): Int = p.x` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(q: Q): Int = run { q.p.x + size(q.p) + q.n }` and `fun g(q: Q): Int = run { q.n + size(q.p) + q.p.y + q.p.x }`
- **THEN** each closure captures the paths `q.p` and `q.n`, in the order of the fields of `Q` whatever the order of the uses

#### Scenario: Paths go through references and boxes
- **WHEN** `struct Todo { val id: Int, val title: String }`, `struct P { var x: Int, var y: Int }`, `struct Cell { val inner: Heap<P> }` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(t: &Todo, c: &Cell): Int = run { t.id + c.inner.x }`
- **THEN** the closure captures the paths `t.id` and `c.inner.x`, the first through the reference `t` and the second through the reference `c` and the box `c.inner`

#### Scenario: Paths stop at indices, safe navigations and other references
- **WHEN** `struct P { var x: Int, var y: Int }`, `struct Bag { val items: List<Int>, val tag: P? }` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(b: &Bag, i: Int, rr: &&P): Int = run { b.items[i] + (b.tag?.x ?: 0) + rr.x }`
- **THEN** the closure captures the paths `b.items`, `b.tag`, `i` and `*rr`, which ends before the dereference of the reference `*rr` holds

#### Scenario: A path does not go through a var reference
- **WHEN** `struct Todo { val id: Int, val title: String }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(a: &Todo, b: &Todo): Int { var r = a; val g = { r.id }; r = b; return run(g) }` is checked
- **THEN** the borrow checker emits `E5004` at the assignment `r = b`, because the closure captures the binding `r`, whose later re-pointing it would see

### Requirement: A Binding's Captured Paths Follow Its Fields
A closure's captured paths of one binding MUST be ordered by their steps, fields in declaration order, whatever the order of their uses, so that reordering the uses of one binding inside a closure changes neither its captures nor its environment.

#### Scenario: Reordered uses keep the captures
- **WHEN** `struct P { var x: Int, var y: Int }` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(): Int { var p = P(1, 2); return run { p.y + p.x } }`
- **THEN** the closure captures the paths `p.x` then `p.y`, both borrowed

## MODIFIED Requirements

### Requirement: Closures Capture by Inferred Borrow
A closure MUST borrow each path it captures that it does not own (see "Closures That Escape Own Their Captures" and "Closures That Outlive a Captured Binding Own It"): mutably when its body assigns it or a place within it, writes through it, borrows it or a part of it mutably, or holds a closure capturing it mutably, and shared otherwise. The borrows MUST last from the closure's creation while its value is live, and conflicting uses of an overlapping place meanwhile MUST be rejected as for any borrow.

#### Scenario: Closures capture by inferred borrow
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and a body declares `var u = User("a")` and runs `val f = { u.name = "z"; 1 }; greet(u.name); run(f)`, or `val g = { greet(u.name) }; greet(u.name); run(g)`, or `val h = { greet(u.name) }; u.name = "b"; run(h)`, or `val k = { u.name = "z"; 1 }; run(k); u.name = "b"`
- **THEN** the borrow checker emits `E5003` at `u.name` in `greet(u.name)` after `f`, accepts `g`, emits `E5004` at `u.name` in the assignment after `h`, and accepts `k`, whose borrow ends when `run(k)` returns

#### Scenario: Moving a captured variable while the closure is live
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { val g = { greet(s) }; take(s); run(g) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`

#### Scenario: A nested closure makes the capture mutable
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var x = 1; val f = { run { x = 2; 1 } }; print(x); run(f)`
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`, because the outer closure borrows `x` mutably for the inner one

#### Scenario: A closure borrowing a field leaves the others free
- **WHEN** `struct User { var name: String }`, `struct Order { var user: User, var email: String }`, `fun greet(s: &String): Int = 1`, `fun show(u: &User): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and a body declares `var o = Order(User("a"), "b")` and runs `val f = { o.email = "z"; 1 }; greet(o.user.name); o.user.name = "c"; run(f)`, or `val g = { o.email = "z"; 1 }; greet(o.email); run(g)`, or `val h = { o.user.name = "z"; 1 }; show(o.user); run(h)`
- **THEN** the borrow checker accepts `f`, whose closure borrows only `o.email`, emits `E5003` at `o.email` in `greet(o.email)` after `g`, and emits `E5003` at `o.user` in `show(o.user)` after `h`, because `o.user` contains the captured `o.user.name`

#### Scenario: A closure writing through a mutable reference leaves the other fields free
- **WHEN** `struct P { var x: Int, var y: Int }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(r: &mut P): Int { val g = { r.x += 1; r.x }; r.y = 5; return run(g) }` is checked
- **THEN** the file passes `prism check`, because the closure borrows `r.x` mutably and the assignment writes `r.y`

### Requirement: Closures That Escape Own Their Captures
A closure whose value may escape MUST own every path it captures, copying it or owning its binding (see `ownership`, "Owning Closures Copy the Paths They Can"). A value escapes when it is returned from a function, closure or default value, initializes a top-level `val`, is stored in a struct field, an enum payload or a box, is written through a reference or to a field, is assigned to a captured variable, is passed for a generic parameter or an `escaping` parameter, or is captured by value by another closure, also after flowing through locals, nullables and branches.

#### Scenario: Escaping closures own their captures
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the file passes `prism check`, and each closure owns what it captures

#### Scenario: A closure bound to a local first
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Button { val h = { greet(s) }; val k: (() -> Int)? = h; return Button(k ?: { 0 }) }` is checked
- **THEN** the file passes `prism check`, and the closure `{ greet(s) }` owns `s`, because its value reaches the field through `h`, the nullable `k` and the `?:`

#### Scenario: Boxed and generic closures own their captures
- **WHEN** `fun <T> keep(x: T): T = x`, `struct Box<T>(val v: T)` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): Int { val g = keep({ greet(s) }); return g() }`, `fun h(s: String): Int { val b = Heap({ greet(s) }); return 1 }`, `fun k(s: String): Box<() -> Int> = Box({ greet(s) })`, `fun m(): Heap<() -> Int> = Heap({ 2 })` and `fun n(): Int = keep({ 1 })()` are checked
- **THEN** the file passes `prism check`, the closures of `f`, `h` and `k` own `s`, and those of `m` and `n` capture nothing

#### Scenario: A nested closure captured by an escaping one
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(s: String): () -> Int { val inner = { greet(s) }; return { inner() + 1 } }` is checked
- **THEN** the file passes `prism check`: the returned closure owns `inner`, so `inner` escapes and owns `s`

#### Scenario: A closure passed to an unmarked parameter keeps borrowing
- **WHEN** `fun <T> remember(calc: () -> T): T = calc()`, `fun run(f: () -> Int): Int = f()`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String) { run { greet(s) }; remember { greet(s) }; take(s) }` is checked
- **THEN** the file passes `prism check`, because neither closure escapes, so both borrow `s`

#### Scenario: An escaping closure copies the field it reads
- **WHEN** `struct Todo { val id: Int, val title: String }` and `struct Button { val onClick: () -> Int }` are declared and `fun f(t: &Todo): Button = Button({ t.id * 2 })` is checked
- **THEN** the file passes `prism check`, and the closure owns a copy of `t.id`

### Requirement: Escaping Function Values Hold No Borrows
A value whose type contains a function type MUST NOT escape when it may hold a borrow: a closure owning a reference, a borrowing closure or an unmarked function-typed parameter, or such a parameter itself. The borrow checker MUST reject it with `EscapingClosure` (`E5007`). A closure without captures, or owning only values that hold no borrow, such as copies of paths read through references, MAY escape.

#### Scenario: An escaping closure capturing a reference
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared and `fun f(r: &String): () -> Int = { greet(r) }` and `fun g(r: &String): Button = Button({ greet(r) + 1 })` are checked
- **THEN** the borrow checker emits `E5007` for each, saying that a closure that captures the reference `r` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

#### Scenario: Escaping through an escaping parameter
- **WHEN** `fun store(f: escaping () -> Int): Int = 1` and `fun greet(s: &String): Int = 1` are declared, and `fun f(r: &String): Int = store { greet(r) }` and `fun g(p: () -> Int): Int = store(p)` are checked
- **THEN** the borrow checker emits `E5007` at the closure in `f`, labelled "passed to an escaping parameter here", and at `p` in `g`

#### Scenario: An escaping closure copying through a reference
- **WHEN** `struct Todo { val id: Int, val title: String }` and `struct Button { val onClick: () -> Int }` are declared, and `fun f(r: &Int): () -> Int = { r }`, `fun g(t: &Todo): Button = Button({ t.id + 1 })` and `fun h(t: &Todo): () -> Int = { t.title.length }` are checked
- **THEN** the borrow checker accepts `f` and `g`, whose closures copy an `Int` read through the reference, and emits `E5007` for `h`, whose closure owns the reference `t` because `t.title` is not `Copy`

#### Scenario: Paths that a closure cannot copy keep the reference
- **WHEN** `struct Todo { val id: Int, val title: String }` and `struct Counter { var n: Int }` are declared and `fun f(c: &Counter): () -> Int = { c.n }` and `fun g(t: &mut Todo): () -> Int = { t.id }` are checked
- **THEN** the borrow checker emits `E5007` for each, because `n` is a `var` field read through a reference and `t` is a mutable reference, so each closure owns the reference itself

### Requirement: Borrow Conflict Diagnostics Show the Borrow and Its Later Use
A diagnostic of `E5003`, `E5004` or `E5005` MUST point at the conflicting access and name the place, and MUST list as related locations where the conflicting borrow was created and where it is used later, with ", in later iteration of loop" when that use is reached only through a loop's back edge. A borrow made by a closure MUST also label the use inside the closure that decided how it is captured, naming the path the closure captures.

#### Scenario: Conflict points at the borrow and its later use
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f() { var x = 1; val r = &x; x = 2; print(r) }`
- **THEN** it reports `error[E5004]: cannot assign to 'x' because it is borrowed` at `x` in `x = 2`, labelled "'x' is assigned here while borrowed", with a related location at `&x` labelled "'x' is borrowed here" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Closure borrow labels
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `prism check` runs on `fun f() { var u = User("a"); val g = { u.name = "z"; 1 }; greet(u.name); run(g) }`
- **THEN** it reports `error[E5003]: cannot borrow 'u.name' as shared because it is also borrowed as mutable` at `u.name` in `greet(u.name)`, labelled "shared borrow occurs here", with related locations at the closure labelled "closure borrows 'u.name' mutably here", at `u.name` inside the closure labelled "borrow occurs due to this use of 'u.name' in the closure", and at `g` in `run(g)` labelled "mutable borrow later used here"

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. A parameter that ends when its body returns MUST be labelled at its declaration, and a later use by the caller, through a `&mut R` parameter, at that parameter. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter that holds a borrow, and its help MUST name the field path a closure reads through a reference it owns.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Dropped parameter labels
- **WHEN** `prism check` runs on `fun keep(slot: &mut &String, v: String) { *slot = &v }`
- **THEN** it reports `error[E5006]: 'v' does not live long enough` at `&v` labelled "borrowed value does not live long enough", with a related location at the parameter `v` labelled "'v' is dropped when the function returns, while still borrowed" and one at the parameter `slot` labelled "borrow later used by the caller through 'slot'"

#### Scenario: Escaping closure labels
- **WHEN** `prism check` runs on `fun greet(s: &String): Int = 1` and `fun f(r: &String): () -> Int = { greet(r) }`
- **THEN** it reports `error[E5007]: a closure that captures the reference 'r' cannot escape` at the closure labelled "returned here", with a related location at `r` inside the closure labelled "'r' is captured here", a note that an escaping closure owns what it captures and a reference cannot be owned, and help suggesting to pass the value `r` refers to instead

#### Scenario: Escaping parameter labels
- **WHEN** `prism check` runs on `fun k(p: () -> Int): () -> Int = p`
- **THEN** it reports `error[E5007]: the parameter 'p' cannot escape` at `p` labelled "returned here", with a related location at the parameter labelled "'p' is a parameter here", a note that an unmarked function-typed parameter may borrow its caller's variables, and help suggesting `p: escaping () -> Int`

#### Scenario: Escaping closure help names the path
- **WHEN** `struct Todo { val id: Int, val title: String }` and `struct Counter { var n: Int }` are declared and `prism check` runs on `fun f(t: &Todo): () -> String = { t.title.clone() }` and `fun g(c: &Counter): () -> Int = { c.n }`
- **THEN** it reports `error[E5007]: a closure that captures the reference 't' cannot escape` with a related location at `t.title` labelled "'t' is captured here" and help "clone 't.title' into a 'val' before the closure and capture that instead: 'val title = t.title.clone()'", and for `g` the help "copy 'c.n' into a 'val' before the closure and capture that instead: 'val n = c.n'" with a note that a closure copies a field read through a reference only when the field is a `val`

### Requirement: Values Written Into a State Escape
A value written into a state, by `mutableStateOf`, `value =`, the result of `update`'s transform or a compound assignment, MUST be at an owning position: a closure written there owns its captures, and a function value that may hold a borrow is `E5007`, as for a generic parameter.

#### Scenario: A borrowing closure stored in a state rejected
- **WHEN** `fun f(s: MutableState<() -> Int>, r: &String) { s.value = { r.length + 1 } }` is checked
- **THEN** the borrow checker emits `E5007`, labelling the capture of `r`

#### Scenario: A handler stored in a state owns its captures
- **WHEN** `fun main() { val n = 2; val s = mutableStateOf({ 0 }); s.value = { n * 3 }; s.read { f -> println(f()) } }` runs with `--check-allocations`
- **THEN** stdout is `6`, and the exit status is 0
