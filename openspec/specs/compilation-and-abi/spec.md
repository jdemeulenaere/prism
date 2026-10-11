# Prism Compilation Pipeline, ABI Hashing & Execution Tiers

## Purpose
The compilation and ABI specification defines the file-level compilation boundaries, two-phase compilation pipeline, BLAKE3 ABI hashing for compilation avoidance, generic monomorphization rules, and two-tier native execution backends.

## Requirements

### Requirement: Single File as Compilation Unit
Every `.prism` source file MUST be compiled into native machine code independently, with interface boundaries defined strictly by exported declarations.

#### Scenario: Compiling independent source files
- **WHEN** compiling multiple source files in a package
- **THEN** each file is lowered and code-generated as a standalone compilation unit

### Requirement: Two-Phase Compilation Pipeline
The compiler MUST separate compilation into Phase 1 (declaration and ABI extraction) and Phase 2 (body lowering and machine code generation).

#### Scenario: Phase 1 surface extraction
- **WHEN** the compiler scans source files
- **THEN** it extracts public and package struct definitions, enums, and function signatures to produce the exported ABI without typechecking function bodies

#### Scenario: Phase 2 body lowering
- **WHEN** Phase 1 completes
- **THEN** function bodies are typechecked and compiled in parallel against the imported ABI interface hashes

### Requirement: BLAKE3 ABI Hash Computation and Compilation Avoidance
The compiler MUST compute an authoritative cryptographic hash (BLAKE3) of exported declarations, excluding private symbols and non-generic function bodies.

#### Scenario: Modifying non-generic function body
- **WHEN** a developer modifies the internal implementation of a public function without changing its signature
- **THEN** the file's exported ABI hash remains unchanged, and zero downstream files that import it are recompiled

#### Scenario: Modifying public function signature
- **WHEN** a developer changes the parameter types or return type of a public function
- **THEN** the file's exported ABI hash changes, triggering recompilation of direct downstream importers

#### Scenario: Modifying private helper function
- **WHEN** a private helper function is added, removed, or modified in a source file
- **THEN** the exported ABI hash is unaffected and downstream files remain cached

### Requirement: Monomorphization Invalidation for Generics
Exported generic functions MUST record a Monomorphization Hash of their normalized AST in their ABI signature.

#### Scenario: Changing generic function body
- **WHEN** the body of an exported generic function changes
- **THEN** its Monomorphization Hash changes, triggering re-monomorphization only in downstream call sites that instantiate that generic

### Requirement: Two-Tier Native Execution Engine
The compiler MUST support two distinct native execution backends: Tier 1 (Cranelift JIT) for development and Tier 2 (LLVM AOT) for release.

#### Scenario: Tier 1 instant script execution
- **WHEN** the user runs `prism run <file.prism>`
- **THEN** the compiler compiles the file into memory using Cranelift JIT in under 15ms without invoking an external toolchain

#### Scenario: Tier 1 hot reload with disabled cross-file inlining
- **WHEN** code is running in development mode under Tier 1
- **THEN** cross-file inlining across compilation units is disabled, allowing function-pointer tables to be dynamically patched during hot reload

#### Scenario: Tier 2 release optimization
- **WHEN** the user builds with `prism build --release`
- **THEN** the compiler invokes the LLVM backend with whole-program optimization and Link-Time Optimization (LTO)
