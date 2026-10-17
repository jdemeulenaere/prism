# References Specification

## Purpose

The references specification defines how borrowed references are used as the type checker enforces it: where borrows are created implicitly, how values are read and written through references, how reference bindings are re-pointed and reborrowed, how borrowed nullables are viewed, and how reference types relate. Borrow conflicts and the lifetimes of returned borrows belong to the borrow checker and are added here when it exists.

## ADDED Requirements

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
A parameter of type `&mut T` MUST NOT borrow an owned argument implicitly. The caller MUST write `&mut`, and an owned place passed without it MUST be rejected with `E4004`, with help suggesting `&mut` before the argument.

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
