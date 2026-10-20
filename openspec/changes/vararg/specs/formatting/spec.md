## ADDED Requirements

### Requirement: Vararg Parameters and Spread Arguments Are Printed
The formatter MUST print `vararg` and one space before a vararg parameter's name, and `*` directly before a spread argument's expression, after `name = ` when the argument is named.

#### Scenario: Vararg and spread formatted
- **WHEN** formatting `fun f( vararg   xs:Int,last:Int=0):Int=g(  *  xs)+h(items= * xs)`
- **THEN** the result is `fun f(vararg xs: Int, last: Int = 0): Int = g(*xs) + h(items = *xs)`
