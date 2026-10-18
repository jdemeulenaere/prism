# Code Generation Specification Delta

## ADDED Requirements

### Requirement: A Template Is One Runtime Call
The backend MUST lower a template rvalue to one call of a runtime function receiving the destination and a table of the parts in order: text parts as the address, byte length and character count of the code unit's literal data, string parts as the address of the borrowed string, and scalar parts as their values. A template MUST NOT call any other string or conversion function of the runtime, and MUST NOT be rejected with `E6001`.

#### Scenario: A template calls the runtime once
- **WHEN** `fun label(n: Int, d: Double, s: &String): String = "Count: $n of $s at $d"` is declared and `prism dump-clif main.prism` runs
- **THEN** the body of `label` calls the runtime's template function once and calls no runtime concatenation or `toString()` function

### Requirement: A Template Allocates Its Result Once
The runtime MUST format a template's scalar parts itself, with the text their built-in `toString()` gives, and MUST build the result with exactly one allocation, of exactly its length in bytes, or with none when the result is empty.

#### Scenario: A template allocates its result once
- **WHEN** `fun label(n: Int, s: &String): String = "Count: $n, ${n * 2} of $s"` is compiled and called with `7` and a borrowed literal `"x"`, then its result is dropped, under the allocation check
- **THEN** the result is `Count: 7, 14 of x`, exactly one allocation is made, and none is live afterwards
- **WHEN** `fun empty(s: &String): String = "$s$s"` is called with a borrowed empty literal
- **THEN** the result is empty and no allocation is made
