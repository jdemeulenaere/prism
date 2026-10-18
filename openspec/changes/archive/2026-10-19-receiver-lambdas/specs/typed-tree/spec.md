# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Lambdas With a Receiver Bind It as Their First Parameter
A lambda with a receiver MUST record its receiver as its first parameter: a binding named `this`, of the receiver's parameter type (`&T` or `&mut T`), marked as a closure receiver, declared at the lambda's `{`, followed by its declared or implicit parameters. Uses of `this`, written or implicit, MUST be uses of that binding, and a closure nested in the lambda that uses it MUST capture it.

#### Scenario: A receiver lambda in the tree
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun nest(f: Scope.(Int) -> Int): Int = f(Scope(1), 2)` are declared and `prism dump-typed-tree` runs on a file adding `fun g(k: Int): Int = nest { depth + it + k }`
- **THEN** the closure lists the parameters `this` and `it`, the body's locals list `this` of type `&Scope` as a closure receiver and `it` of type `Int` as an implicit closure parameter, `depth` is field 0 of the dereferenced `this`, and the closure captures `k` only

### Requirement: Calls of Receiver Function Values Are Normalized
A call of a value of a receiver function type MUST be a call of that value whose first argument is the receiver, in each of its three forms: the written first argument of `f(r, a)`, the receiver borrow of `r.f(a)`, or a reborrow of the innermost `this` in `f(a)`.

#### Scenario: Receiver value calls in the tree
- **WHEN** `struct Scope : Copy { val depth: Int }` is declared and `prism dump-typed-tree` runs on a file adding `fun Scope.run(f: Scope.(Int) -> Int): Int { val s = Scope(2); return f(s, 1) + s.f(2) + f(3) }`
- **THEN** each of the three calls is a call of the parameter `f` with two arguments: a shared borrow of `s`, a shared receiver borrow of `s`, and a reborrow of the extension's `this`, each followed by the literal

### Requirement: Context Arguments Lead a Call's Arguments
A call of a function with a context parameter MUST start its arguments with the context argument, a shared borrow of its candidate's referent marked as the context, before the receiver and the other arguments. An extension with a context MUST be named by its module, receiver type, name and context type, dumped `<module>::<receiver>.<name>#context(<type>)`.

#### Scenario: A context call in the tree
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `fun row(f: RowScope.() -> Modifier): Modifier = f(RowScope())` and `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` are declared and `prism dump-typed-tree` runs on a file adding `fun g(): Modifier = row { Modifier(0.0).weight(1.0) }`
- **THEN** the call names its callee `main::Modifier.weight#context(RowScope)`, with three arguments: the context, a shared borrow of the lambda's `this` referent; a shared receiver borrow of the temporary `Modifier(0.0)`; and the literal `1.0`
