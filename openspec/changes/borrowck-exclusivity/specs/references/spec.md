# References Specification Delta

## ADDED Requirements

### Requirement: Shared Borrows Coexist
Any number of shared borrows of a value MUST be allowed to be live at the same time, together with reads of the value through its owner.

#### Scenario: Concurrent shared borrows
- **WHEN** `fun both(a: &String, b: &String): Int = 1` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Int { val r = &s; val t = &s; greet(s); return both(r, t) + both(s, s) }` is checked
- **THEN** the file passes `prism check`

### Requirement: A Mutable Borrow Is Exclusive
While a mutable borrow of a place is live, the borrow checker MUST reject any other borrow of an overlapping place and any read of it through its owner with `BorrowConflict` (`E5003`). While a shared borrow is live, it MUST reject a mutable borrow of an overlapping place with `E5003`. A closure capture and the implicit borrow of an argument or a receiver are borrows.

#### Scenario: Exclusive mutable borrow prevents concurrent access
- **WHEN** `fun print(n: &Int) {}` and `fun bump(r: &mut Int) {}` are declared and a body declares `var x = 1` and then runs `val m = &mut x; val k = &mut x; print(m)`, or `val m = &mut x; print(x); print(m)`, or `val r = &x; bump(&mut x); print(r)`
- **THEN** the borrow checker emits `E5003` at the second `&mut x`, at `x` in `print(x)`, and at `&mut x` in `bump(&mut x)`

#### Scenario: A mutable receiver conflicts with a live shared borrow
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}` and `fun print(n: &Int) {}` are declared and a body runs `var v = Stack(0); val c = &v.count; v.push(1); print(c)`
- **THEN** the borrow checker emits `E5003` at the receiver `v` of `push`

#### Scenario: Two mutable borrows passed to one call
- **WHEN** `struct User { var name: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared and a body runs `var u = User("a"); touch(&mut u, &mut u.name)`
- **THEN** the borrow checker emits `E5003` at `&mut u.name`

### Requirement: Borrowed Values Cannot Be Assigned or Moved
While a borrow of a place is live, the borrow checker MUST reject an assignment to an overlapping place with `AssignToBorrowed` (`E5004`), and a move of an overlapping place with `MoveOutWhileBorrowed` (`E5005`). Re-pointing a reference variable MUST NOT count as an assignment to what it refers to.

#### Scenario: Assigning a borrowed value
- **WHEN** `struct User { var name: String }`, `fun print(n: &Int) {}` and `fun greet(s: &String): Int = 1` are declared, and a body runs `var x = 1; val r = &x; x = 2; print(r)`, or `var u = User("a"); val n = &u.name; u.name = "b"; greet(n)`, or `var w = User("a"); val v = &w; w.name = "b"; greet(v.name)`
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`, at `u.name` in the assignment, and at `w.name` in the assignment

#### Scenario: Moving a borrowed value
- **WHEN** `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String) { val r = &s; take(s); greet(r) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`, with help suggesting `s.clone()`

### Requirement: Borrows Last Until Their Last Use
A borrow MUST be live from where it is created until the last use of every reference that holds it, on each path, and not until the end of its block. A use in a later iteration of a loop MUST keep the borrow live around the loop.

#### Scenario: Borrow ends at its last use
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; val r = &x; print(r); x = 2 }` is checked
- **THEN** the file passes `prism check`, because `r` is not used after `print(r)`

#### Scenario: A borrow used on one path only
- **WHEN** `fun print(n: &Int) {}` is declared, and `fun f(c: Bool) { var x = 1; val r = &x; if (c) print(r); x = 2 }` and `fun g(c: Bool) { var x = 1; val r = &x; if (c) x = 2; print(r) }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5004` at `x` in `x = 2` of `g`

#### Scenario: A borrow used in a later iteration
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var x = 1; val r = &x; while (c) { print(r); x = 2 } }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`, whose borrow is later used by `print(r)` in a later iteration of the loop

### Requirement: References Carry Their Borrows
A reference copied into another local, wrapped as a `(&T)?`, borrowed itself, reborrowed, or borrowed through for one of its referent's fields MUST keep the borrows it holds live for as long as the new reference is live. A value written through a reference MUST become held by what the reference points at. Re-pointing a `var` reference MUST end the borrows it held, from that point on.

