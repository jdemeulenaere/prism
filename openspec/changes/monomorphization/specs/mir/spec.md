# MIR Specification Delta

## MODIFIED Requirements

### Requirement: Conditional Operators Become Branches
`if`, `&&`, `||` and `?:` MUST become branches that evaluate only the operands and branches their values need. `?:` MUST read the tag of its nullable operand and, when it is present, move or copy its payload out. A `?:` whose left operand is not nullable MUST evaluate only that operand, as its value, unless the operand's type is a type parameter: such an operand MUST be wrapped into a `T?` temporary, whose tag is then read as for a nullable operand.

#### Scenario: Logical and evaluates its right operand only when needed
- **WHEN** `fun g(): Bool = true` is declared and `fun f(a: Bool): Bool = a && g()` is lowered
- **THEN** the body branches on `a`, calls `g` only on the edge where `a` is true, and gives `false` on the other edge

#### Scenario: Elvis branches on the tag
- **WHEN** `fun f(n: String?): String = n ?: "anon"` is lowered
- **THEN** `n` is moved into a temporary whose discriminant is read; where it is present its payload is moved into the return place, and where it is `null` the string `"anon"` is

#### Scenario: Elvis on a non-nullable left side is the left side
- **WHEN** `fun g(): Int = 2` is declared and `fun f(a: Int): Int = a ?: g()` and `fun h(s: String): String = s ?: "anon"` are lowered
- **THEN** `a` is copied and `s` moved into the return place, no discriminant is read, and neither `g` nor the string `"anon"` is evaluated

#### Scenario: Elvis on a type parameter wraps its left side
- **WHEN** `fun <T> orElse(a: T, c: T): T = a ?: c` is lowered
- **THEN** `a` is wrapped into a temporary of type `T?` whose discriminant is read; where it is present its payload is moved into the return place, and where it is `null` `c` is

## ADDED Requirements

### Requirement: Instances Are Substituted Bodies
An instance of a generic body MUST be its elaborated MIR with the instance's type arguments substituted for the body's type parameters in every local, aggregate, constant and call type argument, while each call keeps its callee's declared parameter types. The instance MUST record its type arguments and no type parameter. A drop of a value whose substituted type no longer needs a drop MUST become a jump to its target. The instance MUST be well-formed elaborated MIR.

#### Scenario: Instances of a generic function
- **WHEN** `fun <T> discard(x: T) {}` is declared and `fun main() { discard(1); discard("a") }` runs through `prism dump-mir --instances main.prism`
- **THEN** it prints the bodies `main::discard<Int>`, whose argument `_1` is an `Int` and which drops nothing, and `main::discard<String>`, whose argument is a `String` that it drops when it returns

#### Scenario: A call inside an instance names its instance
- **WHEN** `fun <T> id(x: T): T = x` and `fun <T> twice(x: T): T = id(id(x))` are declared and `fun main() { println(twice(1)) }` runs through `prism dump-mir --instances main.prism`
- **THEN** the body `main::twice<Int>` calls `main::id<Int>` twice, each call keeping the declared parameter type `T`, and `main::id<Int>` is printed too

### Requirement: Nullable Arguments Flatten Instances
In an instance where a type parameter `T` has a nullable argument, `T?` is that argument (the `generics` spec), so wrapping a value of type `T` into a `T?` MUST be the value itself, and the payload of a place of type `T?` MUST be the place itself. A tag read from a `T?` MUST read the argument's own tag.

#### Scenario: Wrapping and unwrapping a nullable argument
- **WHEN** `fun <T> wrap(a: T): T? = a` and `fun <T> first(a: T?, c: T): T = a ?: c` are declared and `fun main() { println(wrap<Int?>(null) ?: 0); println(first<Int?>(null, 5) ?: 0) }` runs through `prism dump-mir --instances main.prism`
- **THEN** `main::wrap<Int?>` moves `_1` into the return place without a wrap, and `main::first<Int?>` reads the discriminant of its `Int?` temporary and, where it is present, moves the temporary itself into the return place, with no payload projection

### Requirement: Instance MIR Dump
`prism dump-mir --instances <file.prism>` MUST print, as deterministic text, the elaborated MIR of every instance that the program whose entry is the file needs, in order of their symbols, each headed by the generic's path followed by its type arguments. When any file of the program has errors, it MUST report them, print no MIR and exit with a failure status.

#### Scenario: Dumping the instances of a program
- **WHEN** `lib/box.prism` declares `public fun <T> keep(x: T): T = x`, and `app/main.prism` imports `/lib/box` and declares `fun main() { println(keep(1)); println(keep(true)) }`
- **THEN** `prism dump-mir --instances app/main.prism` prints the bodies `fun lib/box::keep<Bool>(_1: Bool): Bool` and then `fun lib/box::keep<Int>(_1: Int): Int`, the same text on every run, and exits with status 0
