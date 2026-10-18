# References Specification Delta

## ADDED Requirements

### Requirement: Closures That Escape Own Their Captures
A closure whose value may escape MUST own every binding it captures. A value escapes when it is returned from a function, closure or default value, initializes a top-level `val`, is stored in a struct field, an enum payload or a box, is written through a reference or to a field, is assigned to a captured variable, is passed for a generic parameter or an `escaping` parameter, or is captured by value by another closure, also after flowing through locals, nullables and branches.

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

### Requirement: Capture Modes Are Decided From the Source
The type checker MUST decide which captures a closure owns from the source, flow-insensitively, in time linear in the body: a closure stored in a local binding MUST own its captures when that binding, or a binding it flows into, is used at an escaping position anywhere in its body, whichever closure it holds there. The typed tree MUST record each capture's mode.

#### Scenario: A variable that may escape owns every closure stored in it
- **WHEN** `fun store(f: escaping () -> Int): Int = 1`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Int { var h = { greet(s) }; h(); h = { 0 }; store(h); take(s); return 1 }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `take(s)`, with a related location at the closure `{ greet(s) }` labelled "value moved into closure here" and one at `h` in `store(h)` labelled "the closure owns its captures because it is passed to an escaping parameter here"

### Requirement: Closures That Outlive a Captured Binding Own It
A closure stored, directly or through other bindings, in a binding declared outside the block that declares one of its captures MUST own that capture, and MUST keep borrowing its other captures unless its value escapes. The type checker MUST decide this from the blocks the bindings are declared in, whether or not the closure is used after the captured binding's block ends. A closure MUST NOT own a capture for any other reason, so a conflicting use of a borrowed capture stays a borrow error.

#### Scenario: A closure kept past a block
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(c: Bool): Int { var h: () -> Int = { 0 }; if (c) { val s = "a"; h = { greet(s) } }; return h() }` is checked
- **THEN** the file passes `prism check`, and the closure owns `s`

#### Scenario: Only the outlived capture is owned
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(): Int { var count = 0; var h: () -> Int = { 0 }; var i = 0; while (i < 3) { val s = "a"; h = { count += 1; greet(s) }; i += 1 }; h(); return count }` is checked
- **THEN** the file passes `prism check`: the closure owns `s`, which ends with each iteration, and borrows `count` mutably

#### Scenario: A closure stored outside a binding's block owns it
- **WHEN** `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(c: Bool): Int { var h: () -> Int = { 0 }; if (c) { val s = "a"; h = { greet(s) }; h(); take(s) }; return 1 }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `take(s)`, with a related location at the closure labelled "value moved into closure here" and one at the assignment to `h` labelled "the closure is stored here, in a variable that outlives 's'"

#### Scenario: A conflict does not make a closure own its capture
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { val g = { greet(s) }; take(s); run(g) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`, as it does for any closure that borrows `s`

### Requirement: Escaping Parameters Keep Function Values
A parameter declared `escaping` MUST hold a function value that holds no borrow: inside its function it MAY be stored, returned, boxed, passed to an `escaping` or generic parameter or captured by an escaping closure. Passing a value to it MUST make that value escape. An unmarked function-typed parameter MUST NOT escape, and MUST be rejected with `EscapingClosure` (`E5007`) when it does.

#### Scenario: Storing an escaping parameter
- **WHEN** `struct Button { val label: String, val onClick: () -> Unit }` is declared and `fun button(label: String, onClick: escaping () -> Unit): Button = Button(label, onClick)` and `fun later(f: escaping () -> Int): () -> Int = f` are checked
- **THEN** the file passes `prism check`

#### Scenario: An unmarked parameter cannot escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun <T> keep(x: T): T = x` are declared, and `fun k(p: () -> Int): () -> Int = p`, `fun g(p: () -> Int): Button = Button(p)`, `fun h(p: () -> Int): Int = keep(p)()` and `fun m(p: () -> Int): () -> Int = { p() }` are checked
- **THEN** the borrow checker emits `E5007` for each: for `k`, `g` and `h` saying that the parameter `p` cannot escape, and for `m` that a closure that captures the parameter `p` cannot escape, each with help suggesting to declare `p` as `escaping`

