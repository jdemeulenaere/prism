# References Specification Delta

## ADDED Requirements

### Requirement: The Elision Rules Tie a Returned Borrow to Inputs
Which inputs a returned borrow (a result whose type contains a reference outside a function type) comes from MUST follow from the signature alone, without lifetime annotations. Input borrows are the parameters whose type contains a reference outside a function type and the receiver of a non-`consuming` method. Rules 1 (one input borrow) and 3 (several) tie the result to every input borrow, rule 2 (a borrowed receiver) to the receiver only, and rule 4 (none) to nothing.

#### Scenario: Rule 1 - Single input borrow parameter
- **WHEN** `struct Team { var lead: String; var nick: String? }`, `fun nick(t: &Team): (&String)? = &t.nick`, `fun lead(t: &Team): &String = &t.lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a", null); val l = lead(&t); t.lead = "b"; greet(l)`
- **THEN** the borrow checker accepts `nick` and `lead`, and emits `E5004` at `t.lead` in the assignment, because the result of `lead` is tied to its argument

#### Scenario: Rule 3 - Multiple input borrow parameters
- **WHEN** `fun longest(a: &String, b: &String, c: Bool): &String = if (c) a else b`, `fun pick(a: &mut Int, b: &Int): &Int = b` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val y = 2; val r = pick(&mut x, &y); print(x); print(r) }` is checked
- **THEN** the borrow checker accepts `longest` and `pick`, and emits `E5003` at `x` in `print(x)`, because the result of `pick` borrows from both inputs, each in its own mode, whatever its body returns

#### Scenario: The signature decides, not the body
- **WHEN** `struct User { val name: String }`, `struct Team { var lead: String }`, `val DEFAULT = User("guest")`, `fun Team.fallback(): &User = &DEFAULT` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val f = t.fallback(); t.lead = "b"; greet(f.name)`
- **THEN** the borrow checker emits `E5004` at `t.lead`, because rule 2 ties the result of `fallback` to its receiver, although its body returns a borrow of a top-level `val`

