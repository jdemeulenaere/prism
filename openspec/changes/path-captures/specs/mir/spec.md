## MODIFIED Requirements

### Requirement: Closures Are Separate Bodies
Each closure MUST be lowered to its own body whose captured paths are capture locals of the paths' types following its arguments, a use of a captured path or of a place within it reading that local, and the enclosing body MUST build the closure value from them in capture order: a move or copy of each path the typed tree records as owned, the place of each other one. MIR as lowered MUST record no capture mode; the borrow checker infers how a borrowed one is borrowed, and elaborated MIR MUST record one mode per capture local, shared, mutable or owned (see `references`).

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int, b: Int): Int = run { b + a + b }` is lowered
- **THEN** the closure's body has the capture locals `b` and `a` and adds them, and the body of `g` builds the closure value from its arguments `b` and `a` and passes it to `run`

#### Scenario: Capture modes in elaborated MIR
- **WHEN** `struct User { var name: String }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(n: Int) { var u = User("a"); run { u.name = "z"; n } }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure value from a mutable borrow of `u.name` and a shared borrow of `n`, while the MIR as lowered records no mode

#### Scenario: Mixed capture modes in elaborated MIR
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(): Int { var count = 0; var h: () -> Int = { 0 }; if (count == 0) { val s = "a"; h = { count += 1; greet(s) } }; h(); return count }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure `{ count += 1; greet(s) }` from a mutable borrow of `count` and a move of `s`, and its body records the modes mutable and owned

#### Scenario: Captured paths are capture locals
- **WHEN** `struct P { var x: Int, var y: Int }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(): Int { var p = P(1, 2); return run { p.x += 1; p.x + p.y } }` passes the borrow checker
- **THEN** the closure's body has the capture locals `p.x` and `p.y`, each of type `Int`, and the elaborated MIR of `f` builds the closure value from a mutable borrow of the place `p.x` and a shared borrow of the place `p.y`

### Requirement: Owned Captures Are Operands of the Closure Value
A closure value MUST be built from a move of each non-`Copy` capture it owns and a copy of each `Copy` capture it owns, a copied path read from its place, as the typed tree records them, already in MIR as lowered, and, in elaborated MIR, a borrow of each capture it borrows, in capture order. The enclosing body MUST NOT drop a binding a closure moved, and `prism dump-mir` MUST print owned captures as operands (`[move _1, copy _2, _3]` as lowered, `[move _1, copy _2, &_3]` elaborated).

#### Scenario: Owned captures in elaborated MIR
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(s: String, n: Int): () -> Int = { greet(s) + n }` passes the borrow checker
- **THEN** the MIR of `f`, as lowered and elaborated, builds the closure value from a move of `s` and a copy of `n`, and the elaborated MIR has no drop of `s`

#### Scenario: Copied paths in MIR
- **WHEN** `struct Todo { val id: Int, val title: String }` is declared and `prism dump-mir` runs on `fun f(t: &Todo): () -> Int = { t.id }` and `fun g(u: Todo): () -> Int = { u.id }`
- **THEN** it prints the closure value of `f` as built from `[copy (*_1).id]` and that of `g` from `[copy _1.id]`, and the body of `g` still drops `u`
