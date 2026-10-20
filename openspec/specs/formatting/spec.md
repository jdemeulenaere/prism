# Formatting Specification

## Purpose
Defines `prism fmt`, the formatter that rewrites Prism source into one canonical layout: the command and its check mode, the layout rules, how comments, blank lines and written forms are kept, and the guarantees that formatting never changes meaning and is idempotent.

## Requirements

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
The formatted source MUST parse to the same syntax tree as the input, ignoring source positions and the order of imports ("Imports Are Sorted"), and MUST contain the same comments, with the same text and in the same order. The formatter MUST verify both on every run by parsing its own output; on any difference it MUST leave the file untouched, report an internal error (a formatter bug, which carries no catalog code), and exit with a non-zero status.

#### Scenario: Every fixture and example keeps its meaning
- **WHEN** formatting every `.prism` file under `crates/prism-cli/tests/fixtures/` and `examples/` that parses
- **THEN** each output parses to the input's syntax tree, ignoring positions and the order of imports, and holds the input's comments in order

#### Scenario: Line breaks are only added where they keep the parse
- **WHEN** formatting a statement whose expression is too long for one line, inside a block where a line break ends an expression
- **THEN** the formatter breaks only inside parentheses or brackets, after a binary operator, `=` or `->`, or before `.`, `?.` or `?:`, so the statement parses as before

### Requirement: Formatting Is Idempotent
Formatting the output of the formatter MUST return it unchanged.

#### Scenario: Formatting formatted code
- **WHEN** formatting the formatted form of every `.prism` file under `crates/prism-cli/tests/fixtures/` and `examples/` that parses
- **THEN** the result is identical to its input

### Requirement: Indentation and Line Width
The formatter MUST indent with four spaces per level, never with tabs, and MUST keep lines within 100 characters (Unicode scalar values, so a wide character counts as one) wherever the layout rules of this spec provide a break; a line they cannot break MAY exceed it, such as a long string literal or name, or a chain of field accesses, which breaks only before a call ("Breaking Chains and Operators"). Every part of the layout that fits on the current line MUST stay on it.

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
The formatter MUST put exactly one blank line after the package declaration and after the last import when something follows them, and print import paths with `/` between segments. The output MUST end with exactly one newline, and an input without declarations or comments MUST format to the empty file. Every line MUST end with `\n`: `\r\n` line endings, in code and in comments, are normalized. A leading byte order mark is dropped, as `rustfmt` does, so the output never starts with one.

#### Scenario: Package and imports separated
- **WHEN** formatting `package app\nimport /core/fmt\nimport util\nfun main() {}`
- **THEN** the result is `package app\n\nimport /core/fmt\nimport util\n\nfun main() {}\n`

#### Scenario: Line endings normalized
- **WHEN** running `prism fmt main.prism` on a file containing `val x = 1 // one\r\nval y = 2\r\n`, laid out canonically apart from its line endings
- **THEN** the file then contains `val x = 1 // one\nval y = 2\n` and the command prints `main.prism`

#### Scenario: Leading byte order mark dropped
- **WHEN** running `prism fmt main.prism` on a file that starts with a byte order mark (U+FEFF) followed by `val x = 1\n`
- **THEN** the file then contains `val x = 1\n` without the mark and the command prints `main.prism`

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

### Requirement: Patterns and Destructuring Are Printed
The formatter MUST print patterns as written, in the spacing of calls: constructor patterns like calls (`Shape.Rect(val w, _)`, `Point(y = 0)`), bindings as `val name` or `var name`, one space around `if` before a guard and around `as` in a destructuring entry, and destructuring declarations as `val (a, b as c) = value`. Pattern lists break like argument lists.

#### Scenario: Patterns, guards and destructuring
- **WHEN** formatting `fun f(s:Shape,u:User):Int{val(name as n,email)=u;return when(s){Shape.Rect(val w,_)if w>1->1;Point(y=0),_->2}}`
- **THEN** the result is `fun f(s: Shape, u: User): Int {\n    val (name as n, email) = u\n    return when (s) {\n        Shape.Rect(val w, _) if w > 1 -> 1\n        Point(y = 0), _ -> 2\n    }\n}\n`

### Requirement: Templates Are Printed as Written
The formatter MUST print the text runs and name parts of a string template exactly as written, escapes and unicode sequences included, and MUST keep each part's written form (`$x` or `${x}`). Each expression part MUST be printed by the usual rules for its expression, on one line: a template MUST NOT be broken across lines, whatever its width.