#### Scenario: Elision across files uses the imported signature
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }`, `public fun Team.captain(o: &String): &String = &lead` and `public fun first(a: &String, b: &String): &String = a`, and `app/main.prism` imports `/lib/team` and declares `fun greet(s: &String): Int = 1` and `fun take(s: String) {}`, and either `fun f(t: Team, s: String): Int { var u = t; var x = s; val c = u.captain(&x); x = "y"; return greet(c) }` or `fun g(a: String, b: String): Int { val r = first(&a, &b); take(b); return greet(r) }`
- **THEN** `prism check app/main.prism` accepts `f`, whose call result is tied to the receiver only by the imported signature of `captain`, and emits `E5005` at `b` in `take(b)` of `g`, whose call result is tied to both arguments of `first`

### Requirement: Returned Borrows Derive Only From Tied Inputs
A body MUST return only borrows of top-level `val`s, which are valid everywhere, or borrows derived from the inputs its elision rule ties the result to. The borrow checker MUST reject a returned borrow of a local, a temporary or a by-value parameter with `DanglingBorrowReturn` (`E5008`), and one derived from a parameter the result is not tied to with `UntiedBorrowReturn` (`E5009`).

#### Scenario: Rule 2 - Method receiver borrow
- **WHEN** `struct Team { var lead: String }`, `fun greet(s: &String): Int = 1` and `fun Team.captain(o: &String): &String = &lead` are declared and `fun f(t: Team, s: String): Int { var u = t; var x = s; val c = u.captain(&x); x = "y"; return greet(c) }` is checked, or `fun Team.other(o: &String): &String = o` is checked
- **THEN** the borrow checker accepts `f`, because the result of `captain` is tied to the receiver `u` only, and emits `E5009` at `o` in the body of `other`, which returns a borrow derived from a parameter other than its receiver

#### Scenario: Rule 4 - Zero input borrows returning a borrow
- **WHEN** `fun f(): &Int { val n = 1; return &n }` and `fun g(s: String): &String = &s` are checked
- **THEN** the borrow checker emits `E5008` at `&n` in `f`, a borrow of the local `n`, and at `&s` in `g`, a borrow of the by-value parameter `s`

#### Scenario: Returning a borrow of a local under any rule
- **WHEN** `struct Team { var lead: String }` and `fun make(): String = "x"` are declared, and `fun f(t: &Team): &String { val s = "x"; return &s }`, `fun g(t: &Team): &String = &make()` and `fun Team.h(): &String { val c = lead.clone(); return &c }` are checked
- **THEN** the borrow checker emits `E5008` for each: at `&s` for the local `s`, at `&make()` for a temporary value, and at `&c` for the local `c`

#### Scenario: Borrow of a top-level val
- **WHEN** `struct User { val name: String }`, `val DEFAULT = User("guest")`, `fun greet(s: &String): Int = 1`, `fun guest(): &User = &DEFAULT` and `fun orGuest(u: &User, c: Bool): &User = if (c) u else &DEFAULT` are declared and `fun f(): Int { val g = guest(); return greet(g.name) }` is checked
- **THEN** the file passes `prism check`: a borrow of a top-level `val` may be returned under any rule, including from `guest`, which has no input borrow, and the result of `guest` keeps nothing borrowed

#### Scenario: A top-level value cannot borrow a temporary
- **WHEN** `fun make(): String = "x"`, `val LIMIT = 10`, `val REF: &Int = &LIMIT`, `val R: &Int = &1` and `val S: &String = &make()` are checked
- **THEN** the borrow checker accepts `REF` and emits `E5008` at `&1` and at `&make()`, borrows of temporaries that end with the initializer

### Requirement: Function Types Elide by the Same Rules
A function type MUST elide by rules 1, 3 and 4: a call of a function value MUST tie its result to every input-borrow argument, or, when its type has no input borrow, to the function value's own borrows. So a closure MUST return only borrows of top-level `val`s or derived from its reference parameters (`E5009` otherwise), or, when its type has no input borrow, also of what it captures; a borrow of its own local or temporary is `E5008`.

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

### Requirement: Default Values Borrow Only Through Shared References
A reference-typed parameter's default MUST return only a borrow of the temporary in the caller's slot, of a top-level `val`, or derived from what an earlier shared reference parameter (`&T`, `(&T)?`) or a `fun` receiver refers to. A default borrowing an earlier by-value parameter, or deriving a borrow from an earlier `&mut` or function-typed parameter or a `mut fun` receiver, which the call passes beside it, MUST be rejected at its declaration with `E5009`.

#### Scenario: A default borrowing an earlier by-value parameter
- **WHEN** `fun m(a: String, b: &String = a): Int = 1` and `fun k(a: Int, b: &Int = a): Int = 1` are checked
- **THEN** the borrow checker emits `E5009` at `a` in each default value, because the call takes `a` by value

#### Scenario: A default borrowing a mutable receiver
- **WHEN** `struct User { var name: String }` is declared, and `mut fun User.rename(n: &String = name) {}`, `fun f(m: &mut User, n: &String = m.name): Int = 1` and `fun g() { var u = User("a"); u.rename() }` are checked
- **THEN** the borrow checker emits `E5009` at `name` in the default of `rename` and at `m.name` in the default of `f`, and nothing at the call `u.rename()`

#### Scenario: A default reading through a shared receiver keeps it borrowed
- **WHEN** `struct User { var name: String }`, `fun User.hi(n: &String = name): &String = n` and `fun greet(s: &String): Int = 1` are declared, and `fun f(): Int { var u = User("a"); val h = u.hi(); return greet(h) }` and `fun g(): Int { var u = User("a"); val h = u.hi(); u.name = "b"; return greet(h) }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5004` at `u.name` in `g`, because `h` borrows `u` through the default

### Requirement: Borrows Stored Through Inputs Outlive the Call
A body MAY store a borrow derived from its inputs through a `&mut R` parameter whose `R` holds borrows, and a closure into a variable it captures, since callers keep what such a parameter or variable points at borrowed by every other operand (see "Calls and Closures Write Borrows Through What They Are Given"). A by-value parameter MUST end when its body returns, so storing a borrow of a local, a temporary or a by-value parameter there MUST be rejected with `DroppedWhileBorrowed` (`E5006`).

#### Scenario: Storing an input borrow through another input
- **WHEN** `fun store(slot: &mut &Int, v: &Int) { *slot = v }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Storing a borrow of a by-value parameter
- **WHEN** `fun run2(f: (Int) -> Int): Int = f(1)` and `fun print(n: &Int) {}` are declared, and `fun keep(slot: &mut &Int, v: Int) { *slot = &v }` and `fun g() { var a = 1; var r: &Int = &a; run2 { x -> r = &x; 1 }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` at `&v`, which `keep`'s caller would see through `slot` after `v` ends, and at `&x`, which `r` would keep after the closure returns

### Requirement: Returned Borrow Diagnostics Name the Source
An `E5008` diagnostic MUST point at where the borrow is returned, name the local, temporary or parameter it borrows, and label where it is borrowed when that is elsewhere; for a function with no input borrow it MUST note that only borrows of top-level values can be returned. An `E5009` diagnostic MUST point at where the borrow is returned, name the parameter it derives from with a related location at that parameter, and note the rule that does not tie the result to it.

#### Scenario: Dangling returned borrow labels
- **WHEN** `prism check` runs on `fun f(): &Int { val n = 1; val r = &n; return r }`
- **THEN** it reports `error[E5008]: cannot return a borrow of local 'n'` at `r` in `return r` labelled "returned here", with a related location at `&n` labelled "'n' is borrowed here", and a note that `f` takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)

#### Scenario: Untied returned borrow labels
- **WHEN** `struct Team { var lead: String }` is declared and `prism check` runs on `fun Team.other(o: &String): &String = o`
- **THEN** it reports `error[E5009]: method 'other' cannot return a borrow derived from parameter 'o'` at `o` in the body labelled "returned here", with a related location at the parameter `o` labelled "'o' is a parameter here", and a note that a method's returned borrow is tied to its receiver only (elision rule 2)

