# Type System Delta

## ADDED Requirements

### Requirement: Default Values Are Not Composable Contexts
A parameter's default value MUST be checked outside any composable context, also for a parameter of a composable, since its generator runs in the caller before the call. A composable call in a default value MUST be rejected with `E4008`, with a note that default values are not composable.

#### Scenario: A composable call in a default value rejected
- **WHEN** `compose fun Size(): Int = 4` and `compose fun Box(width: Int = Size()) { }` are checked
- **THEN** the type checker reports `E4008` for the call of `Size`, noting that default values are not composable

#### Scenario: A constant default of a composable
- **WHEN** `compose fun Box(width: Int = 4) { }` and `compose fun Screen() { Box() }` are checked
- **THEN** the file typechecks
