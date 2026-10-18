# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Implicit Closure Parameters Are Locals
A closure's implicit `it` MUST be recorded as the closure's one parameter: a binding named `it`, of the expected parameter's type, marked as an implicit closure parameter, declared at the closure's `{`. Its uses MUST be ordinary uses of that binding, and nothing else in the tree MUST differ from the closure `{ it -> ... }`.

#### Scenario: Implicit parameter in the tree
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` is declared and `prism dump-typed-tree` runs on a file adding `fun g(k: Int): Int = apply { it + k }`
- **THEN** the closure lists the parameter `it`, the body's locals list `it` of type `Int` as an implicit closure parameter at the closure's `{`, the closure captures `k` only, and `it` in the body is a use of that local
