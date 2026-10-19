## MODIFIED Requirements

### Requirement: The Toolkit Modules Follow Compose's Artifacts
The toolkit MUST be declared by four bundled modules, one per Jetpack Compose artifact: `/prism/compose/ui` (`compose-ui`), `/prism/compose/foundation` (`compose-foundation`), `/prism/compose/layout` (`compose-foundation-layout`) and `/prism/compose/material3` (`compose-material3`). Each name MUST be declared by the module of the artifact that declares it in Compose, so that a file uses it only after importing that module.

#### Scenario: UI toolkit is imported
- **WHEN** a file without imports declares `compose fun Screen() { Column { } }`
- **THEN** the type checker emits `E4001` for `Column`, with help naming `import /prism/compose/layout`, and the file typechecks once it declares `import /prism/compose/layout`

#### Scenario: Names live in their Compose module
- **WHEN** a file declares `import /prism/compose/ui`, `import /prism/compose/layout` and `compose fun Screen() { Box(modifier = Modifier.background(Color.Red)) { } }`
- **THEN** the type checker emits `E4006` for `background`, with help naming `import /prism/compose/foundation`, and the file typechecks once it also declares `import /prism/compose/foundation`
