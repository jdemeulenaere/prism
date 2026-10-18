# Formatting Specification Delta

## Purpose
Defines `prism fmt`, the formatter that rewrites Prism source into one canonical layout: the command and its check mode, the layout rules, how comments, blank lines and written forms are kept, and the guarantees that formatting never changes meaning and is idempotent.

## ADDED Requirements

### Requirement: Format Command
`prism fmt <path>...` MUST rewrite each named file in place with the canonical layout. A file that is already formatted MUST NOT be rewritten. The command MUST print the path of each file it rewrote, one per line, and exit with status 0 when every file was formatted. Formatting MUST depend only on the file's own text: it resolves no imports, reads no configuration file, and has no style options.

#### Scenario: Formatting a file in place
- **WHEN** running `prism fmt main.prism` on a file containing `fun main( ):Int{return 1+2}`
- **THEN** the file then contains `fun main(): Int {\n    return 1 + 2\n}\n`, the command prints `main.prism`, and it exits with status 0

#### Scenario: Formatted file left alone
- **WHEN** running `prism fmt main.prism` on a file that is already formatted
- **THEN** the file is not written, nothing is printed, and the command exits with status 0

### Requirement: Formatting Directories
A directory named on the command line MUST be walked recursively, in sorted order, for files ending in `.prism`, skipping directories whose name starts with `.` and not following symbolic links to directories. A file named explicitly MUST be formatted whatever its extension.

#### Scenario: Formatting a directory
- **WHEN** running `prism fmt src` where `src/a.prism` and `src/b/c.prism` are unformatted, `src/d.prism` is formatted and `src/.hidden/e.prism` is unformatted
- **THEN** `a.prism` and `c.prism` are rewritten and listed, `d.prism` is neither rewritten nor listed, and `.hidden/e.prism` is untouched

### Requirement: Formatting Standard Input
The path `-` MUST read source from standard input and write the formatted source to standard output.

#### Scenario: Formatting standard input
- **WHEN** running `prism fmt -` with `val x=1` on standard input
- **THEN** it writes `val x = 1\n` to standard output and exits with status 0

### Requirement: Check Mode
`prism fmt --check <path>...` MUST write no file. It MUST print the path of each file whose formatting would change it, one per line, and exit with a non-zero status if there is at least one, and with status 0 otherwise. With `-`, it MUST write nothing to standard output and exit non-zero if the input is not formatted.

#### Scenario: Check mode with an unformatted file
- **WHEN** running `prism fmt --check src` where `src/a.prism` is unformatted and `src/b.prism` is formatted
- **THEN** it prints the path of `a.prism` only, both files are unchanged, and it exits with a non-zero status

#### Scenario: Check mode with formatted files
- **WHEN** running `prism fmt --check src` where every file under `src` is formatted
- **THEN** it prints nothing and exits with status 0

### Requirement: Files With Syntax Errors Are Not Formatted
A file that does not parse MUST NOT be formatted or rewritten. `prism fmt` MUST report its syntax error with the same diagnostic as `prism check` (`E2001` or `E2002`), continue with the remaining files, and exit with a non-zero status. In check mode the file is reported the same way and is not listed as a file that would change.

#### Scenario: Syntax error refused
- **WHEN** running `prism fmt src` where `src/bad.prism` contains `fun f( {` and `src/good.prism` is unformatted
- **THEN** it reports `error[E2001]` at `bad.prism`, `bad.prism` is byte-for-byte unchanged, `good.prism` is formatted, and it exits with a non-zero status

#### Scenario: Syntax error on standard input
- **WHEN** running `prism fmt -` with `val = 1` on standard input
- **THEN** it reports `error[E2001]`, writes nothing to standard output, and exits with a non-zero status

### Requirement: Formatting Preserves Meaning
The formatted source MUST parse to the same syntax tree as the input, ignoring source positions, and MUST contain the same comments, with the same text and in the same order. The formatter MUST verify both on every run by parsing its own output; on any difference it MUST leave the file untouched, report an internal error (a formatter bug, which carries no catalog code), and exit with a non-zero status.

