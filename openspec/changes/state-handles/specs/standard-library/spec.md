# Standard Library Delta

## ADDED Requirements

### Requirement: The State Declarations of the Compose Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T> : Copy` and `MutableState<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the members `read` of `State<T>` and `update` and `edit` of `MutableState<T>`, and the composable `SideEffect`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T> : Copy`, `public intrinsic struct MutableState<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T, R> State<T>.read(reader: (&T) -> R): R`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic fun <T> MutableState<T>.edit(editor: (&mut T) -> Unit)` and `public intrinsic compose fun SideEffect(effect: escaping () -> Unit)`

#### Scenario: State handles need no import
- **WHEN** a file without imports declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the result type of `counter` as `prism/compose/runtime.MutableState<Int>`

### Requirement: Intrinsic Struct Markers Hold for Every Type Argument
The markers the catalog gives a generic intrinsic struct MUST hold for every type argument, unlike the conditional markers of declared generic structs: `State<String>` and `MutableState<List<Int>>` are `Copy`.

#### Scenario: A state of a String is Copy
- **WHEN** `fun f(s: MutableState<String>): Int { val a = s; val b = s; return 1 }` is checked
- **THEN** the file typechecks

### Requirement: Writable Intrinsic Properties
An `intrinsic var` property MUST be an intrinsic extension property that can also be assigned, as the catalog lowers it. It MUST be checked against the catalog like an `intrinsic val`, and declaring as `var` a property the catalog lists as read-only, or the reverse, MUST be `E4029`.

#### Scenario: A writable property the catalog lists as read-only
- **WHEN** a library test module declares `public intrinsic var String.length: Int`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `length` as a read-only property

### Requirement: The Value of a State Is Typed by Its Own Rule
`value` of `State<T>` and `MutableState<T>` MUST be typed by the catalog's rule rather than as a computed property: a copy for a `Copy` `T`, a place for any other `T`, and an assignment target on `MutableState<T>` (compose, "Reading a Copy Value", "Borrowing a Non-Copy Value in Composable Contexts", "Writing a Value").

#### Scenario: Assigning the value of a read-only handle
- **WHEN** `fun f(s: State<Int>) { s.value = 1 }` is checked
- **THEN** the type checker emits `E4003`, saying that a `State<Int>` is read-only, with help suggesting a `MutableState<Int>` parameter