#### Scenario: Template text and parts kept
- **WHEN** formatting `val s = "Count: $count, sum ${a+b}, \$5 \u{1F680} ${ name }"`
- **THEN** the result is `val s = "Count: $count, sum ${a + b}, \$5 \u{1F680} ${name}"`

#### Scenario: A long template is not broken
- **WHEN** formatting a `val` whose initializer is a template of 120 characters with the part `${format(first, second, third)}`
- **THEN** the template stays on one line with the part unbroken

### Requirement: Templates That Cannot Be Flat Are Kept Verbatim
A template whose expression parts hold a comment, were written over several lines, or hold an expression that the formatter always breaks (a block, a `when`, a closure with several statements) MUST be printed exactly as written, from its opening to its closing quote, with its comments printed once. Such a template written over several lines MUST stay on the line of the `=`, `return` or assignment operator before it.

#### Scenario: A template part over lines kept as written
- **WHEN** formatting `val s = "a ${when (n) {\n    1 -> "one"\n    else -> "many" // plural\n}} b"`
- **THEN** the template is printed exactly as written, after `val s = ` on its first line, with the comment once, and formatting the result again leaves it unchanged

### Requirement: Escaping Modifiers Are Kept
The formatter MUST print `escaping` before each parameter type that has it, in declarations and in function types, followed by one space, and keep `compose` after it.

#### Scenario: Escaping parameters formatted
- **WHEN** formatting `fun b(o:escaping ()->Unit,f:escaping (()->Unit)?=null){}` and `fun c(content:escaping compose ()->Unit,r:(escaping ()->Unit)->Unit){}`
- **THEN** the results are `fun b(o: escaping () -> Unit, f: escaping (() -> Unit)? = null) {}` and `fun c(content: escaping compose () -> Unit, r: (escaping () -> Unit) -> Unit) {}`

### Requirement: Closure Headers Are Kept as Written
The formatter MUST print a closure's header exactly when it was written, including the empty header `{ -> }`, and MUST NOT add, remove or rename a closure parameter, so a closure using the implicit `it` stays without a header.

#### Scenario: Empty headers and implicit it kept
- **WHEN** formatting `val f: (Int) -> Int = {it+1}`, `run {->1}`, `run {  -> }` and `xs.forEach {it -> println(it)}` inside a function
- **THEN** the results are `val f: (Int) -> Int = { it + 1 }`, `run { -> 1 }`, `run { -> }` and `xs.forEach { it -> println(it) }`

### Requirement: Library Declarations Are Printed
`prism fmt` MUST print `external` and `intrinsic` declarations, and intrinsic extension properties, with their modifiers in the canonical order (visibility, `external` or `intrinsic`, `compose`, receiver mode) and no body, each on its own line, and MUST break their parameter lists as any function's.

#### Scenario: Library declarations formatted
- **WHEN** `prism fmt -` reads `public   intrinsic  mut fun <T> List<T>.add( element :T )` and `public intrinsic val <T> List<T>.size :Int`
- **THEN** it writes `public intrinsic mut fun <T> List<T>.add(element: T)` and `public intrinsic val <T> List<T>.size: Int`, each on its own line

### Requirement: Receivers and Context Parameters Are Printed
The formatter MUST print a function type's receiver as written, with no space around its `.` and one space after `compose`, `mut` and `escaping`, and MUST print a context parameter on its own line directly above the rest of its declaration, as `context(name: Type)`. Comments before a declaration MUST stay above its context line.

#### Scenario: Receiver types and contexts printed
- **WHEN** formatting `fun f(a: ColumnScope . ()->Unit, b: escaping  compose  mut Builder.(Int)->Unit) {}` and `// Weight.` followed by `context( scope : RowScope ) public fun Modifier.weight(w: Double): Modifier = this`
- **THEN** the results are `fun f(a: ColumnScope.() -> Unit, b: escaping compose mut Builder.(Int) -> Unit) {}`, and `// Weight.`, `context(scope: RowScope)` and `public fun Modifier.weight(w: Double): Modifier = this` on three lines

### Requirement: Intrinsic Structs Are Printed
`prism fmt` MUST print an intrinsic struct declaration on one line, as its visibility, `intrinsic struct`, its name, its type parameters and its markers, with the spacing of other struct declarations.

#### Scenario: Intrinsic structs formatted
- **WHEN** `prism fmt -` reads `public  intrinsic struct RecomposeScope:Copy`
- **THEN** it writes `public intrinsic struct RecomposeScope : Copy` and a newline

