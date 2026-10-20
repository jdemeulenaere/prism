## ADDED Requirements

### Requirement: Attached and Capture Modifiers Are Kept
The formatter MUST print `attached` before each parameter type that has it, in declarations and in function types, followed by one space and keeping `compose` after it, and `capture` directly before `fun` after `intrinsic`.

#### Scenario: Attached parameters and a capture function formatted
- **WHEN** formatting `fun b(o:attached ()->Unit,c:attached compose ()->Unit,r:(attached ()->Unit)->Unit){}` and `public  intrinsic capture fun<T>MutableState<T>.weak():WeakReference<MutableState<T>>`
- **THEN** the results are `fun b(o: attached () -> Unit, c: attached compose () -> Unit, r: (attached () -> Unit) -> Unit) {}` and `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`
