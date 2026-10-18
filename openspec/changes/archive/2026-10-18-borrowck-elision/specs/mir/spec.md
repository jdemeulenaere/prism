# MIR Specification Delta

## MODIFIED Requirements

### Requirement: Calls Name Their Callee
A call MUST name its callee (a declaration path for an own or imported function or extension, the built-in `clone`, a function-typed place, or a default value's generator), pass one operand per parameter in parameter order, the receiver first, and be marked composable when its callee is composable. The declaration path of an extension names its receiver type, so the first operand of a call of an extension, and the first argument of an extension's body, MUST be its receiver; no other call has one.

#### Scenario: Imported function named by its module
- **WHEN** `models/user.prism` declares `public fun greet(n: Int): Int = n`, and a file importing `/models/user` declares `fun hello(): Int = greet(1)`
- **THEN** the MIR of `hello` calls `models/user::greet` with the constant `1`

#### Scenario: Composable calls are marked
- **WHEN** `compose fun Label() {}`, `fun log() {}` and `compose fun Screen() { Label(); log() }` are declared
- **THEN** the body of `Screen` is composable, its call of `Label` is marked composable, and its call of `log` is not

#### Scenario: An extension call names its receiver
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }` and `public fun Team.captain(o: &String): &String = &lead`, and a file importing `/lib/team` declares `fun f(t: Team, s: String): Int { val c = t.captain(&s); return 1 }`
- **THEN** the MIR of `f` calls `lib/team::Team.captain`, whose path names the receiver type `Team`, with the receiver borrow of `t` first and the borrow of `s` second

### Requirement: Optional Mutable Borrows Are Reborrowed
Where a `(&mut T)?` place is lent as a `(&T)?`, the MIR MUST read its tag and pass `null`, or a shared borrow of the dereferenced payload wrapped as non-null, without moving the place. Where a `(&mut T)?` place is lent as a `(&mut T)?`, the MIR MUST likewise read its tag and pass `null`, or a mutable borrow of the dereferenced payload wrapped as non-null, without moving the place.

#### Scenario: Lending an optional mutable borrow twice
- **WHEN** `struct User { var name: String }` and `fun show(n: (&User)?): Int = 1` are declared and `fun f(m: (&mut User)?) { show(m); show(m) }` is lowered
- **THEN** before each call the tag of `m` is read, and `show` receives either `null` or a wrapped shared argument borrow of the dereference of `m`'s payload; `m` is never moved

#### Scenario: Lending an optional mutable borrow as mutable twice
- **WHEN** `struct User { var name: String }` and `fun edit(n: (&mut User)?) {}` are declared and `fun f(m: (&mut User)?) { edit(m); edit(m) }` is lowered
- **THEN** before each call the tag of `m` is read, and `edit` receives either `null` or a wrapped mutable argument borrow of the dereference of `m`'s payload; `m` is never moved