#### Scenario: A closure passed to an escaping parameter
- **WHEN** `fun store(f: escaping () -> Int): Int = 1` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Int = store { greet(s) }` is checked
- **THEN** the file passes `prism check`, and the closure owns `s`

### Requirement: Escaping Function Values Hold No Borrows
A value whose type contains a function type MUST NOT escape when it may hold a borrow: a closure owning a reference, a borrowing closure or an unmarked function-typed parameter, or such a parameter itself. The borrow checker MUST reject it with `EscapingClosure` (`E5007`). A closure without captures, or owning only values that hold no borrow, MAY escape.

#### Scenario: An escaping closure capturing a reference
- **WHEN** `struct Button { val onClick: () -> Int }` is declared and `fun f(r: &Int): () -> Int = { r }` and `fun g(r: &Int): Button = Button({ r + 1 })` are checked
- **THEN** the borrow checker emits `E5007` for each, saying that a closure that captures the reference `r` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

#### Scenario: Escaping through an escaping parameter
- **WHEN** `fun store(f: escaping () -> Int): Int = 1` is declared, and `fun f(r: &Int): Int = store { r }` and `fun g(p: () -> Int): Int = store(p)` are checked
- **THEN** the borrow checker emits `E5007` at the closure in `f`, labelled "passed to an escaping parameter here", and at `p` in `g`

### Requirement: A Closure Returns No Borrow of What It Owns
In a closure's body, a returned borrow of a capture the closure owns MUST be rejected with `DanglingBorrowReturn` (`E5008`), naming the capture as owned by the closure. A borrow of a capture the closure borrows follows "Function Types Elide by the Same Rules".

#### Scenario: Returning a borrow of an owned capture
- **WHEN** `fun f(s: String): () -> &String = { &s }` is checked
- **THEN** the borrow checker emits `E5008` at `&s` in the closure, saying that it cannot return a borrow of captured `s`, which the closure owns

### Requirement: Calling a Function Value Borrows It
A call of a function value MUST only read it: it MUST NOT move it, and MAY be made through a shared reference or a `val`, any number of times. Passing a function value as an argument or storing it MUST move it.

#### Scenario: Calling a stored handler through a borrow
- **WHEN** `struct Button { val onClick: () -> Int }` is declared and `fun fire(b: &Button): Int = b.onClick() + b.onClick()` and `fun twice(h: escaping () -> Int): Int { val a = h(); return a + h() }` are checked
- **THEN** the file passes `prism check`

## MODIFIED Requirements

### Requirement: Closures Capture by Inferred Borrow
A closure MUST borrow each variable it captures, unless it owns it (see "Closures That Escape Own Their Captures" and "Closures That Outlive a Captured Binding Own It"): mutably when its body assigns the variable or one of its fields, writes through it, borrows it or a part of it mutably, or contains a closure that captures it mutably, and shared otherwise. The borrows MUST start where the closure is created and last while the closure value is live. Conflicting uses of a captured variable meanwhile MUST be rejected as for any borrow.

#### Scenario: Closures capture by inferred borrow
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and a body declares `var u = User("a")` and runs `val f = { u.name = "z"; 1 }; greet(u.name); run(f)`, or `val g = { greet(u.name) }; greet(u.name); run(g)`, or `val h = { greet(u.name) }; u.name = "b"; run(h)`, or `val k = { u.name = "z"; 1 }; run(k); u.name = "b"`
- **THEN** the borrow checker emits `E5003` at `u.name` in `greet(u.name)` after `f`, accepts `g`, emits `E5004` at `u.name` in the assignment after `h`, and accepts `k`, whose borrow ends when `run(k)` returns

#### Scenario: Moving a captured variable while the closure is live
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { val g = { greet(s) }; take(s); run(g) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`

#### Scenario: A nested closure makes the capture mutable
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var x = 1; val f = { run { x = 2; 1 } }; print(x); run(f)`
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`, because the outer closure borrows `x` mutably for the inner one

### Requirement: Function Types Elide by the Same Rules
A function type MUST elide by rules 1, 3 and 4: a call of a function value MUST tie its result to every input-borrow argument, or, when its type has no input borrow, to the function value's own borrows. So a closure MUST return only borrows of top-level `val`s or derived from its reference parameters (`E5009` otherwise), or, when its type has no input borrow, also of what it captures by borrow; a borrow of its own local or temporary is `E5008`.

