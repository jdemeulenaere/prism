# Formatting Specification Delta

## ADDED Requirements

### Requirement: Closure Headers Are Kept as Written
The formatter MUST print a closure's header exactly when it was written, including the empty header `{ -> }`, and MUST NOT add, remove or rename a closure parameter, so a closure using the implicit `it` stays without a header.

#### Scenario: Empty headers and implicit it kept
- **WHEN** formatting `val f: (Int) -> Int = {it+1}`, `run {->1}`, `run {  -> }` and `xs.forEach {it -> println(it)}` inside a function
- **THEN** the results are `val f: (Int) -> Int = { it + 1 }`, `run { -> 1 }`, `run { -> }` and `xs.forEach { it -> println(it) }`