#### Scenario: Every fixture and example keeps its meaning
- **WHEN** formatting every `.prism` file under `crates/prism-cli/tests/fixtures/` and `examples/` that parses
- **THEN** each output parses to the input's syntax tree, ignoring positions, and holds the input's comments in order

#### Scenario: Line breaks are only added where they keep the parse
- **WHEN** formatting a statement whose expression is too long for one line, inside a block where a line break ends an expression
- **THEN** the formatter breaks only inside parentheses or brackets, after a binary operator, `=` or `->`, or before `.`, `?.` or `?:`, so the statement parses as before

### Requirement: Formatting Is Idempotent
Formatting the output of the formatter MUST return it unchanged.

#### Scenario: Formatting formatted code
- **WHEN** formatting the formatted form of every `.prism` file under `crates/prism-cli/tests/fixtures/` and `examples/` that parses
- **THEN** the result is identical to its input

### Requirement: Indentation and Line Width
The formatter MUST indent with four spaces per level, never with tabs, and MUST keep lines within 100 characters wherever a break that keeps the parse exists; a line that cannot be broken (a long string literal or name) MAY exceed it. Every part of the layout that fits on the current line MUST stay on it.

#### Scenario: Short call stays on one line
- **WHEN** formatting `foo(\n  a,\n  b,\n)` as a statement
- **THEN** the result is `foo(a, b)`, without the trailing comma

### Requirement: Breaking Lists
A parameter, argument, struct shorthand field or enum payload list that does not fit MUST be broken with one item per line, indented one level, each followed by a comma, with the closing parenthesis on its own line. A list that fits MUST have no trailing comma. Type-parameter lists, type-argument lists and types are never broken.

#### Scenario: Long parameter list breaks one per line
- **WHEN** formatting `fun f(alpha: Int, beta: String, gamma: Double, delta: Bool, epsilon: Char, zeta: Int, eta: Int): Int = 0`
- **THEN** each parameter is on its own line indented by four spaces and followed by a comma, and the line `): Int = 0` closes the list

### Requirement: Breaking Chains and Operators
A chain of two or more calls that does not fit MUST break before each `.` or `?.` that starts a call, indented one level, keeping its receiver on the first line. A binary expression that does not fit MUST break after its operator, except `?:`, which starts the continuation line; continuation lines are indented one level.

#### Scenario: Long call chain breaks before each call
- **WHEN** formatting `val result = items.filter { x -> x > threshold }.map { x -> x * factor }.fold(0) { a, b -> a + b }` inside a function
- **THEN** `val result = items` is the first line and each of `.filter`, `.map` and `.fold` starts its own continuation line, indented one level deeper than the statement

#### Scenario: Long binary expression breaks after the operator
- **WHEN** formatting a `val` whose initializer is a sum `a + b + c` of names too long for one line, even after moving it below the `=`
- **THEN** each line of the sum but the last ends with `+`, and the continuation lines are indented one level deeper than the first operand

#### Scenario: Elvis continuation starts the line
- **WHEN** formatting a `?:` expression too long for one line
- **THEN** the continuation line starts with `?:`

### Requirement: Breaking After Equals and Arrows
The right side of `=` (in a declaration, an assignment or an expression body) and the body after a `when` branch's `->` that does not fit MUST move to the next line, indented one level, unless it is a call, a closure, a block, a `when` or an `if` with a braced branch, which stays on the line and breaks inside itself.

#### Scenario: Long initializer moves below the equals sign
- **WHEN** formatting a `val` whose initializer is a field chain that does not fit after `=` but fits on the next line
- **THEN** the line ends with `=` and the initializer is on the next line, indented one level

#### Scenario: Call initializer breaks inside its arguments
- **WHEN** formatting a `val` whose initializer is a call with arguments that do not fit on the line
- **THEN** the call starts after `= ` and its arguments are broken one per line

### Requirement: One Item Per Line
The formatter MUST print one top-level declaration, struct field, enum variant, statement or `when` branch per line, dropping the `;` and `,` separators between them, except in a struct or enum body kept on one line. A `when` MUST have one branch per line.

