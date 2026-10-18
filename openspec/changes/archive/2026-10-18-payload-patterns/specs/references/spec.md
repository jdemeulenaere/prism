# References Specification Delta

## ADDED Requirements

### Requirement: Borrowing Bindings Borrow the Subject's Parts
A binding of a borrowed subject's part MUST be a borrow of that part, carrying the subject's loan. It is shared for a `&E` subject and mutable for a `&mut E` one. Like any borrow, it lasts until its last use, so assigning, moving or mutably borrowing the subject while it is live MUST be rejected as for any other borrow (`E5003` to `E5005`).

#### Scenario: Assigning the subject while a binding is live
- **WHEN** `enum Msg { Text(String), Empty }` and `fun len(s: &String): Int = 1` are declared and `fun f(m: Msg): Int { var v = m; return when (&v) { Msg.Text(val s) -> { v = Msg.Empty; len(s) }; Msg.Empty -> 0 } }` is checked
- **THEN** the borrow checker emits `E5004` at the assignment to `v`, labelling the borrow `&v` and the later use of `s`

#### Scenario: A binding's borrow ends at its last use
- **WHEN** with that `Msg` and `len`, `fun f(m: Msg): Int { var v = m; return when (&v) { Msg.Text(val s) -> { val n = len(s); v = Msg.Empty; n }; Msg.Empty -> 0 } }` is checked
- **THEN** the file passes the borrow checker

#### Scenario: A mutable binding is exclusive
- **WHEN** with that `Msg`, `fun count(m: &Msg): Int = 1` and `fun f(m: &mut Msg): Int = when (m) { Msg.Text(val s) -> { val n = count(m); *s = "x"; n }; Msg.Empty -> 0 }` are checked
- **THEN** the borrow checker emits `E5003` at `m` in `count(m)`, labelling the mutable borrow of the payload and its later use in `*s = "x"`

### Requirement: Disjoint Payloads Are Borrowed Independently
Borrows of different payloads of the same variant MUST NOT conflict, as borrows of different fields do not. Payloads of different variants share storage, so their places MUST overlap.

#### Scenario: Two mutable payload bindings
- **WHEN** `enum Pair { Of(String, String) }` and `fun swap(a: &mut String, b: &mut String) {}` are declared and `fun f(p: &mut Pair) { when (p) { Pair.Of(val a, val b) -> swap(a, b) } }` is checked
- **THEN** the file passes the borrow checker
