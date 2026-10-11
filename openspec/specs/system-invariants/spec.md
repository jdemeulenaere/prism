# Prism System Invariants

## Purpose
The system invariants define the permanent, non-negotiable architectural rules of the Prism programming language that all compiler components, tooling, and runtime layers must uphold.

## Requirements

### Requirement: Single File as Compilation Unit and Explicit Export Boundaries
Any `.prism` source file MUST compile independently into native machine code, with default private visibility and explicit types on exported boundaries.

#### Scenario: Default private visibility
- **WHEN** a declaration omits a visibility modifier
- **THEN** it is visible only within its declaring source file

#### Scenario: Explicit types on exported signatures
- **WHEN** a declaration is marked `public` or `package`
- **THEN** it MUST specify explicit parameter and return types to permit independent Phase 1 ABI extraction

### Requirement: Zero Mandatory Build Files and Zero Marker Files
A Prism workspace MUST require no build configuration files (`build.gradle`, `Cargo.toml`, `CMakeLists.txt`) and no marker files (`.prism-root`, `.git`).

#### Scenario: Pure path workspace root discovery
- **WHEN** the compiler discovers a project root
- **THEN** it derives the root path strictly by matching the file path against its declared package statement

### Requirement: Deterministic Hermetic Builds and Compilation Avoidance
The compilation artifact of any source file MUST be a pure, deterministic function of the file content, direct import ABI hashes, compiler version, and target triple.

#### Scenario: Identical inputs produce cache hit
- **WHEN** none of the compilation inputs have changed
- **THEN** compilation MUST be avoided completely

### Requirement: Kotlin Syntax Familiarity without Grammatical Ambiguity
Prism syntax MUST provide Kotlin-like ergonomics while maintaining unambiguous, deterministic Pratt parsing.

#### Scenario: Familiar expression and declaration constructs
- **WHEN** writing Prism code using `fun`, `val`, `var`, `struct`, `enum`, `when`, `if`, and `T?`
- **THEN** the syntax parses deterministically without grammatical ambiguities

### Requirement: Pure Zero-Cost Memory Safety
The language MUST guarantee memory safety through single-owner value semantics and deterministic destruction without a runtime GC, without hidden ARC, and without explicit lifetime annotations (`'a`).

#### Scenario: Deterministic cleanup with zero runtime overhead
- **WHEN** an owned resource leaves lexical scope
- **THEN** it is freed deterministically with zero GC pauses and zero reference-counter increments or decrements

### Requirement: First-Class Compose Primitives and Native Hot Reload
The language and runtime MUST provide native support for declarative UI via the `compose` keyword and lightweight `Copy` slot-table state handles.

#### Scenario: Hot reload function pointer swapping
- **WHEN** a composable function body is edited during development
- **THEN** the Tier 1 JIT swaps the function pointer in memory while preserving the existing slot-table UI state