#### Scenario: A copied reference keeps the borrow
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; val r = &x; val r2 = r; x = 2; print(r2) }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`

#### Scenario: Re-pointing a reference variable ends its old borrow
- **WHEN** `fun print(n: &Int) {}` is declared, and `fun f() { var a = 1; var b = 2; var r: &Int = &a; r = &b; a = 3; print(r) }` and `fun g() { var a = 1; var b = 2; var r: &Int = &a; r = &b; b = 3; print(r) }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5004` at `b` in `b = 3` of `g`

#### Scenario: Re-pointing a mutable reference at the value it borrowed
- **WHEN** `fun bump(r: &mut Int) {}` is declared, and `fun f() { var x = 1; var r: &mut Int = &mut x; bump(r); r = &mut x; bump(r) }` and `fun g(c: Bool) { var x = 1; var r: &mut Int = &mut x; while (c) { bump(r); r = &mut x } }` are checked
- **THEN** the file passes `prism check`, because the new borrow of `x` replaces the old one, which `r` no longer holds once it is assigned

#### Scenario: A reborrow keeps its source borrowed
- **WHEN** `struct User { var name: String }` is declared and a body runs `var u = User("a"); val m = &mut u; val n = &mut m.name; u.name = "b"; *n = "c"`
- **THEN** the borrow checker emits `E5004` at `u.name`, because `n` borrows through `m`, which borrows `u`

#### Scenario: References of references
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; var r = &x; val rr = &r; x = 2; print(rr) }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`

