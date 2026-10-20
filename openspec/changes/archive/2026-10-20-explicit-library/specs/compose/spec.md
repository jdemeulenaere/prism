## ADDED Requirements

### Requirement: The Compose Runtime Module
The bundled module `/prism/compose/runtime` MUST declare `Composition`, `composition`, `setContent`, `recompose` and `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope` and `invalidate`, and the state declarations `State`, `MutableState`, `mutableStateOf`, `value`, `update`, `WeakReference`, `weak`, `get` and `SideEffect`. A file MUST use them only after importing the module, and a file that does not import it MAY declare any of those names as its own.

#### Scenario: Compose primitives are imported
- **WHEN** a file declaring `import /prism/compose/runtime` declares `compose fun Counter() { val id = remember { 7 }; val s = currentRecomposeScope() }` and `fun main() { val c = composition { Counter() } }`, and the same declarations are checked without the import
- **THEN** the first typechecks, `remember` and `currentRecomposeScope` resolve to `/prism/compose/runtime`, and `prism run` exits with status 0; the second is rejected with `E4001` for `remember`, `currentRecomposeScope` and `composition`, each with help naming `import /prism/compose/runtime`

#### Scenario: A file's own `key`
- **WHEN** a file declares `fun key(n: Int): Int = n` and `fun f(): Int = key(3)`
- **THEN** it typechecks, and the call resolves to the file's own `key`

#### Scenario: A file's own mutableStateOf
- **WHEN** a file declares `fun mutableStateOf(n: Int): Int = n + 1` and `fun main() { println(mutableStateOf(1)) }`
- **THEN** it runs and prints `2`: the call resolves to the file's own `mutableStateOf`

## MODIFIED Requirements

### Requirement: The Composer Protocol Module
The bundled library module `/prism/composer` MUST declare the composer protocol for code written by hand in the form composables are lowered to. Composers, compositions and scopes MUST be `Int` words there. Each public function MUST behave as the requirement it names describes, through a private external of the compose runtime.

#### Scenario: The protocol module is imported
- **WHEN** a file without imports declares `fun main() { val c = startComposition { k -> } }`
- **THEN** the type checker emits `E4001` for `startComposition`, and the file runs once it declares `import /prism/composer` and disposes `c`

## REMOVED Requirements

### Requirement: The Compose Prelude
**Reason**: There is no prelude (user direction 2026-10-20); its scenario "Compose primitives need no import" no longer holds.
**Migration**: See "The Compose Runtime Module", which keeps the declarations and the scenarios "A file's own `key`" and "A file's own mutableStateOf", and replaces "Compose primitives need no import" with "Compose primitives are imported".