### Requirement: Writable Intrinsic Properties Are Printed
`prism fmt` MUST print an `intrinsic var` declaration on one line, as it prints an `intrinsic val` declaration, with `var` in place of `val`.

#### Scenario: Writable intrinsic properties formatted
- **WHEN** `prism fmt -` reads `public  intrinsic var<T>MutableState<T>.value:T`
- **THEN** it prints `public intrinsic var <T> MutableState<T>.value: T` and a newline

### Requirement: Attached and Capture Modifiers Are Kept
The formatter MUST print `attached` before each parameter type that has it, in declarations and in function types, followed by one space and keeping `compose` after it, and `capture` directly before `fun` after `intrinsic`.

#### Scenario: Attached parameters and a capture function formatted
- **WHEN** formatting `fun b(o:attached ()->Unit,c:attached compose ()->Unit,r:(attached ()->Unit)->Unit){}` and `public  intrinsic capture fun<T>MutableState<T>.weak():WeakReference<MutableState<T>>`
- **THEN** the results are `fun b(o: attached () -> Unit, c: attached compose () -> Unit, r: (attached () -> Unit) -> Unit) {}` and `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`

### Requirement: Imports Are Sorted
The formatter MUST print a file's imports sorted by path, a root-anchored path (`/a/b`) comparing as its text with the leading `/`, so root-anchored imports come before sibling ones, and imports of the same path keeping their order. When a comment lies between the first and the last import, the formatter MUST keep the imports in their written order.

#### Scenario: Imports sorted by path
- **WHEN** formatting `package app\nimport widgets\nimport /prism/core\nimport /app/model\nimport /prism/collections\nfun main() {}`
- **THEN** the result is `package app\n\nimport /app/model\nimport /prism/collections\nimport /prism/core\nimport widgets\n\nfun main() {}\n`

#### Scenario: Imports with comments among them are kept in order
- **WHEN** formatting `import widgets\n// The core.\nimport /prism/core\nfun main() {}`
- **THEN** the result is `import widgets\n// The core.\nimport /prism/core\n\nfun main() {}\n`

### Requirement: Operator Syntax Is Printed
`prism fmt` MUST print `operator` in the canonical modifier order (visibility, `external` or `intrinsic`, `operator`, `compose`, receiver mode), index lists as `a[i, j]` with one space after each comma, and `in` and `!in` with one space on each side, breaking an index list as an argument list.

#### Scenario: Operator syntax formatted
- **WHEN** `prism fmt -` reads `public  operator mut fun Bag.get( index :Int ):&mut Item = &mut items[index]`, `val c = m[ 1,2 ]` and `val b = x  !in xs`
- **THEN** it writes `public operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]`, `val c = m[1, 2]` and `val b = x !in xs`

### Requirement: Vararg Parameters and Spread Arguments Are Printed
The formatter MUST print `vararg` and one space before a vararg parameter's name, and `*` directly before a spread argument's expression, after `name = ` when the argument is named.

#### Scenario: Vararg and spread formatted
- **WHEN** formatting `fun f( vararg   xs:Int,last:Int=0):Int=g(  *  xs)+h(items= * xs)`
- **THEN** the result is `fun f(vararg xs: Int, last: Int = 0): Int = g(*xs) + h(items = *xs)`

## Design Rationale

