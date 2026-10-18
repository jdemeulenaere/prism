# Formatting Delta

## ADDED Requirements

### Requirement: Writable Intrinsic Properties Are Printed
`prism fmt` MUST print an `intrinsic var` declaration on one line, as it prints an `intrinsic val` declaration, with `var` in place of `val`.

#### Scenario: Writable intrinsic properties formatted
- **WHEN** `prism fmt -` reads `public  intrinsic var<T>MutableState<T>.value:T`
- **THEN** it prints `public intrinsic var <T> MutableState<T>.value: T` and a newline
