## ADDED Requirements

### Requirement: Compositions and Scopes Are Library Structs
`Composition` and `RecomposeScope` MUST be laid out as the structs `/prism/compose/runtime` declares, one `Int` field each. Dropping a `Composition` MUST run its drop hook, which disposes the composition, through drop glue like any hooked type, with no special case in the backend.

#### Scenario: Layout of a composition
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Screen { val c: Composition, val s: RecomposeScope }` and `fun f(s: Screen) {}`
- **THEN** it shows `Screen` with size 16 and alignment 8, and `prism/compose/runtime::Composition` and `prism/compose/runtime::RecomposeScope` with size 8, each holding the field it declares

#### Scenario: The drop glue of a composition calls its hook
- **WHEN** `prism dump-clif` runs on `struct Screen { val c: Composition }` and `fun f(s: Screen) {}`
- **THEN** the drop glue of `Screen` calls that of `prism/compose/runtime::Composition`, which calls the hook `prism/compose/runtime::Composition.drop` through its slot, and nothing in the unit calls `prism_compose_dispose` directly

### Requirement: Handler Ids Compare Through Their Equality Operator
`==` of two `HandlerId`s, and its part in the equality glue of every type that holds one, MUST call `HandlerId`'s `equals` operator, which compares the two handlers through `prism_ui_handlers_equal`.

#### Scenario: Comparing handler ids
- **WHEN** `prism dump-clif` runs on a library test module `/prism/compose/test`, of the package `prism.compose`, that imports `ui` and declares `fun same(a: HandlerId, b: HandlerId): Bool = a == b` and `struct Pair : Copy { val h: HandlerId }`, with a comparison of two `Pair`s
- **THEN** the body of `same` and the equality glue of `Pair` each call `prism/compose/ui::HandlerId.equals` through its slot

### Requirement: Intrinsic State Handles Travel as Words
A value of `State<T>`, `MutableState<T>` or `WeakReference<T>`, the intrinsic structs the catalog lays out as one word, MUST be passed to and returned from functions and externals as a 64-bit word, not by address. Its representation is fixed by the catalog, so no signature depends on a declared type's layout.

#### Scenario: A weak reference passed as a word
- **WHEN** `prism dump-clif` runs on `fun f(r: WeakReference<MutableState<Int>>): WeakReference<MutableState<Int>> = r`
- **THEN** `main::f` takes one 64-bit word and returns one, with no address parameter

## REMOVED Requirements

### Requirement: Intrinsic Structs Are Laid Out by the Compiler
**Reason**: `Composition` and `RecomposeScope` are no longer intrinsic structs: they are library structs with a private field, and `Composition`'s disposal is its drop hook (design D6, D7, D13).
**Migration**: "Compositions and Scopes Are Library Structs" replaces it, with the same layout scenario.

### Requirement: Intrinsic Structs Compared Through a Runtime Entry
**Reason**: `HandlerId`, the only intrinsic struct with an equality entry, is a library struct whose `equals` operator calls `prism_ui_handlers_equal` (design D8).
**Migration**: "Handler Ids Compare Through Their Equality Operator" replaces it, with the same comparison scenario.

### Requirement: One-Word Intrinsic Structs Travel as Words
**Reason**: `NodeId`, `HandlerId`, `RecomposeScope` and `Composition` are library structs, passed by address like every declared struct (design D8, D13); only the state handles are still intrinsic.
**Migration**: "Intrinsic State Handles Travel as Words" replaces it; its scenario "An external taking a node handle" becomes "A weak reference passed as a word", since the UI externals now take `Int` words.