#### Scenario: Statements and separators
- **WHEN** formatting `fun f() { val a = 1; var b = a ;b += 2; }`
- **THEN** the result is `fun f() {\n    val a = 1\n    var b = a\n    b += 2\n}\n`

#### Scenario: When branches one per line
- **WHEN** formatting `val s = when (n) { 0 -> "zero"; in 1..9 -> "small", else -> { "large" } }` inside a function
- **THEN** each branch is on its own line indented one level, the separators are dropped, and the `else` branch keeps its braced block, broken over lines

### Requirement: Blocks and Closures
Blocks (function bodies, braced branches, loop bodies and statement-position scopes) MUST be broken over lines, an empty block being `{}`. A closure whose body is a single statement or expression, written on one line, MUST stay on one line when it fits (`{ x -> x + 1 }`); any other closure is broken, its parameters staying after the `{`. An empty argument list before a trailing lambda MUST be dropped.

#### Scenario: Closures and trailing lambdas
- **WHEN** formatting `val f = { x -> x+1 }`, `run() { compute() }` and `items.forEach { x -> total += x; count += 1 }` inside a function
- **THEN** the results are `val f = { x -> x + 1 }`, `run { compute() }` and `items.forEach { x ->` followed by the two statements on their own lines and `}`

#### Scenario: Closure written over lines stays broken
- **WHEN** formatting `Column(padding = 16) {\n    Text("Hello")\n}` and `button(onClick = { count += 1 })` inside a function
- **THEN** the first keeps its closure broken over three lines and the second stays on one line

#### Scenario: Loops, references and write-through
- **WHEN** formatting `for(i in 0..<n){*r+=i}`, `do{n-=1}while(n>0)` and `val r=&mut x` inside a function
- **THEN** the results are `for (i in 0..<n) {\n    *r += i\n}`, `do {\n    n -= 1\n} while (n > 0)` and `val r = &mut x`

### Requirement: Struct and Enum Bodies
A struct or enum body written on one line, without comments, MUST stay on one line when it fits, with `, ` between its members (`enum Color { Red, Green, Blue }`); otherwise each member is on its own line.

#### Scenario: One-line enum kept
- **WHEN** formatting `enum Color{Red;Green,Blue}` and `enum Shape {\nCircle(Double), Square(Double)}`
- **THEN** the results are `enum Color { Red, Green, Blue }` and `enum Shape {\n    Circle(Double)\n    Square(Double)\n}`

### Requirement: Spacing
Spacing MUST follow the Kotlin conventions: one space around binary operators, `=`, `->` and compound assignment operators; none after a prefix operator except `&mut `; one space after each comma and none before; `name: Type` for parameters, fields, bindings and return types; `<T : Copy>` for bounds and `struct Point : Copy` for markers; one space before a `{` that follows a header or a call. Modifiers MUST be in grammar order: visibility, `compose`, then `mut` or `consuming`.

#### Scenario: Declarations with generics, receivers and markers
- **WHEN** formatting `public mut fun<T:Copy>Box<T>.set(value:T):Unit{this.value=value}` and `struct Box<T>:Copy{var value:T}`
- **THEN** the results are `public mut fun <T : Copy> Box<T>.set(value: T): Unit {\n    this.value = value\n}` and `struct Box<T> : Copy { var value: T }`, the struct body staying on the one line it was written on

#### Scenario: Types keep their meaning
- **WHEN** formatting the parameter types `(() -> Unit)?`, `(&Int)?`, `&Int?`, `compose () -> Unit` and `(Int,String)->Bool`
- **THEN** they are printed `(() -> Unit)?`, `(&Int)?`, `&Int?`, `compose () -> Unit` and `(Int, String) -> Bool`, keeping the parentheses that their meaning needs

### Requirement: Blank Lines Between Items
Where the input has one or more blank lines between two declarations, members, statements or `when` branches, or between a comment and what surrounds it, the formatter MUST keep exactly one. It MUST remove every other blank line: none after an opening brace, before a closing brace, or at the start of the file. Two consecutive top-level declarations MUST be separated by one blank line, unless both are top-level `val`s.

