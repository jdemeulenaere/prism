## ADDED Requirements

### Requirement: Field Visibility Modifiers Are Printed
The formatter MUST print a field's visibility modifier as written, an explicit `public` included, before `val` or `var`, in block and shorthand structs, with one space after it.

#### Scenario: Field modifiers formatted
- **WHEN** formatting `struct Account{private  var balance:Int,public val owner:String}` and `struct Session( private val id:Int )`
- **THEN** the results are `struct Account { private var balance: Int, public val owner: String }` and `struct Session(private val id: Int)`
