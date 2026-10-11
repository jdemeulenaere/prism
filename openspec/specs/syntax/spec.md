# Prism Syntax & Grammar Specification

## Purpose
The syntax specification defines the concrete lexical and grammatical rules of the Prism programming language, ensuring Kotlin-like developer ergonomics combined with unambiguous, fast Pratt parsing.

## Requirements

### Requirement: Default Private Visibility and Export Modifiers
The compiler MUST treat any top-level declaration without an explicit visibility modifier as `private` to its declaring source file.

#### Scenario: Declaration without visibility modifier
- **WHEN** a function, struct, or enum is declared without a visibility modifier (`fun foo(): Int = 42`)
- **THEN** the symbol is accessible only within the declaring source file and is excluded from the exported ABI

#### Scenario: Package visibility modifier
- **WHEN** a declaration is annotated with the `package` modifier (`package fun helper(): Int`)
- **THEN** the symbol is accessible only by files sharing the identical package directory

#### Scenario: Public visibility modifier
- **WHEN** a declaration is annotated with the `public` modifier (`public fun api(): Int`)
- **THEN** the symbol is accessible across all packages via module imports

### Requirement: Mandatory Explicit Types on Exported Declarations
All declarations marked `public` or `package` MUST declare explicit parameter types and explicit return types.

#### Scenario: Exported function with explicit types
- **WHEN** a `public` or `package` function declares all parameter types and a return type (`public fun add(a: Int, b: Int): Int = a + b`)
- **THEN** the declaration is valid and its ABI signature can be extracted in Phase 1 without inspecting the body

#### Scenario: Exported function omitting return type
- **WHEN** a `public` or `package` function omits its return type (`public fun compute(x: Int) = x * 2`)
- **THEN** the compiler MUST emit a compile error requiring an explicit return type

#### Scenario: Private function with inferred return type
- **WHEN** a private function omits its return type (`fun localCompute(x: Int) = x * 2`)
- **THEN** type inference is permitted and the compiler infers the return type during body typechecking

### Requirement: Variable Bindings and Immutability
Variables MUST be immutable by default using `val`, while mutable variables MUST be explicitly declared with `var`.

#### Scenario: Reassigning immutable val binding
- **WHEN** code attempts to reassign a variable declared with `val` (`val x = 10; x = 20`)
- **THEN** the compiler MUST reject the reassignment with a compile error

#### Scenario: Reassigning mutable var binding
- **WHEN** code reassigns a variable declared with `var` (`var x = 10; x = 20`)
- **THEN** the mutation is valid

### Requirement: Value Structs and Algebraic Enums
The language MUST provide value structs that own their fields inline, and algebraic enums supporting payloads and recursive types via `Heap<T>`.

#### Scenario: Value struct definition
- **WHEN** a struct is declared with fields (`struct Point { val x: Double, val y: Double }`)
- **THEN** instances of `Point` are stored inline by value without heap indirection

#### Scenario: Recursive enum with Heap indirection
- **WHEN** an enum variant contains recursive instances wrapped in `Heap<T>` (`enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }`)
- **THEN** the enum layout is finite and accepted by the compiler

### Requirement: Expression-Oriented Control Flow
Control flow constructs (`if` and `when`) MUST function as first-class expressions producing values.

#### Scenario: If expression assignment
- **WHEN** an `if-else` block is assigned to a variable (`val result = if (condition) 10 else 20`)
- **THEN** both branches are typechecked to a common supertype and the value is assigned

#### Scenario: When pattern matching
- **WHEN** a `when` expression matches over values or ranges (`when (x) { 0 -> "zero", in 1..10 -> "small", else -> "large" }`)
- **THEN** the branches are evaluated in order and the matching branch expression is returned

### Requirement: Null Safety and Optional Types
The language MUST enforce compile-time null safety where nullable types are designated with `T?`.

#### Scenario: Assigning null to non-nullable type
- **WHEN** code attempts to assign `null` to a non-nullable variable (`val name: String = null`)
- **THEN** the compiler MUST reject the assignment at compile time

#### Scenario: Safe navigation and Elvis operator
- **WHEN** code accesses a property on a nullable reference using `?.` and provides a fallback with `?:` (`val len = name?.length ?: 0`)
- **THEN** the expression safely evaluates to 0 when `name` is null

### Requirement: First-Class Compose UI Keyword
The grammar MUST recognize `compose` as a native modifier on function declarations (`compose fun`).

#### Scenario: Composable function declaration
- **WHEN** a function is declared with `compose fun CounterView()`
- **THEN** the compiler parses it as a composable UI function eligible for slot-table transformation

## Design Rationale
Prism chooses Kotlin-like ergonomics combined with value ownership semantics to eliminate GC pauses without lifetime annotation complexity.