#### Scenario: Writing a borrow through a reference
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var a = 1; var b = 2; var r: &Int = &b; val rr = &mut r; *rr = &a; a = 3; print(r) }` is checked
- **THEN** the borrow checker emits `E5004` at `a` in `a = 3`, because `r` now refers to `a`

#### Scenario: An optional borrow keeps the borrow
- **WHEN** `struct Profile { var nick: String? }` and `fun show(n: (&String)?) {}` are declared and a body runs `var p = Profile(null); val a: (&String)? = &p.nick; p.nick = null; show(a)`
- **THEN** the borrow checker emits `E5004` at `p.nick` in `p.nick = null`

### Requirement: Calls and Closures Write Borrows Through What They Are Given
A call MUST be assumed to write the borrows of its other arguments through each `&mut R` or `(&mut R)?` argument whose `R` holds borrows, through each function-typed argument and through a called function value, and a closure the borrows of its other captures through each variable it captures mutably. What these point at MUST hold those borrows afterwards, however the reference reached them.

#### Scenario: Writing a borrow through a closure
- **WHEN** `fun print(n: &Int) {}`, `fun run(f: () -> Int): Int = f()` and `fun apply(g: (&Int) -> Int, v: &Int): Int = g(v)` are declared, and `fun f(c: Bool) { var b = 2; var r: &Int = &b; val rr = &mut r; if (c) { val a = 1; run { *rr = &a; 1 } }; print(r) }` and `fun g(c: Bool) { var b = 2; var r: &Int = &b; if (c) { val a = 1; apply({ v -> r = v; 1 }, &a) }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` for `a` in `f`, because the closure may write a borrow of `a` into `r` through `rr`, and in `g`, because `apply` may pass `&a` to the closure that writes `r`

#### Scenario: Writing a borrow through a mutable reference argument
- **WHEN** `fun print(n: &Int) {}`, `fun h(s: &mut &Int, v: &Int) {}` and `fun set(s: (&mut &Int)?, v: &Int) {}` are declared, and `fun g(s: &mut &Int) { val n = 1; val t = s; h(t, &n) }` and `fun f(c: Bool) { var b = 2; var r: &Int = &b; if (c) { val a = 1; set(&mut r, &a) }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` for `n` in `g`, because `h` may write `&n` into the caller's reference that `s` points at, and for `a` in `f`, because `set` may write `&a` into `r`

### Requirement: Disjoint Fields Are Borrowed Independently
Borrows of different fields of one value MUST NOT conflict, whether the value is a local or is reached through a reference. A borrow of the value itself, or of a field, MUST conflict with a borrow of any place inside it.

#### Scenario: Disjoint field borrows
- **WHEN** `struct User { var name: String }`, `struct Order { var user: User, var email: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared, and `fun f(o: &mut Order) { touch(&mut o.user, &mut o.email) }` and `fun g(o: Order) { var p = o; val a = &mut p.user; val b = &mut p.email; touch(a, b) }` are checked
- **THEN** the file passes `prism check`

#### Scenario: A borrow of the whole value conflicts with a field borrow
- **WHEN** `struct User { var name: String }`, `struct Order { var user: User, var email: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared and a body runs `var p = Order(User("a"), "b"); val a = &mut p.user; val b = &p; touch(a, &mut p.email)`
- **THEN** the borrow checker emits `E5003` at `&p`, and not at `&mut p.email`

### Requirement: Mutable Receivers Are Borrowed After the Arguments
The mutable borrow of a `mut fun` call's receiver MUST start when the call is made, after its arguments are evaluated, so arguments that read the receiver and end before the call MUST NOT conflict with it. A borrow that an argument still holds when the call is made MUST conflict with it.

#### Scenario: Two-phase receiver borrows
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}`, `mut fun Stack.pushRef(n: &Int) {}` and `fun Stack.size(): Int = count` are declared, and a body runs `var v = Stack(0); v.push(v.size()); v.push(v.count)`, or `var w = Stack(0); w.pushRef(&w.count)`
- **THEN** the borrow checker accepts `v.push(v.size())` and `v.push(v.count)`, and emits `E5003` at the receiver `w` of `pushRef`, because `&w.count` is still held by the call

### Requirement: Safe-Call Receivers Are Borrowed at Their Test
A receiver that a `?.` test reaches (the payload of a nullable), shared or mutable, MUST be borrowed right after its test, before the arguments, so an argument that changes or mutably borrows the tested value MUST conflict with that borrow, as MUST one that reads it while the receiver is borrowed mutably.

#### Scenario: A safe-call receiver is borrowed at its test
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}`, `fun Stack.show(n: Int) {}` and `fun clear(s: &mut Stack?): Int { *s = null; return 1 }` are declared, and a body runs `var n: Stack? = Stack(0); n?.push(clear(&mut n))`, or `var m: Stack? = Stack(0); m?.show(clear(&mut m))`
- **THEN** the borrow checker emits `E5003` at `&mut n` and at `&mut m`, because the receiver is borrowed from the test of its tag, which `clear` could otherwise make `null` before the call

### Requirement: Calls Returning References Keep Their Inputs Borrowed
The result of a call whose type contains a reference MUST hold every borrow passed to the call's input borrows (its parameters whose type contains a reference, other than function types), each in its own mode, so a `&mut` argument MUST stay frozen while the result is live. The result of calling a function value MUST also hold the borrows of the function value. The borrows a default value's generator takes of earlier by-value arguments MUST NOT be held by its result.

#### Scenario: A returned borrow keeps every input borrowed
- **WHEN** `fun first(a: &String, b: &String): &String = a`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared, and `fun f(x: String, y: String) { val r = first(&x, &y); take(y); greet(r) }` and `fun g(x: String, y: String) { val r = first(&x, &y); greet(r); take(y) }` are checked
- **THEN** the borrow checker emits `E5005` at `y` in `take(y)` of `f`, and accepts `g`

#### Scenario: A mutable input stays frozen while the result is live
- **WHEN** `fun pick(a: &mut Int): &Int = a` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val r = pick(&mut x); print(x); print(r) }` is checked
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`

#### Scenario: A method result keeps its receiver borrowed
- **WHEN** `struct Team { var lead: String }`, `fun Team.captain(): &String = &lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val c = t.captain(); t.lead = "b"; greet(c)`
- **THEN** the borrow checker emits `E5004` at `t.lead`

#### Scenario: An omitted reference default after a by-value parameter
- **WHEN** `fun greet(a: String, n: &String = "anon"): Int = 1` is declared and `fun f(s: String): Int = greet(s)` is checked
- **THEN** the file passes `prism check`

### Requirement: Borrowed Values Outlive Their Borrows
A local or temporary MUST NOT be dropped, or end its storage, while a borrow of it is live. Such a drop MUST be rejected by the borrow checker with `DroppedWhileBorrowed` (`E5006`), pointing at the borrow, with a related location where the value ends.

#### Scenario: Borrow of a local that goes out of scope
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, saying that `n` does not live long enough, with a related location at the closing brace of the `if` branch

#### Scenario: Borrow of a temporary that is dropped
- **WHEN** `fun make(): String = "x"`, `fun first(a: &String, b: &String): &String = a` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String) { var r: &String = &s; r = &make(); greet(r) }` and `fun g(s: String) { val r = first(&make(), &s); greet(r) }` are checked
- **THEN** the borrow checker emits `E5006` at `&make()` in `f` and in `g`, saying that a temporary value is dropped while borrowed, with help suggesting to bind the value to a local first

#### Scenario: Borrowed temporaries that are extended
- **WHEN** `fun make(): String = "x"` and `fun greet(s: &String): Int = 1` are declared, and `fun f() { val r = &make(); greet(r) }`, `fun g(c: Bool, s: String) { val r = if (c) &make() else &s; greet(r) }` and `fun h(n: String?): Int = greet(&n ?: &"anon")` are checked
- **THEN** the file passes `prism check`, because each temporary lives as long as `r`, or until the end of the statement that borrows it

#### Scenario: A borrow of a loop's local kept for the next iteration
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var a = 1; var r: &Int = &a; while (c) { val n = 1; print(r); r = &n } }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, with the later use `print(r)` labelled as in a later iteration of the loop, and nothing for the assignment of the next iteration's `n`

#### Scenario: A closure stores a borrow of its own local in a capture
- **WHEN** `fun print(n: &Int) {}` and `fun run(f: () -> Int): Int = f()` are declared and `fun f() { var a = 1; var r: &Int = &a; run { val n = 2; r = &n; 1 }; print(r) }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, because the closure's local `n` ends while `r` refers to it

### Requirement: Closures Capture by Inferred Borrow
A closure MUST borrow each variable it captures: mutably when its body assigns the variable or one of its fields, writes through it, borrows it or a part of it mutably, or contains a closure that captures it mutably, and shared otherwise. The borrows MUST start where the closure is created and last while the closure value is live. Conflicting uses of a captured variable meanwhile MUST be rejected as for any borrow.

#### Scenario: Closures capture by inferred borrow
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and a body declares `var u = User("a")` and runs `val f = { u.name = "z"; 1 }; greet(u.name); run(f)`, or `val g = { greet(u.name) }; greet(u.name); run(g)`, or `val h = { greet(u.name) }; u.name = "b"; run(h)`, or `val k = { u.name = "z"; 1 }; run(k); u.name = "b"`
- **THEN** the borrow checker emits `E5003` at `u.name` in `greet(u.name)` after `f`, accepts `g`, emits `E5004` at `u.name` in the assignment after `h`, and accepts `k`, whose borrow ends when `run(k)` returns

#### Scenario: Moving a captured variable while the closure is live
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { val g = { greet(s) }; take(s); run(g) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`

#### Scenario: A nested closure makes the capture mutable
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var x = 1; val f = { run { x = 2; 1 } }; print(x); run(f)`
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`, because the outer closure borrows `x` mutably for the inner one

### Requirement: Capturing Closures Do Not Escape
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT escape: being returned from a function, closure or default value, initializing a top-level `val`, being stored in a struct field or enum payload, being written through a reference, or being assigned to a captured variable MUST be rejected by the borrow checker with `EscapingClosure` (`E5007`). A closure without captures MAY escape, and passing any closure to a call is not an escape.

#### Scenario: Capturing closures do not escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }`, `fun k(p: () -> Int): () -> Int = p` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the borrow checker emits `E5007` for each: for `f` saying that a closure that captures `s` cannot escape and labelling where it is returned, for `g`, `h` and `m` likewise, and for `k` saying that the function-typed parameter `p` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

### Requirement: Borrow Conflict Diagnostics Show the Borrow and Its Later Use
A diagnostic of `E5003`, `E5004` or `E5005` MUST point at the conflicting access and name the place, and MUST list as related locations where the conflicting borrow was created and where it is used later, with ", in later iteration of loop" when that use is reached only through a loop's back edge. A borrow made by a closure MUST also label the use inside the closure that decided how it is captured.

#### Scenario: Conflict points at the borrow and its later use
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f() { var x = 1; val r = &x; x = 2; print(r) }`
- **THEN** it reports `error[E5004]: cannot assign to 'x' because it is borrowed` at `x` in `x = 2`, labelled "'x' is assigned here while borrowed", with a related location at `&x` labelled "'x' is borrowed here" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Closure borrow labels
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `prism check` runs on `fun f() { var u = User("a"); val g = { u.name = "z"; 1 }; greet(u.name); run(g) }`
- **THEN** it reports `error[E5003]: cannot borrow 'u.name' as shared because 'u' is borrowed as mutable` at `u.name` in `greet(u.name)`, labelled "shared borrow occurs here", with related locations at the closure labelled "closure borrows 'u' mutably here", at `u.name` inside the closure labelled "borrow occurs due to this use of 'u' in the closure", and at `g` in `run(g)` labelled "mutable borrow later used here"

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Escaping closure labels
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism check` runs on `fun f(s: String): () -> Int = { greet(s) }`
- **THEN** it reports `error[E5007]: a closure that captures 's' cannot escape` at the closure labelled "returned here", with a related location at `s` inside the closure labelled "'s' is captured here", a note that a closure borrows what it captures, and help suggesting to pass `s` to the closure as a parameter
