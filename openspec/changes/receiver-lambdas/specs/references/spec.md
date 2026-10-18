# References Specification Delta

## ADDED Requirements

### Requirement: Receivers of Function Values Are Borrowed Like Extension Receivers
The receiver of a call of a receiver function value MUST be borrowed as its parameter's type says: in `f(r, a)` as any `&T` or `&mut T` argument (implicitly for a shared receiver, with `&mut` written for a mutable one); in `r.f(a)` as the receiver of a `fun` or `mut fun` extension, looking through `&`, `&mut` and `Heap<T>` layers, a mutable receiver being a mutable place (`E4003` otherwise); and in `f(a)` as a reborrow of the innermost `this`.

#### Scenario: Mutable receivers of function values
- **WHEN** `struct Builder { var size: Int }` is declared and `fun a(f: mut Builder.() -> Unit) { var b = Builder(0); f(&mut b); b.f() }`, `fun c(f: mut Builder.() -> Unit) { val b = Builder(0); b.f() }` and `fun d(f: mut Builder.() -> Unit) { var b = Builder(0); f(b) }` are checked
- **THEN** the type checker accepts `a`, emits `E4003` for `c`, because `b` is a `val`, and emits `E4004` for `d`, because a mutable borrow is written `&mut b`

#### Scenario: A shared receiver may be a temporary
- **WHEN** `struct Scope : Copy {}` and `fun make(): Scope = Scope()` are declared and `fun a(f: Scope.() -> Int): Int = f(Scope()) + make().f()` is checked
- **THEN** the file passes `prism check`, each receiver being a temporary that lives until the end of the statement

### Requirement: Context Arguments Are Shared Borrows
A context argument MUST be a shared borrow of its candidate's referent (the innermost receiver lambda's or the extension's receiver, or the context parameter), held for the call as any shared argument is, so a call needing a context MUST NOT be made while the candidate is mutably borrowed elsewhere.

#### Scenario: A context argument from a mutable receiver
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct Builder { var count: Int }` and `context(b: Builder) fun Modifier.counted(): Int = b.count` are declared and `fun build(f: mut Builder.() -> Int): Int { var b = Builder(1); return b.f() }` and `fun g(): Int = build { count += 1; Modifier(0.0).counted() }` are checked
- **THEN** the file passes `prism check`, the call of `counted` borrowing the lambda's receiver shared after the write through it

### Requirement: Receiver Function Types Elide Like Function Types
A function type with a receiver MUST elide by rules 1, 3 and 4 as every function type does, its receiver counting as an input borrow like any reference parameter, so it MUST tie a returned borrow exactly as the plain function type with the same parameters does. A context parameter MUST count as an input borrow parameter: with a borrowed receiver, rule 2 ties a returned borrow to the receiver only.

#### Scenario: A receiver lambda returning a borrow of its other parameter
- **WHEN** `struct Scope : Copy {}` and `fun pick(f: Scope.(&Int) -> &Int): Int = 1` are declared and `fun a(): Int = pick { k -> k }` and `fun b(): Int = pick { k -> val m = 2; &m }` are checked
- **THEN** the borrow checker accepts `a`, because the result of a `Scope.(&Int) -> &Int` is tied to both of its input borrows, and emits `E5008` at `&m` in `b`

#### Scenario: A context extension returns a borrow of its receiver only
- **WHEN** `struct Team { val lead: String }`, `struct RowScope { val name: String }`, `context(s: RowScope) fun Team.leader(): &String = &lead` and `context(s: RowScope) fun Team.scopeName(): &String = &s.name` are checked
- **THEN** the borrow checker accepts `leader` and emits `E5009` at `&s.name` in `scopeName`, because rule 2 ties the result to the receiver only
