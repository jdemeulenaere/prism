## ADDED Requirements

### Requirement: Equality Calls a Type's Equality Operator
Where the equality expansion or an equality glue function compares two values of a struct or enum with an `equals` operator, it MUST call that function, through the function table, with the addresses of the two values, instead of expanding its fields or payloads, counting one comparison toward the inline budget. Glue of nullables, boxes, lists, kept values and state records MUST reach it the same way.

#### Scenario: Equality expansion calling equals
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User { val id: Int, val name: String }`, `operator fun User.equals(other: &User): Bool = id == other.id`, `struct Team { val lead: User, val size: Int }` and `fun same(a: &Team, b: &Team): Bool = a == b`
- **THEN** `same` compares `size` inline and calls the function of `User.equals` for `lead`, and no code compares the `name` strings

#### Scenario: List search calling equals
- **WHEN** `prism dump-clif main.prism` runs on a file declaring that `User` and `fun has(xs: &List<User>, u: &User): Bool = xs.contains(u)`
- **THEN** the index-of glue of `List<User>` calls the function of `User.equals` for each element compared
