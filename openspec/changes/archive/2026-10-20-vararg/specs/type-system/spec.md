## MODIFIED Requirements

### Requirement: Trailing Lambdas Bind to the Last Parameter
A trailing lambda MUST be bound to the callee's last parameter, for every callee: declared, extension (the receiver is not a parameter), generic (inferring type arguments as usual), function-typed value, struct constructor (last field) or variant (last payload), unless that parameter is a vararg. The arguments inside the parentheses MUST bind as they would without it, and a parameter they leave unset other than the last MUST take its default or be reported missing (`E4004`).

#### Scenario: Trailing lambda binds to the last parameter
- **WHEN** running `fun Button(label: String = "", onClick: () -> Unit = {}, content: () -> Unit) { print(label); onClick(); content() }` called as `Button(onClick = { println("hi") }) { println("content") }`
- **THEN** `label` takes its default, `onClick` the named closure and `content` the trailing lambda, so the program prints `hi` then `content`

#### Scenario: Trailing lambda binds to the last parameter of every callee
- **WHEN** a trailing lambda is passed to a generic function (`pick(3) { it + 1 }` for `fun <T: Copy> pick(value: T, choose: (T) -> T): T`), an extension (`2.twice { println(it) }` for `fun Int.twice(action: (Int) -> Unit)`), a function-typed value (`f(1) { println("body") }` for `f: (Int, () -> Unit) -> Unit`), a struct constructor (`Handler("h") { println("run") }` for `struct Handler(name: String, run: () -> Unit)`), a variant (`Event.Click(5) { println(it) }` for `enum Event { Click(Int, (Int) -> Unit) }`) or a function whose vararg comes before its last parameter (`on(1, 2) { println("run") }` for `fun on(vararg keys: Int, action: () -> Unit)`)
- **THEN** it binds to the last parameter, field or payload, and the program runs with the lambda in that position
