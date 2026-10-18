# Patterns Specification Delta

## ADDED Requirements

### Requirement: Patterns Look Through Boxes
A constructor, value or range pattern checked against a subject or part of type `Heap<T>`, behind any boxes and references, MUST be checked against `T` and match the box's contents. A wildcard or a binding at such a position MUST take the box itself, with the binding mode of its subject.

#### Scenario: Evaluating an expression tree recursively
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` is declared and `fun eval(e: &Expr): Int = when (e) { Expr.Lit(val n) -> n; Expr.Add(val l, val r) -> eval(l) + eval(r) }` is checked
- **THEN** the file typechecks, `n` has type `Int`, `l` and `r` have type `&Heap<Expr>`, each lending its contents to `eval`, and the `when` is exhaustive

#### Scenario: A nested pattern through a box
- **WHEN** with that `Expr`, `fun isZeroSum(e: &Expr): Bool = when (e) { Expr.Add(Expr.Lit(0), Expr.Lit(0)) -> true; else -> false }` is checked
- **THEN** the file typechecks, each `Expr.Lit(0)` matching the contents of a box

#### Scenario: A boxed subject
- **WHEN** with that `Expr`, `fun f(h: Heap<Expr>): Int = when (h) { Expr.Lit(val n) -> n; Expr.Add(_, _) -> 0 }` and `fun g(h: &Heap<Int>): Int = when (h) { 0 -> 1; in 1..9 -> 2; else -> 3 }` are checked
- **THEN** both typecheck, the patterns of `f` matching the contents of `h` without moving it, since no binding moves anything

#### Scenario: Mutable bindings through a box
- **WHEN** `enum Counter { Count(Int) }` is declared and `fun bump(h: &mut Heap<Counter>) { when (h) { Counter.Count(val n) -> *n += 1 } }` is checked
- **THEN** the file typechecks, `n` having type `&mut Int`

### Requirement: Taking Apart Through a Box Unboxes It
When a value is taken apart and a by-value non-`Copy` binding lies inside the contents of a box part, or of a boxed subject, the box MUST be unboxed: its contents taken apart and the box freed. The move rules of taking apart MUST apply to the outermost subject unchanged.

#### Scenario: Moving a payload out through a box
- **WHEN** `enum Doc { Text(String), Wrapped(Heap<Doc>) }` is declared and `fun inner(d: Doc): String = when (d) { Doc.Wrapped(Doc.Text(val s)) -> s; else -> "" }` and `fun top(h: Heap<Doc>): String = when (h) { Doc.Text(val s) -> s; else -> "" }` are checked
- **THEN** both typecheck, `s` having type `String`, and `d` and `h` are moved by the branches that bind `s`

#### Scenario: Taking apart a borrowed box rejected
- **WHEN** with that `Doc`, `struct Holder { val h: Heap<Doc> }` is declared and `fun f(o: Holder): String = when (o.h) { Doc.Text(val s) -> s; else -> "" }` is checked
- **THEN** the type checker emits `E4017`, because the field `h` of `o` cannot be taken apart, with help suggesting `when (&o.h)`

### Requirement: Exhaustiveness Looks Through Boxes
For exhaustiveness and for the uncovered value named by `E4009`, a `Heap<T>` subject or part MUST be covered exactly by the patterns that cover `T`, and an uncovered value inside a box MUST be named without any box syntax.

#### Scenario: A box covered by its contents' patterns
- **WHEN** `enum Bit { Zero, One }` and `enum Pair { Of(Heap<Bit>, Heap<Bit>) }` are declared and `fun f(p: &Pair): Int = when (p) { Pair.Of(Bit.Zero, _) -> 0; Pair.Of(Bit.One, Bit.Zero) -> 1; Pair.Of(Bit.One, Bit.One) -> 2 }` is checked
- **THEN** the `when` is exhaustive without `else`

#### Scenario: An uncovered value inside a box
- **WHEN** with that `Bit` and `Pair`, `fun f(p: &Pair): Int = when (p) { Pair.Of(Bit.Zero, _) -> 0; Pair.Of(_, Bit.Zero) -> 1 }` is checked
- **THEN** the type checker emits `E4009`, stating that `Pair.Of(Bit.One, Bit.One)` is not covered