Decisions from `prism-fmt` (planned autonomously on 2026-10-17, reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **An AST printer over a document algebra** (D1). The formatter parses the file and prints its syntax tree into a Wadler/Prettier document (text, `line`, `softline`, hard line, indentation, `group`, `if_break`, and a line-comment primitive), which a renderer lays out at width 100 with Prettier's `fits` rule; a group holding a hard line or a line comment is broken. Breaks are placed only where `syntax`'s newline rules keep the parse (after an operator, `=`, `->` or `(`, before `.`, `?.` and `?:`, inside parentheses and brackets). It lives in `crates/prism-formatter` (`format_source`), which depends on `prism-ast`, `prism-lexer` and `prism-parser` only, so formatting resolves no imports, per the single-file compilation unit invariant. Rejected: a token-stream reindenter (it cannot decide where to break or join, and keeps each author's line breaks); a layout without width (very tall or very long code); printing from the typed tree or MIR, which have desugared away what the user wrote.
- **Comment placement by list items, with a fallback at nodes** (D3). A cursor over the comment list only moves forward, so the output keeps the comments' order by construction. Before each item of a list (declaration, member, statement, branch, parameter, argument), the comments that start before it are printed on their own lines, or inline for a block comment followed by the item; a comment on the line where an item ends trails it after one space; comments left before a closing `}` or `)` go on their own lines before it. In comma lists, comments after an item follow its comma, where the next run finds them again. A comment between an operand and its operator stays before the operator, one after an operator on its line stays there, and one before a `.` of a call chain breaks the chain there. Comments before a block's `{` move inside the block when one of them ends its line, so the `{` stays on its header's line (from the review). Any other comment is printed before the next node that starts after it; a line comment there follows code after one space and ends the line, and what it pushes to the next line is indented one level deeper than the line it ends (from the review). A line comment is a document primitive that ends its output line, rather than a deferred "line suffix", whose reordering against a later block comment would break the order guarantee. Rejected: Prettier's full attachment (enclosing, preceding and following node with per-node handlers), at a much larger cost for rare positions.
- **Meaning is verified on every run** (D4). `format_source` parses its own output, erases every span of both trees with an exhaustive walk (so a new AST field must be handled) and compares them, and compares the comment texts in order. A mismatch is `FormatError::Internal`: the file is left untouched and the error is reported as a formatter bug, which takes no `diagnostics` code since it is not a diagnostic of the user's source. This is Black's AST safety check: every printer bug, including a line break that the newline rules give a meaning, becomes a refusal instead of a silent change. Idempotency is checked by the corpus tests only (every fixture and example, and variants with comments before every token, a trailing comment on every line, squeezed whitespace and lengthened names), since checking it on every run would double the cost for a property that does not affect correctness. Rejected: comparing token streams, which cannot work once separators, trailing commas and redundant `()` are dropped.
- **Written forms the tree does not record are read from the tokens** (D5). The printer keeps the lexed tokens and reads back literal spellings, expression parentheses (nested pairs collapse; parentheses that precedence needs are added as a safety net), an explicit `private`, the struct shorthand form and a shorthand field's `val`/`var`, braced bodies, and whether a struct or enum body's braces are on one line. Type parentheses are not recorded, so only the ones meaning needs are printed (`(() -> Unit)?`, `(&Int)?`, `(Int?)?`). Separators, trailing commas, empty `()` before a trailing lambda and the import separator are normalized, since the tree is the same either way. Rejected: removing redundant parentheses (it second-guesses `(a && b) || c`); always printing or always dropping `private`; converting shorthand structs to block form.
- **Style: the Kotlin coding conventions** (D6). Four-space indentation, also for continuation lines, and width 100. Lists break one item per line with a trailing comma; type-parameter lists, type-argument lists and types never break. The right side of `=` and a `when` branch body stay on the line when they are a call, closure, block, `when` or braced `if`, which break inside themselves, and otherwise move to the next line. Binary operators break after the operator, which also keeps the parse (a line starting with `-` is a new statement), except `?:`, which starts the continuation line. A chain with at least two calls breaks before each `.` or `?.` that starts a call, the receiver and its field accesses staying on the first line; a chain of field accesses alone never breaks. `when` is always one branch per line; blocks are always broken. Trailing comments get one space, with no column alignment. Rejected: rustfmt's style; widths 80 and 120.
- **Trailing commas only in broken lists** (D7), at declaration and call sites alike, as ktfmt does, and never before a trailing lambda. Rejected: never printing them; keeping each author's.
- **At most one blank line, kept where written** (D8). A run of blank lines between items or around comments becomes one; none after `{`, before `}` or at the start of the file. One blank line follows the package declaration and the imports, and separates top-level declarations unless both are `val`s (IntelliJ's Kotlin defaults). Rejected: keeping adjacent declarations adjacent (gofmt); deciding by whether a declaration spans several lines, which depends on the width and would not be idempotent.
- **The author's one-line choice for bodies and closures** (D9). A struct or enum body written on one line without comments, and a closure with a single statement or expression written on one line, stay on one line when they fit; one written over lines stays broken, so `remember { mutableStateOf(0) }` stays compact while a `Column { ... }` layout stays expanded. This is Prettier's rule for object literals and gofmt's for function literals, and it is idempotent because the output's line structure is what the next run reads. Rejected: always broken (`enum Bit { Zero, One }` on four lines); always joining when it fits.
- **Files with syntax errors are refused** (D10) with the `prism check` diagnostic, untouched, while the other files are still formatted, as gofmt and rustfmt do. Formatting up to the error and a token-level fallback for broken files were rejected: the parser has no error recovery, and a fallback would be a second formatter.
- **The command surface** (D11). `prism fmt [--check] <path>...` walks directories in sorted order, skips dot-directories and does not follow symbolic links to directories; `-` is standard input to standard output (gofmt, Black). Changed files are listed (`gofmt -l -w`), unchanged files are never written, and `--check` lists and fails (`rustfmt --check`). There is no configuration file and no option: a formatter configuration file would be a build file, against the zero-build-file invariant, and one style is the point of a canonical formatter. The walk and the per-file loop live in `prism_formatter::files`, so `main.rs` only parses arguments and renders diagnostics.
- **Spec placement and tests** (D12). This capability holds the command, the layout and the guarantees; the comment-collecting lexer mode is lexer behaviour and lives in `syntax`. Layout scenarios are golden files in `crates/prism-formatter/tests/golden/`, named after their scenario slugs, and command scenarios are CLI tests in `crates/prism-cli/tests/fmt_cli_tests.rs`, since the `prism check` fixture harness cannot express an expected output.
- **No lossless syntax tree yet** (D13). A lossless concrete syntax tree (rust-analyzer's rowan, Roslyn) would make the formatter and the LSP simpler and more exact, but means replacing the parser's output type, which every stage consumes. The formatter recovers what the AST lacks from the tokens (D5) and the comments (D2 of `syntax`); a later lossless tree would replace both without changing the formatter's behaviour or its tests.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Templates are printed as written, on one line** (D14). Text runs and `$name` parts come from their tokens, escapes included, and `${x}` is never rewritten to `$x` or back (that is a lint). Each `${...}` expression is printed by the usual rules inside a `flat` document, so a template never breaks across lines, as Prettier does for JavaScript template literals; a template whose parts hold a comment, were written over several lines or always break (blocks, `when`, closures of several statements) is printed verbatim, with the comment cursor moved past its comments. The corpus variants insert and squeeze nothing between the text-level tokens of a template. Rejected: always verbatim (inconsistent spacing), and breaking inside `${}` (moves text).
- **A multi-line template stays on the line of its `=`** (D16). Such a verbatim template forces its groups to break, so it is kept on the line of the `=`, `return` or assignment operator, as a `when` or braced `if` is; a one-line template that does not fit still moves to the next line, as a long string literal does. Moving a multi-line one would misalign its later lines, printed as written. Cost: one check of the template's source span per initializer.

Decisions from `implicit-it` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Closure headers are kept as written** (D2, BREAKING): since `{ -> 1 }` declares zero parameters and `{ 1 }` may have an implicit `it`, the printer prints a header whenever the tree records one, including `{ -> }`, and never adds, removes or renames a parameter; the meaning check compares headers and erases only the arrow's span. Before, an empty header was printed as `{ 1 }`, which no fixture or example wrote.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A context parameter goes on its own line above its declaration** (D11), like an annotation, which keeps scoped modifiers' signatures within the line width; comments above a declaration stay above its context line. Receiver function types print as written, with one space after `compose`, `mut` and `escaping` and none around the `.`. The meaning check compares receivers and contexts. Rejected: the context on the declaration's line, which made most scoped modifiers' parameter lists break.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **Intrinsic structs print on one line** (D16), as their visibility, `intrinsic struct`, name, type parameters and markers, like other struct headers.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`intrinsic var` prints as `intrinsic val` does** (D4), with `var` in place of `val`.

Decisions from `explicit-library` (planned and implemented on 2026-10-20, and logged in `docs/decisions-for-review.md`):

- **Imports are sorted** (D9). Explicit imports multiply (a UI file imports up to five library modules), and an order that depends on who added which import first churns diffs, so the formatter sorts a file's imports by path text, a root-anchored path with its leading `/`, which puts root-anchored imports before sibling ones and library imports among the root-anchored ones; a stable sort keeps duplicates in order. Imports have no order to preserve: `E4012` and the Phase 1 binder treat them as a set, and only the check key hashes them in order. A comment from the first import to the end of the last import's line keeps the written order, so no comment moves relative to code; the verifier compares imports as sorted lists. Rejected: the written order, blank-line groups (library, root-anchored, sibling), and moving comments with their import.

Decisions from `operator-conventions` (planned autonomously on 2026-10-20 for part L1 of the review's agreed item A4, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the conventions themselves are in `operators`):

- **Operator syntax is printed, with no new layout rule** (D17): `operator` in the canonical modifier order, index lists like argument lists, `in` and `!in` spaced.
