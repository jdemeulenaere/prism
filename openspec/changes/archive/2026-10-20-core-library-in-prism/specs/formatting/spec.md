## ADDED Requirements

### Requirement: Extension Property Getters Are Printed
The formatter MUST print an extension property's header (`val`, type parameters, receiver, `.`, name, `: ` and type) and then its getter. An expression getter MUST follow on the header's line, as ` get() = ` and the expression, when the whole declaration fits the line width, and otherwise start the next line, indented one level. A block getter MUST follow on the header's line as ` get() ` and the block, broken as any function's.

#### Scenario: Getters formatted
- **WHEN** formatting `public  val   Int.dp :Dp\n  get( )=Dp( this )` and `val <T> List<T>.someVeryLongPropertyName: Range get() = computeTheRangeOfThisListFromItsElements(this, 0)`
- **THEN** the first becomes `public val Int.dp: Dp get() = Dp(this)`, and the second, which does not fit in 100 columns on one line, becomes `val <T> List<T>.someVeryLongPropertyName: Range` followed by a line `    get() = computeTheRangeOfThisListFromItsElements(this, 0)`

#### Scenario: A getter with a block formatted
- **WHEN** formatting `val Box.area:Int get(){ return w*h }`
- **THEN** the result is `val Box.area: Int get() {`, then `    return w * h` and `}`, each on its own line
