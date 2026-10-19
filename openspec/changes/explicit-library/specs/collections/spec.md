## MODIFIED Requirements

### Requirement: Higher-Order Helpers
`/prism/collections` MUST declare public extensions on `List<T>` written in Prism over the members, which a file calls after importing that module, like any imported extension (`standard-library`, "Library Names Are Imported"). Their function arguments MUST NOT be composable contexts and MUST NOT escape. Each MUST borrow its list, except `filter`, which consumes it.

#### Scenario: Helpers over a list of structs
- **WHEN** `struct Todo { val title: String, var done: Bool }` is declared and `fun f(todos: &List<Todo>): String = todos.joinToString { t -> t.title.clone() }` and `fun g(todos: &List<Todo>): Int = todos.count { t -> t.done }` are checked
- **THEN** the file typechecks, and each helper borrows the list
