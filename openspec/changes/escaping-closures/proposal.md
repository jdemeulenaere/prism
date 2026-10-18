# Proposal: escaping-closures

## Why

Every closure the language has today borrows what it captures, so it cannot outlive the frame that builds it: returning one, storing it in a struct, a box or a generic slot, or keeping it past the scope of a captured binding is `E5007` or `E5006`. Compose needs exactly those closures. In

```
compose fun Counter() {
    val count = remember { mutableStateOf(0) }
    Button(onClick = { count.value += 1 }) { Text("Count: ${count.value}") }
}
```

the `onClick` handler is kept by the UI tree after `Counter` returns, and `struct Item { val onSelect: () -> Unit }` stores a handler in a value. This is item 23 of "Then: the road to Milestone 2" in `ROADMAP.md`. `closures` (D14) chose a representation that leaves room for owned environments and recorded a direction; this change decides the language rules and builds them, with zero GC and zero hidden ARC: a closure that must outlive what it would borrow owns its captures instead, in one heap allocation it frees when it is dropped.

## What Changes

- **Capture by value is inferred, never written** (D1, D2). A closure owns all of its captures when its value may reach a place that needs an owned value: it is returned, stored in a struct field, an enum payload or a box, written through a reference, assigned to a captured variable, passed to a generic parameter or to an `escaping` parameter, or captured by a closure that owns its captures. It owns a single capture when its value may still be live where that captured binding's scope ends (`h = { greet(s) }` with `h` declared outside the block of `s`). Every other capture stays an inferred borrow, so no program that passes `prism check` today changes meaning: capture by value happens only where borrowing is rejected today. The borrow checker decides, in a new first stage on the MIR as lowered, before moves are checked.
- **Capturing by value moves or copies the binding** (D2). Building such a closure copies a `Copy` binding and moves any other into the closure, so a later use of a moved binding is `E5001`, labelled with the closure and the reason it owns its captures. Kotlin's syntax is kept: no `move` keyword, no capture list.
- **`escaping` parameters** (D3, D4). A function-typed parameter that the callee may keep is declared `escaping`: `fun button(onClick: escaping () -> Unit): Button`, also in function types (`(escaping () -> Unit) -> Unit`) and on nullable function types (`escaping (() -> Unit)?`). It is a contextual keyword, recorded in the typed tree, MIR and the ABI. Inside the callee such a parameter holds no borrow and may be stored or returned; at a call, passing a value that may hold a borrow to it is `E5007`. Unmarked function-typed parameters stay as today: they may receive borrowing closures and cannot escape. A function whose parameter is unmarked converts to a function type whose parameter is `escaping`, not the reverse.
- **New `E5010` (`CannotCaptureByValue`)** (D5). A closure that captures a binding by value may capture only a `val`, and only a value it can take: capturing a `var` by value is rejected (the closure would silently stop seeing later writes, unlike Kotlin), as is a non-`Copy` capture of an enclosing closure or an earlier parameter read in a default value. Help suggests copying or cloning it into a `val` first.
- **`E5007` narrows to function values that may hold borrows** (D6). An escaping value is rejected only when it may hold a borrow: a closure capturing a reference, a borrowing closure or an unmarked function-typed parameter by value, or such a parameter itself. The messages and help change accordingly ("declare 'p' as 'escaping'"). Capturing closures stored in local structs, written through references, used as default values, boxed (`heap-boxes` D11) or passed to generic parameters (`generics` D7) now run.
- **Calling a function value borrows it; environments never change** (D7). Owned captures are `val`s, so a call never writes the environment, needs no exclusive access, and works through `&(() -> Unit)`. Passing an owned function value to a parameter moves it, as today. There is no `FnMut`-like type.
- **Owned environments are one exact-sized heap allocation** (D8) made when the closure is built, its captures laid out like a struct's fields, and freed by the drop entry of the closure's descriptor (`closures` D3), `<closure symbol>#drop-environment`, which drops the captures in capture order. **An environment of at most one pointer-sized word that needs no drop is stored in the environment word itself** (D9), so a handler capturing one `Copy` handle (`count`) allocates nothing.
- **A closure returns no borrow of what it owns** (D10): `E5008`, since its environment may be dropped while the result is live.
- **Function types stay neither `Copy` nor `Clone`** (D11). The planned compose engine (`docs/architecture.md` §5) remembers only `Copy` values, moves each rebuilt handler into the node that keeps it, and compares handlers for skipping by descriptor and `Copy` captures, so it needs no copy or clone of a closure.
- **Closures cannot refer to themselves** (D13), so owned environments form a tree and single-owner drops free them all.
- `ROADMAP.md`: the relaxation of `E5007` for closures written through references, stored in local structs or used as defaults is done by this change; the risk of liveness ignoring drops (`borrowck-exclusivity` D5) is closed (D14).
- Not in scope: lending function values to unmarked parameters instead of moving them; mutable environments (`FnMut`); cloneable or `Copy` function values; closures that hold borrows escaping as far as elision would allow; inline environments for borrowing closures; composable closures, which stay `E6001` until `compose-transform`.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `references`: closures capture by inferred borrow or by value; where a closure captures by value; `escaping` parameters; `E5007` for escaping values that may hold borrows; generic slots and boxes accept owning closures; owning closures return no borrows of their captures.
- `ownership`: capturing by value moves or copies the binding (`E5001` after it); `E5010` for captures that cannot be taken by value; the `E4017` wording for a closure moving a capture.
- `syntax`: the `escaping` parameter modifier, a contextual keyword.
- `type-system`: `escaping` in function types and its subtyping.
- `compilation-and-abi`: `escaping` recorded and hashed in the ABI.
- `formatting`: `prism fmt` keeps `escaping`.
- `mir`: closure values built from captures taken by value, and the `Owned` capture mode in elaborated MIR.
- `code-generation`: owned environments, inline small environments and their drop entries.
- `execution`: runtime meaning of closures that own their captures.

## Impact

- `crates/prism-ast`, `crates/prism-parser`, `crates/prism-formatter`: `escaping` on parameters and function-type parameters.
- `crates/prism-typed-tree`, `crates/prism-typechecker`: escaping flags on `Type::Fn` and parameter locals, subtyping, the representation-preserving conversion (`closures` D8), the `E4017` wording.
- `crates/prism-abi`: flags in `AbiType::Fn` and signatures, and the hash.
- `crates/prism-mir`: `Capture`, `CaptureMode::Owned`, escaping flags on calls and argument locals, dump and validation.
- `crates/prism-borrowck`: a new first stage deciding captures by value (`capture_values.rs`), `E5010`, `E5001` labels, the loan flow of owned captures, the narrowed escape check, `E5008` for owned captures.
- `crates/prism-codegen`, `crates/prism-runtime`: owned and inline environments, environment drop functions.
- `crates/prism-cli`: diagnostics rendering and fixtures, every executable one under `--check-allocations`. The `E5007` fixtures that now pass become running programs.
- Specs: the deltas above. The diagnostics catalog gains `E5010` and rewords `E5007` directly.
- **Prerequisites:** `closures`, and through it `payload-patterns`, `heap-boxes`, `string-templates` and `monomorphization`, must be archived first; several deltas modify requirements as `closures` and `heap-boxes` leave them (`design.md`, "Baseline").
