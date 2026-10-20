## ADDED Requirements

### Requirement: Operator Syntax Is Printed
`prism fmt` MUST print `operator` in the canonical modifier order (visibility, `external` or `intrinsic`, `operator`, `compose`, receiver mode), index lists as `a[i, j]` with one space after each comma, and `in` and `!in` with one space on each side, breaking an index list as an argument list.

#### Scenario: Operator syntax formatted
- **WHEN** `prism fmt -` reads `public  operator mut fun Bag.get( index :Int ):&mut Item = &mut items[index]`, `val c = m[ 1,2 ]` and `val b = x  !in xs`
- **THEN** it writes `public operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]`, `val c = m[1, 2]` and `val b = x !in xs`
