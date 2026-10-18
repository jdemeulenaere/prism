# Formatting Specification Delta

## ADDED Requirements

### Requirement: Templates Are Printed as Written
The formatter MUST print the text runs and name parts of a string template exactly as written, escapes and unicode sequences included, and MUST keep each part's written form (`$x` or `${x}`). Each expression part MUST be printed by the usual rules for its expression, on one line: a template MUST NOT be broken across lines, whatever its width.

#### Scenario: Template text and parts kept
- **WHEN** formatting `val s = "Count: $count, sum ${a+b}, \$5 \u{1F680} ${ name }"`
- **THEN** the result is `val s = "Count: $count, sum ${a + b}, \$5 \u{1F680} ${name}"`

#### Scenario: A long template is not broken
- **WHEN** formatting a `val` whose initializer is a template of 120 characters with the part `${format(first, second, third)}`
- **THEN** the template stays on one line with the part unbroken

### Requirement: Templates That Cannot Be Flat Are Kept Verbatim
A template whose expression parts hold a comment, were written over several lines, or hold an expression that the formatter always breaks (a block, a `when`, a closure with several statements) MUST be printed exactly as written, from its opening to its closing quote, with its comments printed once.

#### Scenario: A template part over lines kept as written
- **WHEN** formatting `val s = "a ${when (n) {\n    1 -> "one"\n    else -> "many" // plural\n}} b"`
- **THEN** the template is printed exactly as written, with the comment once, and formatting the result again leaves it unchanged
