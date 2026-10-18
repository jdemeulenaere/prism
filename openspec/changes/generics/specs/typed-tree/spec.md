# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Generic Declarations Record Their Type Parameters
Every generic function, extension, struct and enum in the typed tree MUST record its type parameters, in order, with their bounds. Inside it, a type parameter MUST be a type of its own, distinct from every struct, enum and opaque name. Each generic struct and enum in the file's types MUST record its type parameters and marker parameters, and its fields or variants with the declared types.

#### Scenario: A generic function and struct in the tree
- **WHEN** `struct Box<T> : Copy { val v: T }` and `fun <T : Copy> wrap(x: T): Box<T> = Box(x)` are declared
- **THEN** the typed tree records `Box` with the type parameter `T` and the marker parameter `T`, and `wrap` with the type parameter `T` bounded by `Copy`, whose parameter `x` and construction of `Box<T>` are typed with that parameter, and `prism dump-typed-tree` prints `fun <T : Copy>` before the function's path

### Requirement: Generic Calls Record Their Type Arguments
A call of a generic function or extension MUST record its type arguments, written or inferred, in declaration order, together with the callee's declared parameter types, so each argument's type is its declared parameter type with the type arguments substituted. A construction of a generic struct or variant MUST have the instantiated type, which gives its type arguments.

#### Scenario: Inferred type arguments are recorded
- **WHEN** `fun <T> id(x: T): T = x` and `struct Box<T>(val v: T)` are declared and `fun f(): Box<Int> = Box(id(1))` is checked
- **THEN** the call of `main::id` records the type argument `Int` and the declared parameter type `T`, its argument is the literal `1` of type `Int`, and the construction has type `Box<Int>`; `prism dump-typed-tree` prints the call as `main::id<Int>`

#### Scenario: Type arguments inside a generic body
- **WHEN** `fun <T> id(x: T): T = x` and `fun <U> twice(x: U): U = id(id(x))` are declared
- **THEN** both calls of `main::id` in `twice` record the type argument `U`, the type parameter of `twice`
