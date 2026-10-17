# References Specification

## Purpose
The references specification defines how borrowed references are used as the type checker enforces it: where borrows are created implicitly, how values are read and written through references, how reference bindings are re-pointed and reborrowed, how borrowed nullables are viewed, and how reference types relate. Borrow conflicts and the lifetimes of returned borrows belong to the borrow checker and are added here when it exists.

## Requirements

### Requirement: Implicit Shared Borrow at Argument Sites
Where a call's parameter has type `&T`, an argument of type `T` MUST be borrowed implicitly. A place argument (a variable, a `.` field path, or an index) MUST be borrowed where it lives, without being consumed. Any other argument MUST be evaluated as a temporary, in a consuming position, that lives until the end of the statement. Default values are arguments. Elsewhere a borrow MUST be written with `&`.

#### Scenario: Implicit shared borrow at argument sites
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared and a body of `fun f(u: &User, name: String)` runs `greet(name)`, `greet(u.name)` and `greet("Ada")`
- **THEN** the body typechecks successfully, and neither `name` nor `u.name` is reported as moved

#### Scenario: Borrowing the result of a call
- **WHEN** `fun make(): String` and `fun greet(name: &String)` are declared and `fun f() { greet(make()) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A branch result is a temporary
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared, and `fun f(c: Bool, u: User, v: User) { greet(if (c) u.name else v.name) }` and `fun g(c: Bool) { greet(if (c) "a" else "b") }` are checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`, and accepts `g`

