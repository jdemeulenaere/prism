# References Specification Delta

## ADDED Requirements

### Requirement: Compound Assignments Read and Write Their Target
The borrow checker MUST treat a compound assignment as a read of its target followed by a write of it: a live mutable borrow of an overlapping place MUST be reported once, with `E5003`, and a live shared one with `E5004`. A closure whose body compound-assigns a captured variable or one of its fields MUST capture it mutably. A target that may have been moved MUST be rejected with `E5001`.

#### Scenario: Compound assignment while borrowed
- **WHEN** `fun print(n: &Int) {}` is declared and a body runs `var x = 1; val r = &x; x += 1; print(r)`, or `var y = 1; val m = &mut y; y += 1; print(m)`, or `var z = 1; val k = &z; print(k); z += 1`
- **THEN** the borrow checker emits `E5004` at `x` in `x += 1`, a single `E5003` at `y` in `y += 1`, and accepts the last, whose borrow ends at `print(k)`

#### Scenario: A closure compound-assigning a capture borrows it mutably
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var count = 0; val f = { count += 1; count }; print(count); run(f)`, or `var total = 0; run { total += 1; total }; print(total)`
- **THEN** the borrow checker emits `E5003` at `count` in `print(count)`, and accepts the second, whose closure's borrow ends when `run` returns

#### Scenario: Compound assignment to a moved string
- **WHEN** `fun take(s: String) {}` is declared and `fun f() { var s = "a"; take(s); s += "b" }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `s += "b"`, with a related location at the `s` passed to `take`
