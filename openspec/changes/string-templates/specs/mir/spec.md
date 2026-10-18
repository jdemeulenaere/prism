# MIR Specification Delta

## ADDED Requirements

### Requirement: Templates Build One String
A template MUST be lowered to one template rvalue assigned to a `String` place, holding its parts in source order: each text run as a constant text, each `String` part as a shared borrow of its place with the `operator` origin (as for the operands of `+`), and each scalar part as an operand of its type. The lowering MUST NOT call `toString()` for scalar parts nor concatenate. `prism dump-mir` MUST print the rvalue as `template(...)`, with text parts as quoted strings and value parts as operands.

#### Scenario: A template is one rvalue
- **WHEN** `fun f(n: Int, s: &String): String = "$n: $s"` is lowered
- **THEN** the MIR of `f` takes an `operator` borrow of the referent of `s` into a temporary and assigns its result a template over a copy of `n`, the text `": "` and a move of that temporary, printed as `template(copy _1, ": ", move _3)` when `_3` is the temporary, with no call

### Requirement: Template Parts Are Evaluated in Order
The parts of a template MUST be evaluated left to right: a scalar part that a later part may write MUST first be copied to a temporary, as for the operands of a call. A `toString()` call that gives a part its text form MUST be a call before the rvalue, writing a temporary that is dropped at the end of the statement.

#### Scenario: A later part writing a scalar part
- **WHEN** `fun bump(r: &mut Int): Int { *r = r + 1; return r }` and `fun f(): String { var n = 1; return "$n ${bump(&mut n)}" }` are lowered
- **THEN** the MIR of `f` copies `n` to a temporary before the call of `bump`, and the template reads that temporary, then the result of the call

#### Scenario: A toString part is called before the template
- **WHEN** `struct Point : Copy { val x: Int }`, `fun Point.toString(): String = "p"` and `fun f(p: Point): String = "at $p"` are lowered
- **THEN** the MIR of `f` calls `Point.toString` into a temporary, assigns the template over the text `at ` and a borrow of that temporary, and drops the temporary after the template