#### Scenario: A default value is borrowed like an argument
- **WHEN** `fun greet(name: &String = "guest")` is declared and `fun f() { greet() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling a function-typed value
- **WHEN** `fun f(g: (&String) -> Unit, s: String) { g(s) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Borrows outside arguments are written
- **WHEN** `struct User { val name: String }` is declared, and `fun f(name: String) { val r: &String = name }` and `fun g(u: &User): &String = u.name` are checked
- **THEN** the type checker emits `E4002` for `f`, with help suggesting `&name`, and `E4005` for `g`, with help suggesting `&u.name`

### Requirement: Explicit Mutable Borrows at Call Sites
A parameter of type `&mut T` MUST NOT borrow an owned argument implicitly. The caller MUST write `&mut`, and an owned place passed without it MUST be rejected with `E4004`, with help suggesting `&mut` before the argument. The receiver of a `mut fun` call is the one exception: it is borrowed mutably without `&mut` (see "Mutable Receivers Must Be Mutable Places").

#### Scenario: Explicit mutation at call sites
- **WHEN** `fun bump(r: &mut Int)` is declared, and a body declares `var x = 1` and calls `bump(x)` and `bump(&mut x)`
- **THEN** the type checker emits `E4004` for `bump(x)`, with help suggesting `&mut x`, and accepts `bump(&mut x)`

### Requirement: Passing a Mutable Borrow Reborrows It
A place of type `&mut T` passed as an argument where `&mut T` or `&T` is expected MUST be reborrowed for the call rather than moved, so that it stays usable afterwards. `&mut` applied to a `val` binding MUST be rejected with `E4003`, whatever the binding's type.

#### Scenario: Passing a mutable borrow twice
- **WHEN** `fun bump(r: &mut Int)` and `fun read(r: &Int): Int` are declared and `fun f(r: &mut Int): Int { bump(r); bump(r); return read(r) }` is checked
- **THEN** the file typechecks successfully, and each call reborrows `r` without `&mut`

#### Scenario: Mutable borrow of a val reference binding rejected
- **WHEN** `fun f(r: &mut Int) { val m = &mut r }` is checked
- **THEN** the type checker emits `E4003`, because `&mut r` borrows the `val` binding `r` itself

### Requirement: Copy-Out From References
Where a value of type `T` is expected and the expression has a reference type (`&T`, `&mut T` or nested borrows of `T`), a `Copy` value MUST be copied out implicitly, at every value position: initializers with a type, assigned values, returned values, by-value arguments, constructor fields and payloads, and default values. `(&T)?` MUST be accepted where `T?` is expected when `T` is `Copy`. An unannotated initializer MUST keep the reference type.

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

### Requirement: Writing Through a Mutable Reference
`*r = v` MUST write `v` into the place `r` refers to. Every reference layer written through MUST be `&mut`, or the assignment is rejected with `E4003`. A `*` applied to a value that is not a reference MUST be rejected with `NotAReference` (`E4019`). The value MUST be checked against the referent type in a consuming position, and the binding holding the reference MAY be a `val`.

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
- **THEN** the type checker emits `E4019` naming the type `Int`

#### Scenario: The written value is checked and consumed
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(r: &mut String, a: &Int, n: &mut Int, s: String, u: User)` runs `*n = a`, `*r = s`, `*r = u.name` and `*n = "x"`
- **THEN** `*n = a` (a copy-out) and `*r = s` (a move of a whole parameter) are accepted, the type checker emits `E4017` for `u.name`, and it emits `E4002` for `"x"`

#### Scenario: Writing through the result of a call
- **WHEN** `fun pick(a: &mut Int, b: &mut Int): &mut Int` is declared and a body declares `var a = 1` and `var b = 2` and runs `*pick(&mut a, &mut b) = 0`
- **THEN** the body typechecks successfully

### Requirement: Re-Pointing Reference Variables
Assigning to a `var` binding of reference type MUST re-point it to the assigned reference without writing to any referent. Assigning a value of the referent's type to such a binding MUST be rejected (`E4002` for a `var`, `E4003` alone for a `val`), with help suggesting `*r = v` when the reference is `&mut`.

#### Scenario: Re-pointing a reference variable
- **WHEN** a body declares `var r: &Int = &a` and later assigns `r = &b`
- **THEN** `r` refers to `b`, and neither `a` nor `b` is modified

#### Scenario: Assigning a value to a mutable reference variable
- **WHEN** a body declares `var x = 1`, `var y = 2` and `var r: &mut Int = &mut x`, then assigns `r = 5` and `r = &mut y`
- **THEN** the type checker emits `E4002` for `r = 5`, with help suggesting `*r = 5`, and accepts `r = &mut y`

### Requirement: Borrowed Nullables Expose Their Contents
`&(T?)` MUST convert implicitly to `(&T)?`, and `&mut (T?)` to `(&mut T)?` or `(&T)?`, wherever such a type is expected. Safe navigation and `?:` MUST read through references to a nullable as if they were `(&T)?`. A place of type `T?` passed to a `(&T)?` parameter MUST be borrowed where it lives. `(&T)?` MUST NOT convert to `&(T?)`.

#### Scenario: Borrowing a nullable field without cloning
- **WHEN** `struct User { val nick: String? }`, `fun greet(n: &String)` and `fun show(n: (&String)?)` are declared and a body of `fun f(u: &User)` runs `show(u.nick)`, `greet(&u.nick ?: &"anon")` and `val a: (&String)? = &u.nick`
- **THEN** the body typechecks successfully, and `u.nick` is not reported as moved

#### Scenario: Elvis over a borrowed nullable
- **WHEN** `fun greet(n: &String)` is declared, and `fun f(n: &String?) { greet(n ?: &"anon") }` and `fun g(n: &String?) { greet(n ?: "anon") }` are checked
- **THEN** `f` typechecks successfully, and the type checker emits `E4002` for `g`, because `"anon"` is a `String` and the left side is a `&String`

#### Scenario: Safe navigation through a borrowed nullable
- **WHEN** `struct User { val id: Int }` is declared and `fun f(n: &User?): Int? = n?.id` and `fun g(n: &User?): Bool = n == null` are checked
- **THEN** both typecheck successfully

#### Scenario: Mutable view of a borrowed nullable
- **WHEN** `fun set(r: (&mut Int)?)` and `fun read(r: (&Int)?)` are declared and `fun f(m: &mut Int?) { set(m); read(m) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Optional borrow does not become a borrowed nullable
- **WHEN** `fun g(r: &String?)` is declared and `fun f(r: (&String)?) { g(r) }` is checked
- **THEN** the type checker emits `E4004`

### Requirement: Reference Types Are Invariant
`&U` MUST be assignable to `&T` only when `U` and `T` are the same type, or differ only in that a reference inside `U` is `&mut` where `T` has `&`. `&mut U` MUST be assignable to `&mut T` only when `U` and `T` are the same type. `&mut U` MUST be assignable to `&T` whenever `&U` is.

#### Scenario: A borrowed value is not a borrowed nullable
- **WHEN** `fun g(r: &Int?)` is declared and `fun f(a: &Int) { g(a) }` is checked
- **THEN** the type checker emits `E4004`, because `&Int` and `&(Int?)` refer to values of different layouts

#### Scenario: Mutable borrows weaken to shared ones
- **WHEN** `fun read(r: &Int): Int` and `fun peek(rr: &&Int): Int` are declared and `fun f(r: &mut Int, rr: &(&mut Int)): Int { return read(r) + peek(rr) }` is checked
- **THEN** the file typechecks successfully

### Requirement: Method Receivers Borrow Implicitly
The receiver of an extension call MUST be passed by the extension's receiver mode with nothing written at the call site: borrowed shared for `fun`, borrowed mutably for `mut fun`, and by value for `consuming fun` (see `ownership`). Any number of `&`, `&mut` and `Heap<T>` layers around the receiver MUST be looked through. A place MUST be borrowed where it lives, and any other receiver MUST be a temporary, in a consuming position, that lives until the end of the statement.

#### Scenario: Shared receivers of places, references and temporaries
- **WHEN** `struct User { val name: String }`, `fun User.greet(): String`, `fun String.shout(): String` and `fun make(): User` are declared, and a body of `fun f(u: User, r: &User, rr: &&User, m: &mut User, h: Heap<User>)` runs `u.greet()`, `u.name.shout()`, `r.greet()`, `rr.greet()`, `m.greet()`, `h.greet()`, `make().greet()` and `"a".shout()`
- **THEN** the body typechecks successfully, and neither `u` nor `u.name` is reported as moved

#### Scenario: A branch receiver is a temporary
- **WHEN** `fun String.shout(): String` and `struct User { val name: String }` are declared and `fun f(c: Bool, u: User, v: User): String = (if (c) u.name else v.name).shout()` is checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`

### Requirement: Method-Call Receivers Are Classified Before Resolution
The receiver of every call `value.name(...)` MUST be classified before the member it calls is resolved, since that depends on the receiver's type: a place, or a `?.` chain of fields over one, is read in place, and anything else is a temporary in a consuming position. This applies to a function-typed field call and to the built-in `clone()` as to an extension call.

#### Scenario: A branch receiver of clone is a temporary
- **WHEN** `struct User { val name: String }` is declared and `fun g(c: Bool, u: User, v: User): String = (if (c) u.name else v.name).clone()` is checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`

### Requirement: Mutable Receivers Must Be Mutable Places
The receiver of a `mut fun` call MUST be borrowed mutably without `&mut`. It MUST be a `var` binding, a field path whose every hop is mutable, a place reached through `&mut` (a `val` binding holding a `&mut T` is reborrowed), or a temporary. A `val` binding, a `val` field, or a place reached through a shared `&` MUST be rejected with `E4003`.

#### Scenario: Method receivers borrow implicitly
- **WHEN** `struct User { var name: String }` and `mut fun User.rename(n: String) { name = n }` are declared, and a body declares `var u = User("a")` and `val w = User("b")` and calls `u.rename("x")` and `w.rename("y")`
- **THEN** the type checker accepts `u.rename("x")`, which borrows `u` mutably without `&mut`, and emits `E4003` for `w.rename("y")`, with help suggesting `var w`

#### Scenario: Mutable receivers through references
- **WHEN** `mut fun User.rename(n: String)` is declared, and `fun f(m: &mut User) { m.rename("x"); m.rename("y") }` and `fun g(r: &User) { r.rename("x") }` are checked
- **THEN** the type checker accepts `f`, reborrowing `m` for each call, and emits `E4003` for `g`

#### Scenario: Mutable receiver on a field path
- **WHEN** `struct User { var name: String }`, `struct Team { var lead: User, val owner: User }` and `mut fun User.rename(n: String)` are declared, and a body declares `var t: Team` and calls `t.lead.rename("x")` and `t.owner.rename("y")`, and `fun h(p: &mut Team) { p.lead.rename("z") }` is checked
- **THEN** the type checker accepts `t.lead.rename("x")` and `h`, and emits `E4003` for `t.owner.rename("y")`

#### Scenario: Mutable receiver of a temporary
- **WHEN** `fun make(): User` and `mut fun User.rename(n: String)` are declared and `fun f() { make().rename("x") }` is checked
- **THEN** the file typechecks successfully, as `&mut make()` does

#### Scenario: Mutable call on the implicit receiver
- **WHEN** `mut fun User.rename(n: String)` is declared, and `mut fun User.reset() { rename("") }` and `fun User.peek() { rename("") }` are checked
- **THEN** the type checker accepts `reset`, reborrowing `this`, and emits `E4003` for `peek`, because its `this` is a shared borrow

#### Scenario: Mutable safe call on a nullable place
- **WHEN** `mut fun User.rename(n: String)` is declared, and a body declares `var u: User? = null` and `val v: User? = null` and runs `u?.rename("x")` and `v?.rename("y")`
- **THEN** the type checker accepts `u?.rename("x")` and emits `E4003` for `v?.rename("y")`

## Design Rationale
Decisions from `reference-access` (agreed with the user in the borrow-checker roadmap exploration and while proposing the change, 2026-10-17):

- **Shared borrows are implicit at argument sites; `&mut` stays explicit** (user). A move-only `String` makes `&String` parameters common, and an explicit `&` on a read adds noise without adding safety, while mutation stays visible at the call. Default values are evaluated as the call's argument, so they borrow like one. Outside arguments, `&` is written. Rejected: explicit `&` everywhere (noise), and Swift/Hylo/Mojo-style parameter conventions, where `&T` is not a type, because they would reshape the elision rules the invariants commit to.
- **A borrow points at one place or one temporary** (user). A place (a variable, a `.` field path from any root, an index) is borrowed where it lives. Anything else, including `if`, `when`, blocks, `?:` and safe navigation, is evaluated into a consumed temporary that lives until the end of the statement. So `greet(if (c) u.name else v.name)` moves the fields (`E4017`) and `greet(&u.nick ?: &"anon")` needs `&` on each side. Distributing a borrow to the leaves of branches was rejected: one rule matches Rust, and the MIR lowers each borrow to one location. It remains a possible relaxation.
- **Arguments to reference parameters are inferred, not checked.** Such an argument may already be a reference (`greet(longest(&a, &b))`) or a value to borrow, which only its type tells apart. Nothing that needs an expected type (a closure, `null`) is ever borrowed.
- **Implicit copy-out of `Copy` values** (user). Where a `T` is expected, reference layers are peeled until the type fits. A `Copy` value is copied bitwise, and a non-`Copy` one is a move out of the borrow (`E4017`, see `ownership`). Copy-out also works through one `?` (`(&T)?` as a `T?`), since `T?` is then `Copy`. It is not applied inside `if`/`?:` joins, where a join's type would then depend on which side is a reference.
- **Prefix `*` writes through a reference, and only there** (user). Reads have one spelling, the implicit one. Every layer written through must be `&mut` (`E4003`), and `*` on a non-reference is `NotAReference` (`E4019`). `*` as a general place operator (`val x = *r`, `&mut *r`) was rejected, at the cost of having no syntax that reborrows a `&mut` into a local. Transparent C++-style references were rejected because they cannot be re-pointed, and a type-directed `r = v` because it is hard to read.
- **`var` references re-point** (user). `r = &b` assigns the reference itself. A value of the referent's type gets help pointing at `*r = v`, and a `val` reports only `E4003`.
- **Passing a `&mut` place reborrows it** (user). The binding stays usable after the call. `&mut r` on a `val r: &mut T` follows the ordinary `val` rule, because it borrows the binding: with write-through, `*(&mut r) = &mut other` would otherwise re-point a `val`.
- **Borrowed nullables are viewed as optional borrows** (user). `&(T?)` converts to `(&T)?` (and `&mut (T?)` to `(&mut T)?` or `(&T)?`), and `?.` and `?:` read through such a reference. This is a conversion, not subtyping: it reads the tag and points at the payload. The reverse is impossible, because a `(&T)?` has no `T?` to point at. Rejected: an explicit member like Rust's `Option::as_ref` (noise on a common Kotlin idiom), and leaving cloning as the only way to read a borrowed nullable.
- **References are invariant** (user). A reference points at existing storage, so it cannot change its referent's representation: `T` to `T?` adds a tag, which a value conversion can do and a pointer cannot. Only `&mut` weakening to `&` (at the top, and behind shared borrows) is allowed. Covariance would have made codegen hide a converted copy behind a "borrow".
- **One coercion step.** Implicit borrows, reborrows, the nullable view and copy-out are decided by one function at every value position after plain assignability fails, which classifies each as an adjustment for the typed tree. Folding them into assignability was rejected, because assignability is also used for joins and equality, where silent borrows and copies would be wrong. A place's move check runs after the coercion, so a borrow or copy-out moves nothing, and a value that does not fit reports only the mismatch.

Decisions from `extension-methods`:

- **Receivers are borrowed implicitly by their mode, including for `mut fun`** (user). `(&mut u).rename()` would be unreadable, and the method's declaration already says it mutates, as for Swift's `mutating` methods; this is the one exception to explicit `&mut` at call sites. The receiver of a `mut fun` must be a mutable place by the rules of `&mut`, or a temporary, matching `&mut make()`.
- **Receivers are looked through at any depth.** Every `&`, `&mut` and `Heap<T>` layer is peeled until the type matches the extension's receiver, as in Rust's auto-deref, because field access already reads through any depth. Requiring an exact match was rejected: `(*r).greet()` does not even parse, since `*` is writes-only.
- **Every method-call receiver is classified before its member is resolved.** Which member a call reaches depends on the receiver's type, which is only known once the receiver is inferred, and whether it is inferred in a consuming position is what the classification decides. So a branch over fields is a moved temporary for a field call or the built-in `clone()` as for an extension call. Resolving from a speculative inference was rejected: its errors would have to be discarded, including those of declarations inferred on demand along the way.
- **The receiver pass is decided at one point and classified** (auto-deref count, shared or mutable borrow, move or copy-out, temporary or place), so the typed tree can record it as an adjustment and MIR can emit the borrow after the arguments, which two-phase borrows need.
