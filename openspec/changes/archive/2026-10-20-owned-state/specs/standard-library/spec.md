## ADDED Requirements

### Requirement: Intrinsic Struct Markers Are Unconditional
The markers the catalog gives a generic intrinsic struct MUST hold for every type argument, unlike the conditional markers of declared generic structs: `WeakReference<MutableState<String>>` is `Copy`, and `MutableState<Int>` is not, although `Int` is.

#### Scenario: A weak reference of a String state is Copy
- **WHEN** `fun f(s: &MutableState<String>): Int { val r = s.weak(); val a = r; val b = r; return 1 }` is checked
- **THEN** the file typechecks

#### Scenario: A state of an Int is not Copy
- **WHEN** `fun g(s: MutableState<Int>): Int { val a = s; val b = s; return 1 }` is checked
- **THEN** the borrow checker emits `E5001` at the second `s`

### Requirement: Capture Functions Are Library Declarations
A `capture` function MUST be an intrinsic extension that the catalog lists as the capture function of its receiver's type, declared in that type's module, with a shared receiver, no parameters, no type parameters but its receiver's, and the result `WeakReference` of its receiver type. A type MUST have at most one. Any other `capture` declaration MUST be `E4029`. The compiler MUST find a type's capture function through its declarations, never by the type's name.

#### Scenario: A capture function the catalog does not list
- **WHEN** a library test module declares `public intrinsic capture fun String.weak(): WeakReference<String>`
- **THEN** the type checker emits `E4029`, saying that the compiler provides no capture function for `String`

#### Scenario: A capture function of the wrong shape
- **WHEN** a library test module of `/prism/compose/runtime` declares `public intrinsic capture fun <T> MutableState<T>.weak(n: Int): WeakReference<MutableState<T>>`
- **THEN** the type checker emits `E4029`, saying that a capture function takes no parameters

## MODIFIED Requirements

### Requirement: The State Declarations of the Compose Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T>`, `MutableState<T>` and `WeakReference<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the member `update` of `MutableState<T>`, the capture functions `weak` of `State<T>` and of `MutableState<T>`, the member `get` of `WeakReference<T>`, and the composable `SideEffect`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T>`, `public intrinsic struct MutableState<T>`, `public intrinsic struct WeakReference<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>`, `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`, `public intrinsic fun <T> WeakReference<T>.get(): (&T)?` and `public intrinsic compose fun SideEffect(effect: attached () -> Unit)`

#### Scenario: State handles need no import
- **WHEN** a file without imports declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the signature of `counter` as `public fun counter(): MutableState<Int>`

### Requirement: The Value of a State Is Typed by Its Own Rule
`value` of `State<T>` and `MutableState<T>` MUST be typed by the catalog's rule rather than as a computed property: a copy for a `Copy` `T`, a place for any other `T`, and an assignment target on a `MutableState<T>` reached by value or through any reference (compose, "Reading a Copy Value", "A State's Value Is a Place", "Writing a Value").

#### Scenario: Assigning the value of a read-only handle
- **WHEN** `fun f(s: &State<Int>) { s.value = 1 }` is checked
- **THEN** the type checker emits `E4003`, saying that a `State<Int>` is read-only, with help suggesting a `&MutableState<Int>` parameter

### Requirement: Remember Takes Up to Three Keys
A call of `remember` (`fun <T> remember(calculation: () -> T): T`, whose result the catalog types as a copy of a `Copy` `T` and as a shared borrow of any other `T`) MUST also accept up to three keys before the calculation, each typed by a type parameter of its own bounded by `Copy` and borrow-free (compose, "Remembering With Keys").

#### Scenario: Keys of remember
- **WHEN** `compose fun f(a: Int, b: Bool) { val x = remember(a, b) { 1 }; val y = remember(a, b, 'c', 1.5) { 2 } }` is checked
- **THEN** the type checker accepts the first call and reports an argument count error (`E4004`) for the second, which has four keys

## REMOVED Requirements

### Requirement: Intrinsic Struct Markers Hold for Every Type Argument
**Reason**: State objects are no longer `Copy`, so "A state of a String is Copy" no longer holds; the rule itself is unchanged.
**Migration**: See "Intrinsic Struct Markers Are Unconditional", which states the same rule with `WeakReference<T>`.
