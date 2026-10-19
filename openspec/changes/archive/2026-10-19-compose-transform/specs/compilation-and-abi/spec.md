# Compilation and ABI Delta

## ADDED Requirements

### Requirement: Intrinsic Structs in the ABI
An exported intrinsic struct MUST be an ABI entry with its name, type parameters and markers, marked intrinsic, covered by the ABI hash and shown by `prism abi` with its modifier (`public intrinsic struct RecomposeScope : Copy`). Its layout MUST NOT be part of the ABI: it comes from the compiler, whose version every artifact key holds.

#### Scenario: The ABI of the compose module
- **WHEN** the ABI of `/prism/compose/runtime` is extracted
- **THEN** it holds `public intrinsic struct Composition`, `public intrinsic struct RecomposeScope : Copy` and `public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T`, and removing the `Copy` marker of `RecomposeScope` changes the module's ABI hash
