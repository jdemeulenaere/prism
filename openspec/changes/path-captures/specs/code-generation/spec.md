## MODIFIED Requirements

### Requirement: Small Owned Environments Are Held in the Function Value
When an owned environment's layout fits in one pointer-sized word and none of its captures needs a drop, the function value MUST hold the environment's bytes in its environment word, its descriptor's drop entry MUST be null, and building it MUST NOT allocate.

#### Scenario: A handler capturing one handle allocates nothing
- **WHEN** `struct Handle : Copy { val id: Int }`, `struct Button { val onClick: () -> Int }` and `fun button(h: Handle): Button = Button({ h.id * 2 })` are declared and `prism dump-clif main.prism` runs on a file holding them and `fun main() { val b = button(Handle(21)); println(b.onClick()) }`
- **THEN** no function of the file calls the runtime's allocation function, and `prism run --check-allocations main.prism` prints `42` and exits with status 0

#### Scenario: A handler copying a field through a borrow allocates nothing
- **WHEN** `struct Todo { val id: Int, val count: Int }`, `struct Button { val onClick: () -> Int }` and `fun button(t: &Todo): Button = Button({ t.id * 2 })` are declared and `prism dump-clif main.prism` runs on a file holding them and `fun main() { val t = Todo(21, 3); val b = button(t); println(b.onClick()) }`
- **THEN** no function of the file calls the runtime's allocation function, and `prism run --check-allocations main.prism` prints `42` and exits with status 0