#### Scenario: A closure returning a borrow
- **WHEN** `fun app(g: (&Int) -> &Int): Int = 1` is declared and `fun f(): Int = app { k -> k }` and `fun h(): Int = app { k -> val m = 2; &m }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5008` at `&m` in the closure of `h`

#### Scenario: A closure may return a borrow of what it captures
- **WHEN** `fun greet(s: &String): Int = 1` is declared, and `fun f(s: String): Int { var t = s; val g = { &t }; val r = g(); t = "b"; return greet(r) }` and `fun h(s: String): Int { var t = s; val g = { &t }; val r = g(); greet(r); t = "b"; return 1 }` are checked
- **THEN** the borrow checker accepts both closures, emits `E5004` at `t` in `t = "b"` of `f`, because `r` keeps the closure's borrow of `t`, and accepts `h`

#### Scenario: A closure taking a borrowed parameter cannot return its captures
- **WHEN** `fun app(g: (&Int) -> &Int): Int = 1` is declared and `fun f(): Int { val n = 1; return app { k -> &n } }` is checked
- **THEN** the borrow checker emits `E5009` at `&n` in the closure, naming the captured `n`, because the result of a closure of type `(&Int) -> &Int` is tied to its parameter `k` only

#### Scenario: Returning a borrow obtained from a function-typed parameter
- **WHEN** `fun call(h: () -> &Int): &Int = h()` and `fun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)` are checked
- **THEN** the borrow checker emits `E5009` for `call`, naming the function-typed parameter `h`, since a caller does not keep what the closure passed for it borrows, and accepts `apply`, since the result of `g(v)` is tied to `v` by `g`'s type (elision rule 1) and `v` is `apply`'s single input borrow

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. A parameter that ends when its body returns MUST be labelled at its declaration, and a later use by the caller, through a `&mut R` parameter, at that parameter. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter that holds a borrow.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Dropped parameter labels
- **WHEN** `prism check` runs on `fun keep(slot: &mut &String, v: String) { *slot = &v }`
- **THEN** it reports `error[E5006]: 'v' does not live long enough` at `&v` labelled "borrowed value does not live long enough", with a related location at the parameter `v` labelled "'v' is dropped when the function returns, while still borrowed" and one at the parameter `slot` labelled "borrow later used by the caller through 'slot'"

#### Scenario: Escaping closure labels
- **WHEN** `prism check` runs on `fun f(r: &Int): () -> Int = { r }`
- **THEN** it reports `error[E5007]: a closure that captures the reference 'r' cannot escape` at the closure labelled "returned here", with a related location at `r` inside the closure labelled "'r' is captured here", a note that an escaping closure owns what it captures and a reference cannot be owned, and help suggesting to pass the value `r` refers to instead

#### Scenario: Escaping parameter labels
- **WHEN** `prism check` runs on `fun k(p: () -> Int): () -> Int = p`
- **THEN** it reports `error[E5007]: the parameter 'p' cannot escape` at `p` labelled "returned here", with a related location at the parameter labelled "'p' is a parameter here", a note that an unmarked function-typed parameter may borrow its caller's variables, and help suggesting `p: escaping () -> Int`

## REMOVED Requirements

### Requirement: Capturing Closures Do Not Escape
**Reason**: A capturing closure that escapes now owns its captures instead of being rejected; only escaping values that may hold a borrow are rejected.
**Migration**: "Closures That Escape Own Their Captures" lists the same escapes, boxes and top-level `val`s included, and its scenario "Boxed and generic closures own their captures" takes over "Boxing a capturing closure"; "Escaping Function Values Hold No Borrows" keeps "A closure without captures may escape" and, with "Escaping Parameters Keep Function Values", keeps `E5007` for values that may hold a borrow and for unmarked function-typed parameters.

### Requirement: Capturing Closures Are Not Passed to Generic Parameters
**Reason**: A capturing closure passed for a generic parameter now owns its captures, so a generic body may keep it.
**Migration**: "Closures That Escape Own Their Captures" makes a generic parameter an escape ("Boxed and generic closures own their captures", which also keeps `keep({ 1 })`, and "A closure passed to an unmarked parameter keeps borrowing", which keeps `remember { greet(s) }`), and "Escaping Function Values Hold No Borrows" and "An unmarked parameter cannot escape" reject a value that may hold a borrow there (`keep(p)`), keeping the borrow-free rule of `generics`.
