# References Specification Delta

## ADDED Requirements

### Requirement: Template Parts Are Borrowed Until the Template Is Built
A template MUST read each `String` part in place, through a shared borrow that lasts until the template's result is built, and MUST NOT move it; a computed part is a temporary living until the end of the enclosing statement. A part whose evaluation needs a mutable borrow of a string that an earlier part of the same template borrows MUST be rejected as a borrow conflict (`E5003`), as for the operands of `+`.

#### Scenario: Borrowed parts stay usable
- **WHEN** `fun f(s: String, r: &mut String): String { val t = "$s and $r"; *r = t; return s }` is checked
- **THEN** the file typechecks and borrow-checks successfully

#### Scenario: A later part mutating a borrowed part rejected
- **WHEN** `fun grow(s: &mut String): Int { *s = s + "!"; return s.length }` and `fun f() { var s = "a"; val t = "$s ${grow(&mut s)}" }` are checked
- **THEN** the borrow checker emits `E5003` at `&mut s`, labelling the first part's borrow of `s` and its later use, where the template is built

### Requirement: Scalar Template Parts Are Read at Their Position
A scalar part MUST be read at its position in the template, so a later part writing the same place, through a mutable borrow, does not change the earlier part's text and is not a borrow conflict.

#### Scenario: A later part writing an earlier scalar part
- **WHEN** `fun bump(r: &mut Int): Int { *r = r + 1; return r }` and `fun f(): String { var n = 1; return "$n ${bump(&mut n)} $n" }` are checked
- **THEN** the file typechecks and borrow-checks successfully, the first part reading `n` before `bump` writes it
