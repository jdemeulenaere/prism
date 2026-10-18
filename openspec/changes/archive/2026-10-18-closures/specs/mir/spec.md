# MIR Specification Delta

## ADDED Requirements

### Requirement: Function Names and Variant Constructors as Values Are Adapter Closures
A top-level function or a variant constructor read as a value MUST be lowered as a closure without captures, the next closure of the enclosing body. Its arguments MUST have the parameter types of the value's function type, and its body MUST call the function, or build the variant, with each argument converted to the declared parameter type, returning the result converted to the value's result type. MIR MUST hold no other function value.

#### Scenario: A function passed as a value
- **WHEN** `fun double(n: Int): Int = n * 2` and `fun apply(f: (Int) -> Int): Int = f(1)` are declared and `fun main() { println(apply(double)) }` is lowered
- **THEN** the body of `main` builds the closure value `main::main#closure0` with no captures and passes it to `apply`, and the body `main::main#closure0` takes an `Int` argument and returns the result of calling `main::double` with it

#### Scenario: An adapter converts its arguments
- **WHEN** `fun show(n: Int?) {}` and `fun each(f: (Int) -> Unit) { f(1) }` are declared and `fun main() { each(show) }` is lowered
- **THEN** the adapter body takes an `Int` argument, wraps it as an `Int?` and calls `main::show` with the wrapped value

#### Scenario: A variant constructor passed as a value
- **WHEN** `enum Shape { Circle(Int), Dot }` and `fun build(f: (Int) -> Shape): Shape = f(3)` are declared and `fun main() { val s = build(Shape.Circle) }` is lowered
- **THEN** the adapter body takes an `Int` argument and assigns the return place the variant `Circle` of `main::Shape` built from it

### Requirement: Instances Carry Their Closures
An instance of a generic body MUST include every closure lowered inside that body, nested ones included, each instantiated with the instance's type arguments and built by the instance from those instantiated bodies.

#### Scenario: A closure inside a generic body
- **WHEN** `fun run(g: () -> Int): Int = g()` and `fun <T : Copy> count(x: T, f: (T) -> Int): Int = run { f(x) + 1 }` are declared and `fun main() { println(count(2) { n -> n * 10 }) }` runs through `prism dump-mir --instances main.prism`
- **THEN** it prints the body `main::count<Int>`, which builds the closure value `main::count<Int>#closure0` from its arguments `f` and `x`, and the body `main::count<Int>#closure0`, which has the capture locals `f` of type `(Int) -> Int` and `x` of type `Int`
