# Ownership Specification Delta

## ADDED Requirements

### Requirement: Capturing by Value Moves the Binding
A closure that owns a capture (see `references`) MUST copy a `Copy` binding and move any other binding into itself where the closure is built. A later use of a moved binding MUST be rejected with `UseAfterMove` (`E5001`), labelling the closure as where the value moved and the reason the closure owns it. A copied binding MUST stay usable.

#### Scenario: Using a binding a closure took
- **WHEN** `struct Button { val onClick: () -> Int }`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Button { val b = Button({ greet(s) }); take(s); return b }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `take(s)`, with a related location at the closure labelled "value moved into closure here" and one at the closure's use as a field labelled "the closure owns its captures because it is stored in a field here", and help suggesting `val m = s.clone()` before the closure, capturing `m` instead

#### Scenario: A copied capture stays usable
- **WHEN** `struct Button { val onClick: () -> Int }` is declared and `fun f(n: Int): Button { val b = Button({ n + 1 }); val m = n * 2; return b }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Passing an owned function value moves it
- **WHEN** `fun store(f: escaping () -> Int): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(h: escaping () -> Int): Int { store(h); return h() }`, `fun g(h: escaping () -> Int): Int { run(h); return run(h) }` and `fun k(h: escaping () -> Int): Int { val a = run { h() }; return a + run { h() } }` are checked
- **THEN** the borrow checker emits `E5001` at `h` in `h()` of `f` and at the second `h` of `g`, and accepts `k`, whose closures borrow `h`

### Requirement: Captures by Value Are Values the Closure Can Take
A closure that owns a capture MUST capture only a `val`, and, for a non-`Copy` binding, only one its builder owns. Capturing by value a `var`, or a non-`Copy` capture of an enclosing closure or earlier parameter read in a default value, MUST be rejected by the type checker with `CannotCaptureByValue` (`E4028`), labelling the capture and why the closure owns it, with help suggesting to copy or clone it into a `val` first, or, for a `var` the closure writes, to keep the closure from escaping so that it borrows the `var`.

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

## MODIFIED Requirements

### Requirement: A Closure Does Not Move Its Captures
Inside a closure, a non-`Copy` variable that the closure captures MUST NOT be moved, because the closure may be called more than once, whether it borrows its captures (see `references`) or owns them. Such a move MUST be rejected by the type checker with `CannotMoveOut` (`E4017`), naming the source "captured variable `s`", with a note that a closure never gives away what it captures and help suggesting `.clone()` for a `Clone` type. A `Copy` capture MUST be copied, and a variable declared inside the closure MAY be moved.

#### Scenario: Moving a captured variable inside a closure
- **WHEN** `fun take(s: String) {}`, `fun run(f: () -> Int): Int = f()` and `struct Token { val id: Int }` are declared, and `fun f(s: String): Int = run { take(s); 1 }`, `fun g(t: Token): Int = run { val u = t; 1 }`, `fun h(s: String): Int = run { take(s.clone()); 1 }`, `fun k(n: Int): Int = run { val m = n; m }` and `fun l(): Int = run { val s = "a"; take(s); 1 }` are checked
- **THEN** the type checker emits `E4017` for `s` in `f`, naming the captured variable `s` and suggesting `s.clone()`, and for `t` in `g`, without a `.clone()` suggestion, and accepts `h`, `k` and `l`

#### Scenario: A closure in a default value moving an earlier parameter
- **WHEN** `fun take(s: String) {}` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(a: String, b: Int = run { take(a); 1 }): Int = b` is checked
- **THEN** the type checker emits `E4017` for `a`, naming the captured variable `a`

#### Scenario: Moving a captured variable after the closure ran
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { run { greet(s) }; take(s) }` is checked
- **THEN** the file passes `prism check`, because the closure's borrow of `s` ends when `run` returns

#### Scenario: An owning closure does not move its capture
- **WHEN** `fun take(s: String) {}` and `fun store(f: escaping () -> Int): Int = 1` are declared and `fun f(s: String): Int = store { take(s); 1 }` is checked
- **THEN** the type checker emits `E4017` for `s`, naming the captured variable `s` and noting that a closure never gives away what it captures
