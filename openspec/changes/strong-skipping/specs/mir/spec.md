## ADDED Requirements

### Requirement: Composable Calls Pass the Versions of Their Borrowed Arguments
A composable body MUST take, after its change words, one `Int` version argument per parameter that is a borrow, shared or mutable, or a nullable of one, of any type, in parameter order, so that no conversion of a function value in place (a substituted type argument, a `&T` parameter used as a `&mut T` one) changes a calling convention; only a shared borrow of a type other than a state object uses it as its identity, and every composable call MUST pass one per such parameter: `0` for none, `1` for a permanent source, a version argument or capture for a borrow derived from one, or the result of a `state version` call on the state object whose read the borrow derives from.

#### Scenario: Versions passed to a row
- **WHEN** `prism dump-mir --composed` runs on a file declaring `compose fun Show(s: &String) {}` and `compose fun Screen(items: &State<List<String>>) { for (s in items.value) { Show(s) } }`
- **THEN** `main::Show` takes `$version0` after its change word, and `main::Screen` calls `state version<List<String>>` with the state object it calls `state read<List<String>>` with, and passes a copy of that version in each call of `main::Show`

#### Scenario: Versions of locals, top-level values and parameters
- **WHEN** `prism dump-mir --composed` runs on a file declaring `val Title: String = "t"`, `compose fun Show(s: &String) {}` and `compose fun Screen(s: &String) { val local = "a" + "b"; Show(local); Show(Title); Show(s) }`
- **THEN** the three calls of `main::Show` in `main::Screen` pass `const 0`, `const 1` and a copy of `main::Screen`'s `$version0` as their version arguments

### Requirement: Composable Closures Capture the Versions of Their Borrowed References
A composable closure that captures a dereference of a reference MUST also capture that reference's version, after its other captures: by a shared borrow of the local holding it when the closure borrows its captures, by a copy when it owns them. Its body MUST use that capture as the version of borrows derived from the captured reference.

#### Scenario: A key lambda captures a version
- **WHEN** `prism dump-mir --composed` runs on a file declaring `compose fun Show(s: &String) {}` and `compose fun Screen(items: &State<List<String>>) { for (s in items.value) { key(s.length) { Show(s) } } }`
- **THEN** `main::Screen` builds the closure value of the `key` lambda from a borrow of `s` and then a borrow of the local holding the version it passes for `s`, and the lambda's body passes a copy of its last capture local as the version argument of its call of `main::Show`

## MODIFIED Requirements

### Requirement: Composed MIR
After borrow checking, the MIR of every file MUST be composed. A composable body MUST take the composer, its change words and its version arguments as `Int` arguments after its declared ones, start its body group first and end it before every return, and every composable call MUST pass the caller's composer, change words for the callee and the versions of its borrowed arguments. Composed MIR MUST hold no region marker and MUST be well formed.

#### Scenario: Composer passed to composable calls
- **WHEN** `compose fun Label(n: Int) {}` and `compose fun Screen() { Label(1) }` are compiled
- **THEN** `prism dump-mir --composed` shows the call of `Label` passing `Screen`'s composer and a change word marking its argument static

#### Scenario: A composable lambda takes the composer
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Screen() { Column { } }` are composed
- **THEN** the body `main::Screen#closure0` takes a composer and a change word, and `Column`'s call of `content` passes them

### Requirement: Skippable Bodies Keep, Compare and Skip
A skippable body MUST take its replaceable path when a version argument is `0`, keep each argument in a slot of its restart group (a borrow as its address and version, in a slot of its reference type), compare it when its bits are uncertain, test whether it may skip, and on skipping drop what it owns and go to its group's end. It MUST end its restart group with its restart closure, `<symbol>#restart`, which captures nothing, reads the kept arguments and calls the composable with the force bit.

#### Scenario: A skippable body and its restart closure
- **WHEN** `compose fun Badge(n: Int) { println(n) }` is composed
- **THEN** `prism dump-mir --composed` shows `main::Badge` compare its argument with its kept slot of type `Int`, a skip path that ends its restart group, and the body `main::Badge#restart`, without captures, which reads kept slot 0 as an `Int` and calls `main::Badge` with it and the force bit

#### Scenario: A skippable body keeping a borrow
- **WHEN** `compose fun Show(s: &String) { println(s) }` is composed
- **THEN** `prism dump-mir --composed` shows `main::Show` test `$version0` against `0` before its restart group, with a path that starts a replaceable group, keep `s` and `$version0` in a slot of type `&String`, and the body `main::Show#restart` read kept slot 0 as a `&String` and its version and call `main::Show` with both and the force bit

### Requirement: State Operations Are Calls
Every state operation MUST lower to `Call` terminators of state operations, each with the value type as its type argument and the state object passed by shared borrow: creating a state; reading it, which gives a shared reference to its value tied to that borrow; reading its version; writing it; taking a shared or exclusive flag and ending it; moving its value out and putting one back; making a weak handle and resolving one. A state place MUST be the dereference of the reference a read gives.

#### Scenario: A compound assignment through a handle
- **WHEN** `prism dump-mir` runs on `fun f(s: &MutableState<Int>) { s.value += 1 }`
- **THEN** the body borrows `*s` into a temporary once, calls `state read<Int>` with it, adds 1 to a copy of the referent with an overflow check, and calls `state write<Int>` with the temporary and the sum

#### Scenario: An update call
- **WHEN** `prism dump-mir` runs on `fun f(s: &MutableState<Int>) { s.update { n -> n * 2 } }`
- **THEN** the body calls `state take<Int>`, calls the closure value with the taken value moved, and calls `state put<Int>` with the result moved
