# Code Generation Delta

## ADDED Requirements

### Requirement: UI Entry Points Are Checked Like Runtime Entry Points
The JIT MUST resolve `prism-ui`'s entry points by symbol, as it resolves the runtime's and the compose runtime's. The backend MUST find every external that names one in `prism-ui`'s table, and MUST compare its machine signature with the table's (code-generation, "Externals Match the Runtime").

#### Scenario: A UI entry point declared with the wrong signature
- **WHEN** a library test module declares `private external fun prism_ui_set_enabled(node: Int, enabled: Double)` and a program calls it
- **THEN** compiling the program reports `E6005`, saying that the entry point `prism_ui_set_enabled` takes `i8` where the declaration passes `f64`

### Requirement: One-Word Intrinsic Structs Travel as Words
A value of an intrinsic struct that the catalog lays out as one word (`NodeId`, `State<T>`, `MutableState<T>`, `RecomposeScope`, `Composition`) MUST be passed to and returned from functions and externals as a 64-bit word, not by address. Its representation is fixed by the catalog, so no signature depends on a declared type's layout.

#### Scenario: An external taking a node handle
- **WHEN** `prism dump-clif` runs on a library test module declaring `private external fun prism_ui_set_checked(node: NodeId, checked: Bool)` and a call of it
- **THEN** the call passes two machine values, a 64-bit word and an 8-bit `Bool`, and no address
