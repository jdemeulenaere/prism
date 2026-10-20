## MODIFIED Requirements

### Requirement: Lists Grow by Doubling
A vararg's list of `n` elements (`listOf`'s among them) MUST have capacity exactly `n`, and no buffer for `n = 0`. A full list's `add` or `insert` MUST first grow it to twice its capacity, or to 4 from 0, moving elements bitwise through the runtime, so `add` takes amortized constant time; otherwise `add` MUST write in place without calling the runtime. `removeAt`, `removeLast` and `clear` MUST keep the capacity. A buffer size beyond a machine word MUST be the runtime error "out of memory".

#### Scenario: Growing during a million adds
- **WHEN** `fun main() { var xs = listOf<Int>(); for (i in 0..<1000000) xs.add(i); var sum = 0; for (x in xs) sum += x; println(sum) }` runs with `--check-allocations`
- **THEN** it prints `499999500000`, reports no live allocation and exits with status 0
