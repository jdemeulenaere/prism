# Execution Specification Delta

## ADDED Requirements

### Requirement: String Values
A string literal MUST evaluate to a string holding its decoded text. `a + b` MUST give a new string holding the text of `a` followed by that of `b`, leaving both operands unchanged, and `s += t` MUST give `s` the value of `s + t`. `a == b` MUST be `true` exactly when both strings hold the same sequence of characters, and `a != b` its negation.

#### Scenario: Concatenation and equality
- **WHEN** `fun main() { val a = "pri"; val b = a + "sm"; println(b); println(a); println(b == "prism"); println(a != "pri") }` runs
- **THEN** it prints `prism`, `pri`, `true` and `false`

#### Scenario: Compound assignment of strings
- **WHEN** `fun main() { var s = "a"; val t = "b"; s += t; s += t; println(s); println(t) }` runs
- **THEN** it prints `abb` and then `b`

#### Scenario: When over strings
- **WHEN** `fun f(s: String): Int = when (s) { "one" -> 1; "two" -> 2; else -> 0 }` is declared and a program prints `f("two")`, `f("one")` and `f("three")`
- **THEN** it prints `2`, `1` and `0`

#### Scenario: Strings in structs and top-level values
- **WHEN** `lib/names.prism` declares `public struct User { val name: String, val nick: String? }` and `public val ADMIN: String = "ro" + "ot"`, and `app/main.prism` imports `/lib/names` and declares `fun main() { val u = User(ADMIN.clone(), null); println(u.name); println(u.nick == null); println(ADMIN) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `root`, `true` and `root`, writes nothing to stderr and exits with status 0

### Requirement: String Length and Characters
`s.length` MUST be the number of characters (Unicode scalar values) of `s`, and `s[i]` the character at index `i`, counting characters from 0. An index outside `0..<s.length` MUST raise the runtime error "string index <i> out of range for length <n>" at the indexing expression.

#### Scenario: Characters beyond ASCII
- **WHEN** `fun main() { val s = "héllo"; println(s.length); println(s[1]); println(s[4]) }` runs
- **THEN** it prints `5`, `é` and `o`

#### Scenario: An index out of range
- **WHEN** `fun main() { val s = "abc"; println(s[0]); println(s[3]) }` runs
- **THEN** stdout holds `a` and a newline, stderr holds `panic: string index 3 out of range for length 3` and the location of `s[3]`, and the exit status is 101

### Requirement: Scalars Convert to Text
`x.toString()` of an `Int`, `Double`, `Bool` or `Char` MUST give a new string holding the text that `print(x)` writes.

#### Scenario: Converting scalars
- **WHEN** `fun main() { val n = -42; println(n.toString() + "!"); println(0.1.toString()); println(1.0E7.toString()); println(true.toString()); println('é'.toString().length) }` runs
- **THEN** it prints `-42!`, `0.1`, `1.0E7`, `true` and `1`

### Requirement: Values Are Freed Where Their Owner's Scope Ends
Each drop that the elaborated MIR runs MUST free, at that point, the memory the dropped value owns, deterministically and without a garbage collector or reference counting, so that every allocation a value owns is freed exactly once: where its owner's scope ends, where it is overwritten by an assignment or a write through a reference, or by the code it was moved to. A top-level value MUST never be dropped.

#### Scenario: Deterministic scope exit cleanup
- **WHEN** `fun main() { var total = 0; for (i in 0..<1000) { val s = i.toString() + "x"; total += s.length }; println(total) }` runs with `--check-allocations`
- **THEN** it prints `3890`, writes nothing to stderr and exits with status 0

#### Scenario: A conditionally moved value is dropped only if present
- **WHEN** `fun take(s: String): Int = s.length` and `fun f(c: Bool) { val s = 1.toString(); if (c) println(take(s)) }` are declared and `fun main() { f(true); f(false) }` runs with `--check-allocations`
- **THEN** it prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Writing through a reference drops the old referent
- **WHEN** `fun set(r: &mut String, v: String) { *r = v }` is declared and `fun main() { var s = 1.toString(); set(&mut s, 2.toString()); s = 3.toString(); println(s) }` runs with `--check-allocations`
- **THEN** it prints `3`, writes nothing to stderr and exits with status 0

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `fun make(): String = 5.toString()` and `fun greet(n: &String): Int = n.length` are declared and `fun main() { println(greet(make())); println(greet(make())) }` runs with `--check-allocations`
- **THEN** it prints `1` twice, writes nothing to stderr and exits with status 0

#### Scenario: Top-level values are kept
- **WHEN** `val NAME: String = 4.toString() + "2"` is declared and `fun main() { println(NAME); println(NAME.length) }` runs with `--check-allocations`
- **THEN** it prints `42` and then `2`, writes nothing to stderr and exits with status 0

### Requirement: Cloning Values That Own Storage
`x.clone()` of a value that owns storage MUST give a value equal to `x` that shares no owned memory with it, so that changing or dropping either leaves the other unchanged, and MUST run no user code.

#### Scenario: String clone is independent
- **WHEN** `fun main() { var a = 1.toString(); val b = a.clone(); a = 2.toString(); println(a); println(b) }` runs with `--check-allocations`
- **THEN** it prints `2` and then `1`, writes nothing to stderr and exits with status 0

#### Scenario: Struct clone clones each field
- **WHEN** `struct User : Clone { var name: String, val nick: String? }` is declared and `fun main() { var a = User(1.toString(), 2.toString()); val b = a.clone(); a.name = 3.toString(); println(a.name); println(b.name); println(b.nick == a.nick) }` runs with `--check-allocations`
- **THEN** it prints `3`, `1` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Allocation Checking
With the option `--check-allocations` before the file, `prism run` MUST track every allocation of the program; without it nothing MUST be tracked. When `main` returns, allocations still live that no top-level value owns MUST be reported, after the output is flushed, as `allocation check: allocations still live at exit: <n> (<b> bytes)` on stderr, and the program MUST exit with status 102. A program ending with a runtime error MUST NOT be checked.

#### Scenario: A program that frees what it allocates
- **WHEN** `prism run --check-allocations main.prism` runs on `fun main() { val s = "a" + "b"; println(s) }`
- **THEN** it prints `ab`, writes nothing to stderr and exits with status 0

#### Scenario: A leak is reported
- **WHEN** a program run with `--check-allocations` returns from `main` while one allocation of 5 bytes, owned by no top-level value, is still live
- **THEN** stderr holds `allocation check: allocations still live at exit: 1 (5 bytes)` and the exit status is 102

#### Scenario: A runtime error is not checked
- **WHEN** `prism run --check-allocations main.prism` runs on `fun zero(): Int = 0` and `fun main() { val s = "a" + "b"; println(1 / zero()) }`
- **THEN** stderr holds only the runtime error, and the exit status is 101

### Requirement: Invalid Frees Are Reported
Under `--check-allocations`, a free of memory that is not a live allocation, or that gives another size or alignment than its allocation, MUST flush the output, write `allocation check: freed memory that is not allocated` to stderr and end the program with status 102.

#### Scenario: A double free is reported
- **WHEN** a program run with `--check-allocations` frees the same allocation twice
- **THEN** stderr holds `allocation check: freed memory that is not allocated` and the exit status is 102

### Requirement: Running Out of Memory Is a Runtime Error
An allocation the system cannot satisfy, or a string whose length would not fit in memory, MUST raise the runtime error "out of memory", which ends the program as any runtime error does but reports no location.

#### Scenario: An allocation that fails
- **WHEN** the runtime cannot allocate the memory a program asks for
- **THEN** the output is flushed, stderr holds exactly `panic: out of memory` and a newline, and the exit status is 101

## MODIFIED Requirements

### Requirement: Output Built-Ins Print Their Argument
`print(x)` MUST write the text of `x` to stdout, `println(x)` the text of `x` followed by a newline, and `println()` a newline. An `Int` MUST be written in decimal with a leading `-` when negative, a `Bool` as `true` or `false`, a `Char` as its UTF-8 encoding and a `String` as its text in UTF-8.

#### Scenario: Printing scalars
- **WHEN** `fun main() { print(1); print(true); println('x'); println(-5); println() }` runs
- **THEN** stdout holds `1truex`, a newline, `-5`, a newline, and a newline

#### Scenario: Printing strings
- **WHEN** `fun main() { val s = "Grüße, \u{1F680}"; print(s); println("!"); println(s) }` runs
- **THEN** stdout holds `Grüße, 🚀!`, a newline, `Grüße, 🚀` and a newline

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory" MUST write no location line. No further code of the program MUST run, and no drop MUST run.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline
