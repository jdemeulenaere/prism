# MIR Specification Delta

## ADDED Requirements

### Requirement: Owned Captures Are Operands of the Closure Value
A closure value MUST be built from a move of each non-`Copy` capture it owns and a copy of each `Copy` capture it owns, as the typed tree records them, already in MIR as lowered, and, in elaborated MIR, a borrow of each capture it borrows, in capture order. The enclosing body MUST NOT drop a binding a closure moved, and `prism dump-mir` MUST print owned captures as operands (`[move _1, copy _2, _3]` as lowered, `[move _1, copy _2, &_3]` elaborated).

#### Scenario: Owned captures in elaborated MIR
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(s: String, n: Int): () -> Int = { greet(s) + n }` passes the borrow checker
- **THEN** the MIR of `f`, as lowered and elaborated, builds the closure value from a move of `s` and a copy of `n`, and the elaborated MIR has no drop of `s`

## MODIFIED Requirements

### Requirement: Closures Are Separate Bodies
Each closure MUST be lowered to its own body whose captured bindings are capture locals following its arguments, and the enclosing body MUST build the closure value from those bindings, in capture order: from a move or copy of each one the typed tree records as owned, and from the place of each other one. How a borrowed binding is borrowed MUST NOT be decided by the MIR as lowered: the borrow checker infers it, and the elaborated MIR MUST record exactly one mode per capture local, shared, mutable or owned (see `references`), and MIR as lowered none.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int, b: Int): Int = run { b + a + b }` is lowered
- **THEN** the closure's body has the capture locals `b` and `a` and adds them, and the body of `g` builds the closure value from its arguments `b` and `a` and passes it to `run`

#### Scenario: Capture modes in elaborated MIR
- **WHEN** `struct User { var name: String }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(n: Int) { var u = User("a"); run { u.name = "z"; n } }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure value from a mutable borrow of `u` and a shared borrow of `n`, while the MIR as lowered records no mode

#### Scenario: Mixed capture modes in elaborated MIR
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(): Int { var count = 0; var h: () -> Int = { 0 }; if (count == 0) { val s = "a"; h = { count += 1; greet(s) } }; h(); return count }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure `{ count += 1; greet(s) }` from a mutable borrow of `count` and a move of `s`, and its body records the modes mutable and owned
