## ADDED Requirements

### Requirement: Intrinsic Operators
An `intrinsic` declaration MAY carry `operator` when the catalog lists the intrinsic as an operator: `List<T>.contains` of `/prism/collections` is declared `public intrinsic operator fun <T> List<T>.contains(element: &T): Bool`. A declaration whose `operator` flag differs from the catalog's MUST be rejected with `E4029`, as any other signature difference.

#### Scenario: The list membership operator
- **WHEN** a file declares `fun f(xs: &List<Int>): Bool = 3 in xs`
- **THEN** the file typechecks, `in` calling the intrinsic member `contains` of `List`

#### Scenario: An intrinsic's operator flag must match the catalog
- **WHEN** a library test module declares `public intrinsic fun <T> List<T>.contains(element: &T): Bool` without `operator`, or `public intrinsic operator fun <T> List<T>.indexOf(element: &T): Int`
- **THEN** the type checker emits `E4029` for each, showing the signature the compiler provides