#### Scenario: Blank lines collapsed
- **WHEN** formatting `fun f() {\n\n    val a = 1\n\n\n\n    val b = 2\n\n}`
- **THEN** the result is `fun f() {\n    val a = 1\n\n    val b = 2\n}\n`

#### Scenario: Declarations separated
- **WHEN** formatting `val a = 1\nval b = 2\nfun f() {}\nstruct S {}`
- **THEN** the result is `val a = 1\nval b = 2\n\nfun f() {}\n\nstruct S {}\n`

### Requirement: File Structure
The formatter MUST put exactly one blank line after the package declaration and after the last import when something follows them, and print import paths with `/` between segments. The output MUST end with exactly one newline, and an input without declarations or comments MUST format to the empty file.

#### Scenario: Package and imports separated
- **WHEN** formatting `package app\nimport /core/fmt\nimport util\nfun main() {}`
- **THEN** the result is `package app\n\nimport /core/fmt\nimport util\n\nfun main() {}\n`

### Requirement: Comment Placement
The formatter MUST keep every line and block comment. A comment on its own line MUST stay on its own line, before the item (declaration, member, statement, branch, argument or parameter) that follows it, at that item's indentation. A comment after an item on the item's last line MUST stay at the end of that line, after one space. A comment before a closing brace or parenthesis MUST stay on its own line before it.

#### Scenario: Leading, trailing and dangling comments
- **WHEN** formatting `fun f() {\n// first\nval a = 1   // one\n    /* two */ val b = 2\n  // last\n}`
- **THEN** the result is `fun f() {\n    // first\n    val a = 1 // one\n    /* two */ val b = 2\n    // last\n}\n`

#### Scenario: Comment before an opening brace
- **WHEN** formatting `fun f(): Int // result\n{\n    return 1\n}` and, inside a function, `if (a) {\n    b()\n} // otherwise\nelse {\n    c()\n}`
- **THEN** each `{` stays on its header's line after one space, and the comment moves to its own line at the start of the block: `fun f(): Int {\n    // result\n    return 1\n}` and `} else {\n        // otherwise\n        c()\n    }`

#### Scenario: Line comment inside an expression or type
- **WHEN** formatting `val w: // width\nInt = 3` and `x[// first\n1] = 2` inside a function
- **THEN** each comment stays at the end of its line after one space (`val w: // width`, `x[ // first`), and the code after it continues on the next line, indented one level deeper than the line the comment ends

#### Scenario: Comments inside lists and expressions
- **WHEN** formatting a call whose arguments are on separate lines, each followed by a line comment, and an expression with a block comment between an operator and its operand
- **THEN** each line comment stays at the end of its argument's line, which keeps the argument list broken one per line, and the block comment stays between the operator and the operand

### Requirement: Comment Text
Comment text MUST be kept verbatim, except that trailing whitespace is removed from line comments. A line comment always ends its output line, and a block comment followed by more code on its line stays on that line.

#### Scenario: Comment text kept
- **WHEN** formatting `val a = 1 /* x  y */ // note   ` followed by a newline
- **THEN** the result is `val a = 1 /* x  y */ // note\n`

### Requirement: Written Forms Are Preserved
Where the syntax tree does not record how something was written, the formatter MUST keep the written form: the spelling of every literal (`0xFF`, `0b1010`, `1_000`, `1e3`, `'\n'`, `"\u{1F680}"`), parentheses around an expression (nested pairs collapse to one), an explicit `private`, the shorthand or block form of a struct, whether a shorthand field wrote `val`, and whether each branch or loop body is braced.

#### Scenario: Literals and parentheses kept
- **WHEN** formatting `val mask = (0xFF_FF)` and `val n = ((a + b)) * c`
- **THEN** the results are `val mask = (0xFF_FF)` and `val n = (a + b) * c`

#### Scenario: Struct forms and private kept
- **WHEN** formatting `private struct Id(id:Int)` and `struct P{\nval x:Int}`
- **THEN** the results are `private struct Id(id: Int)`, keeping `private`, the shorthand form and the field without `val`, and `struct P {\n    val x: Int\n}`, keeping the block form
