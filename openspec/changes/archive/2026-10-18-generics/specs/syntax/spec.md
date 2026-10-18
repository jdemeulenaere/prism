# Syntax Specification Delta

## ADDED Requirements

### Requirement: Type Parameter Lists
A function MUST declare its type parameters between `fun` and its name or receiver (`fun <T> id(x: T)`, `fun <T> Box<T>.get()`), and a struct or enum after its name. A list MUST hold one or more names, optionally with a trailing comma. A function type parameter MAY have a bound, `: Copy` or `: Clone`. Type parameters after a function's name, an empty list, any other bound, and a bound in a struct or enum MUST be rejected with `E2001`.

#### Scenario: Type parameter lists
- **WHEN** parsing `fun <T> id(x: T): T = x`, `public compose fun <T : Copy, U : Clone,> show(a: T, b: U) {}`, `mut fun <T> Box<T>.put(v: T) {}`, `struct Pair<A, B>(val a: A, val b: B)` and `enum Option<T> : Copy { Some(T), None }`
- **THEN** each declaration parses with its type parameters in order, `T` bounded by `Copy` and `U` by `Clone` in `show`

#### Scenario: Type parameters after the function name rejected
- **WHEN** parsing `fun id<T>(x: T): T = x` or `fun User.f<T>(x: T) {}`
- **THEN** the parser reports `E2001`, saying that type parameters go before the name, as in `fun <T> id(...)`

#### Scenario: Malformed type parameter lists rejected
- **WHEN** parsing `fun <> f() {}`, `fun <T : Eq> f(x: T) {}`, `fun <T : Copy, Clone> f(x: T) {}` or `struct Box<T : Copy>(val v: T)`
- **THEN** the parser reports `E2001`: a list needs a type parameter, only `Copy` or `Clone` can bound one, and struct and enum type parameters cannot be bounded yet

### Requirement: Explicit Type Arguments at Call Sites
After an identifier or a member name in an expression, `<` MUST start a type-argument list of the call when the tokens up to the matching `>` parse as types, the list closes with `>`, and the next token is a `(` or `{` on the same line. Otherwise `<` MUST be the comparison operator. In `g(a < b, c > (d))`, the type-argument reading MUST win.

#### Scenario: Type arguments on calls
- **WHEN** parsing `id<Int>(1)`, `Box<String?>(null)`, `remember<Int> { 0 }`, `s.map<Box<Int>> { n -> Box(n) }` and `u?.pick<Int>(1)`
- **THEN** each parses as a call with the written type arguments, and `Box<Box<Int>>` closes with two `>` tokens

#### Scenario: Comparisons stay comparisons
- **WHEN** parsing `a < b`, `if (i<n) f()`, `val m = a < b + 1` and `val k = a < b >= c`
- **THEN** the first three are comparisons, and the last is rejected with `E2001` as a chained comparison, because a list closed by the `>` of `>=` is not a type-argument list

#### Scenario: Comma-separated comparisons read as type arguments
- **WHEN** parsing `g(a < b, c > (d))`
- **THEN** the call `g` has one argument, the call `a<b, c>(d)`, while `g((a < b), c > (d))` has two comparison arguments

## MODIFIED Requirements

### Requirement: Pratt Expression Precedence and Associativity
The parser MUST parse expressions according to Pratt operator precedence, correctly ordering assignment, Elvis, logical, comparison, range, arithmetic, prefix, postfix, and function call operators. Comparison operators (`<`, `<=`, `>`, `>=`) MUST be non-associative: an operand of a comparison MUST NOT itself be an unparenthesized comparison.

#### Scenario: Arithmetic and comparison precedence
- **WHEN** parsing expression `a + b * c > d`
- **THEN** operator precedence structures AST as `((a + (b * c)) > d)`

#### Scenario: Non-associative assignment
- **WHEN** code attempts chained assignment (`a = b = c`)
- **THEN** the parser rejects the expression with a compile error requiring separate statements

#### Scenario: Non-associative comparison
- **WHEN** parsing `a < b < c` or `a < b > c`
- **THEN** the parser reports `E2001` at the second operator, suggesting `&&` between two comparisons, while `(a < b) == c` and `a < b == c > d` parse

#### Scenario: Invalid assignment target rejected
- **WHEN** the left side of `=` is not a variable, a `.` field access, an index expression or a write through `*` (`a + b = c`, `1 = 2`, `g() = 1`, `a?.b = c`)
- **THEN** the parser rejects the assignment with a syntax error at the target

#### Scenario: Elvis operator binds tighter than comparison
- **WHEN** parsing `val eligible = age ?: 0 >= 18`
- **THEN** the Elvis operator binds tighter than comparison: `((age ?: 0) >= 18)`

#### Scenario: Elvis operator binds looser than arithmetic
- **WHEN** parsing `val total = base ?: offset + 10`
- **THEN** arithmetic addition binds tighter than the Elvis fallback: `(base ?: (offset + 10))`

#### Scenario: Newline suppresses call precedence on leading parenthesis
- **WHEN** an expression starting with `(` is on a new line following a call (`cleanup()\n(1 + 2).toString()`)
- **THEN** the parser treats `(1 + 2)` as a separate statement rather than a call argument to `cleanup()`

### Requirement: Extension Function Declarations
A function declaration MAY name a receiver type before its name, `fun T.name(...)`, which declares an extension function on `T`. The receiver MUST be a single type name, optionally with type arguments (`fun <T> Box<T>.get()`, `fun Box<Int>.sum()`). A receiver that is one of the function's own type parameters, a `Heap<...>` receiver, a `?`, a `&` or `&mut`, or a parenthesized or function type MUST be rejected with `E2001`, with a message saying what receivers may be.

#### Scenario: Extension declarations
- **WHEN** parsing `fun User.greet(): String = "Hi " + name`, `public fun Int.squared(): Int = this * this` and `public compose fun User.Avatar(): Unit { }`
- **THEN** each parses as a function named `greet`, `squared` or `Avatar` with the receiver type `User`, `Int` or `User` and the shared receiver mode

#### Scenario: Receivers with type arguments
- **WHEN** parsing `fun <T> Box<T>.get(): &T = &v` and `fun Box<Int>.sum(): Int = v`
- **THEN** each parses as an extension whose receiver type is `Box<T>` or `Box<Int>`

#### Scenario: Receiver with type arguments rejected
- **WHEN** parsing `fun Heap<User>.size(): Int = 0`
- **THEN** the parser reports `E2001`, saying that calls look through `Heap`, so the extension is declared on its contents (`fun User.size()`)

#### Scenario: Type-parameter receiver rejected
- **WHEN** parsing `fun <T> T.also(): T = this`
- **THEN** the parser reports `E2001`, saying that a bare type parameter cannot be a receiver

#### Scenario: Nullable or borrowed receiver rejected
- **WHEN** parsing `fun User?.greet(): String = "x"`, `fun &User.greet(): String = "x"` or `fun &mut User.rename() { }`
- **THEN** the parser reports `E2001`; for `User?` the message says to declare the extension on `User` and call it with `?.`, and for `&` it says that the receiver mode (`fun`, `mut fun`, `consuming fun`) decides how the receiver is borrowed

#### Scenario: Qualified receiver rejected
- **WHEN** parsing `fun models.User.greet(): String = "x"`
- **THEN** the parser reports `E2001`, since type names are not qualified