## MODIFIED Requirements

### Requirement: Passing a Mutable Borrow Reborrows It
A place of type `&mut T` passed as an argument where `&mut T` or `&T` is expected, or a place of type `(&mut T)?` passed where `(&mut T)?` or `(&T)?` is expected, MUST be reborrowed for the call rather than moved, so that it stays usable afterwards. `&mut` applied to a `val` binding MUST be rejected with `E4003`, whatever the binding's type.

#### Scenario: Passing a mutable borrow twice
- **WHEN** `fun bump(r: &mut Int)` and `fun read(r: &Int): Int` are declared and `fun f(r: &mut Int): Int { bump(r); bump(r); return read(r) }` is checked
- **THEN** the file typechecks successfully, and each call reborrows `r` without `&mut`

#### Scenario: Passing an optional mutable borrow twice
- **WHEN** `struct User { var name: String }`, `fun edit(m: (&mut User)?) {}` and `fun both(a: (&mut User)?, b: (&mut User)?) {}` are declared, and `fun f(m: (&mut User)?) { edit(m); edit(m) }`, `fun g(u: User) { var p = u; val m: (&mut User)? = &mut p; edit(m); edit(m) }` and `fun h(m: (&mut User)?) { both(m, m) }` are checked
- **THEN** the borrow checker accepts `f` and `g`, where each call reborrows `m` and `m` stays usable, and emits `E5003` at the second `m` in `both(m, m)`, two mutable reborrows passed to one call

#### Scenario: Mutable borrow of a val reference binding rejected
- **WHEN** `fun f(r: &mut Int) { val m = &mut r }` is checked
- **THEN** the type checker emits `E4003`, because `&mut r` borrows the `val` binding `r` itself

### Requirement: Calls Returning References Keep Their Inputs Borrowed
The result of a call whose type contains a reference outside a function type MUST hold the borrows passed to the inputs its elision rule ties it to (see "The Elision Rules Tie a Returned Borrow to Inputs"), each in its own mode, so a `&mut` argument it is tied to MUST stay frozen while the result is live: for a method with a borrowed receiver, the receiver's borrows only; for any other function, the borrows of every input borrow (its parameters whose type contains a reference, other than function types). The result of calling a function value MUST hold the borrows of every input-borrow argument or, when its type has no input borrow, those of the function value (see "Function Types Elide by the Same Rules"). The result of a default value MUST hold the borrows of its slot and what the earlier shared reference arguments, and a `fun` receiver, borrow, and nothing else: not the default's own borrows of the earlier arguments.

#### Scenario: A returned borrow keeps every input borrowed
- **WHEN** `fun first(a: &String, b: &String): &String = a`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared, and `fun f(x: String, y: String) { val r = first(&x, &y); take(y); greet(r) }` and `fun g(x: String, y: String) { val r = first(&x, &y); greet(r); take(y) }` are checked
- **THEN** the borrow checker emits `E5005` at `y` in `take(y)` of `f`, and accepts `g`

#### Scenario: A mutable input stays frozen while the result is live
- **WHEN** `fun pick(a: &mut Int): &Int = a` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val r = pick(&mut x); print(x); print(r) }` is checked
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`

#### Scenario: A method result keeps its receiver borrowed
- **WHEN** `struct Team { var lead: String }`, `fun Team.captain(): &String = &lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val c = t.captain(); t.lead = "b"; greet(c)`
- **THEN** the borrow checker emits `E5004` at `t.lead`

#### Scenario: A method result does not keep its other arguments borrowed
- **WHEN** `struct Team { var lead: String }`, `mut fun Team.leader(o: &String): &mut String = &mut lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); var s = "x"; val l = t.leader(&s); s = "y"; greet(l)`
- **THEN** the file passes `prism check`, because the result of `leader` is tied to its receiver only

#### Scenario: An omitted reference default after a by-value parameter
- **WHEN** `fun greet(a: String, n: &String = "anon"): Int = 1` is declared and `fun f(s: String): Int = greet(s)` is checked
- **THEN** the file passes `prism check`

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. A parameter that ends when its body returns MUST be labelled at its declaration, and a later use by the caller, through a `&mut R` parameter, at that parameter. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Dropped parameter labels
- **WHEN** `prism check` runs on `fun keep(slot: &mut &String, v: String) { *slot = &v }`
- **THEN** it reports `error[E5006]: 'v' does not live long enough` at `&v` labelled "borrowed value does not live long enough", with a related location at the parameter `v` labelled "'v' is dropped when the function returns, while still borrowed" and one at the parameter `slot` labelled "borrow later used by the caller through 'slot'"

#### Scenario: Escaping closure labels
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism check` runs on `fun f(s: String): () -> Int = { greet(s) }`
- **THEN** it reports `error[E5007]: a closure that captures 's' cannot escape` at the closure labelled "returned here", with a related location at `s` inside the closure labelled "'s' is captured here", a note that a closure borrows what it captures, and help suggesting to pass `s` to the closure as a parameter
