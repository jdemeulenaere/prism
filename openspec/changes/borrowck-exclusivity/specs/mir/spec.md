# MIR Specification Delta

## ADDED Requirements

### Requirement: Scope Exits Are Located Where Control Leaves the Scope
The drops and storage ends that a braced block runs when control reaches its end MUST carry the span of the block's closing brace, and those that a `return`, `break` or `continue` runs MUST carry the span of that jump. Other scopes MUST keep their own span.

#### Scenario: Block-end drops at the closing brace
- **WHEN** `fun f(c: Bool) { if (c) { val s = "a" } }` is lowered, and `fun g(c: Bool) { while (c) { val s = "a"; if (c) break } }`
- **THEN** in `f` the drop and the storage end of `s` are at the `}` that closes the `if` branch, and in `g` the drop of `s` on the path of the `break` is at the `break`, and on the other path at the `}` that closes the loop body

## MODIFIED Requirements

### Requirement: Temporaries Live Until the End of Their Scope
A temporary MUST be dropped at the end of the innermost enclosing statement, block tail or body value, or of the innermost part that is evaluated conditionally or repeatedly: a condition, a `when` pattern test, a `for` range, a branch of `if` or `when`, the right operand of `&&` or `||`, the fallback of `?:`, or the rest of a `?.` chain after its first test. A temporary whose written borrow is the value of a branch of `if` or `when`, of the fallback of `?:` or of a block tail, through any nesting of these, MUST instead belong to the scope of the whole expression whose value that borrow becomes: its storage MUST start before that expression is evaluated, and it MUST be dropped at the end of that scope, on the paths where it was created.

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and a body runs `greet(make())` as a statement
- **THEN** the `String` returned by `make` is held in a temporary that `greet` borrows, and the temporary is dropped after `greet` returns, at the end of the statement

#### Scenario: Condition temporaries dropped before branching
- **WHEN** `fun check(s: &String): Bool = true`, `fun make(): String = "x"` and `fun g() {}` are declared and a body runs `if (check(make())) g()`
- **THEN** the temporary holding the result of `make` is dropped before the branch on the condition

#### Scenario: Branch temporaries dropped in their branch
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and `fun f(c: Bool): Int = if (c) greet(make()) else 0` is lowered
- **THEN** the temporary holding the result of `make` is created and dropped in the `then` branch, and the `else` branch drops nothing

#### Scenario: Tail temporaries dropped before the block's locals
- **WHEN** `fun greet(n: &String): Int = 1` and `fun make(): String = "x"` are declared and `fun f(): Int { val s = "a"; greet(make()) }` is lowered
- **THEN** the temporary holding the result of `make` is dropped before `s` is dropped

#### Scenario: A borrowed fallback temporary lives until the end of the statement
- **WHEN** `fun greet(n: &String): Int = 1` is declared and `fun f(n: String?): Int = greet(&n ?: &"anon")` is lowered
- **THEN** the storage of the temporary holding `"anon"` starts before the tag of `n` is read, the temporary is assigned only on the edge where `n` is `null`, and it is dropped after `greet` returns; in the elaborated MIR that drop is guarded by a drop flag

### Requirement: Borrowed Initializer Temporaries Live as Long as Their Binding
When the initializer of a `val` or `var` is a written borrow of a place rooted at a temporary, or an expression whose value is such a borrow in a branch of `if` or `when`, the fallback of `?:` or a block tail (the temporaries of the previous requirement), each such temporary MUST belong to the binding's block and be dropped just after the binding, instead of at the end of the declaration.

#### Scenario: Borrowed temporary in a val initializer
- **WHEN** `fun make(): String = "x"` and `fun greet(n: &String): Int = 1` are declared and `fun f(): Int { val r = &make(); return greet(r) }` is lowered
- **THEN** the temporary holding the result of `make` is still live when `greet` is called, and is dropped when the block ends

#### Scenario: Borrowed temporaries in the branches of a val initializer
- **WHEN** `fun make(): String = "x"` and `fun greet(n: &String): Int = 1` are declared and `fun f(c: Bool, s: String): Int { val r = if (c) &make() else &s; return greet(r) }` is lowered
- **THEN** the storage of the temporary holding the result of `make` starts before `c` is tested, it is assigned only in the `then` branch, and it is still live when `greet` is called and dropped when the block ends, under a drop flag in the elaborated MIR

### Requirement: Closures Are Separate Bodies
Each closure MUST be lowered to its own body whose captured bindings are capture locals following its arguments, and the enclosing body MUST build the closure value from the places of those bindings, in capture order. How each binding is captured MUST NOT be decided by the MIR as lowered: the borrow checker infers it, and the elaborated MIR MUST record for each capture local whether it is borrowed shared or mutably (see `references`).

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int, b: Int): Int = run { b + a + b }` is lowered
- **THEN** the closure's body has the capture locals `b` and `a` and adds them, and the body of `g` builds the closure value from its arguments `b` and `a` and passes it to `run`

#### Scenario: Capture modes in elaborated MIR
- **WHEN** `struct User { var name: String }` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(n: Int) { var u = User("a"); run { u.name = "z"; n } }` passes the borrow checker
- **THEN** the elaborated MIR of `f` builds the closure value from a mutable borrow of `u` and a shared borrow of `n`, while the MIR as lowered records no mode

### Requirement: MIR Is Well Formed
The MIR of a file without type errors MUST satisfy its structural invariants: every block ends in a terminator and is reachable, every local used is declared, every assignment's value has its place's type except for an explicit subtyping conversion, projections and call operands are well typed, no operand moves a place through a dereference or a place rooted at a capture local, only values whose type needs a drop are dropped, and every local is storage-live wherever it is used. Elaborated MIR MUST record exactly one capture mode per capture local of each closure body, and MIR as lowered none.

#### Scenario: Error-free MIR
- **WHEN** a file typechecks successfully
- **THEN** its MIR satisfies every structural invariant, and so does its elaborated MIR when it passes the borrow checker
