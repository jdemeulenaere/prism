# Proposal: closures

## Why

The language has had closures and function values since the front end: trailing lambdas, function-typed parameters and fields, closures that capture by inferred borrow (`borrowck-exclusivity`), function names used as values. The backend runs none of them. Every function value, every closure and every struct holding one is `E6001` ("function values and closures"), so `items.forEach { ... }`, `repeat(3) { count += 1 }`, `remember { mutableStateOf(0) }` and every generic that takes a function (`map`, `remember`) cannot run. This change is the first half of item 22 of "Then: the road to Milestone 2" in `ROADMAP.md`: closure conversion in the backend for the closures the language has today. It decides how a function value is represented, how a closure's code is called through the function table so `hot-reload` reaches it, where a closure's environment lives, how function names become values, and what happens to the function-type subtyping steps that change a representation, which `typed-tree` left as a risk. The second half of the item, escaping closures that capture by move (needed by `Button(onClick = { ... })`), is split into a new item 23, `escaping-closures`, whose direction this change records because the representation chosen here must already serve it (D1, D14).

## What Changes

- **Closures and function values run.** `prism run` compiles every closure body as a function of its own, calls function values, builds closures that capture by borrow, and runs function names and enum variant constructors used as values. Structs, enums and nullables holding function values are laid out and dropped. `E6001` no longer names "function values and closures".
- **A function value is a descriptor and an environment** (D2, D3). It is 16 bytes: the address of its closure's *descriptor*, static data of the closure's code unit holding the address of the closure code's function-table slot and the entry that drops the environment, and the address of its *environment*, or null when the closure captures nothing. A call loads the code from the slot the descriptor names, so after `hot-reload` repoints a closure's slot, function values already stored (an `onClick` in the UI tree) run the new code. `(() -> Unit)?` adds a tag, as other nullables of values in memory do.
- **A closure's code is a function with its own slot** (D4), named `<parent symbol>#closure<i>` as MIR already names closure bodies, taking its environment's address before its parameters.
- **Environments of today's closures hold pointers, in the creating frame** (D5). A closure that captures by borrow stores one pointer per captured binding, in capture order, in stack storage of the function that builds it. Building it allocates nothing.
- **Function names and variant constructors as values are adapter closures** (D7). MIR lowers `apply(double)` or `build(Shape.Circle)` as a closure without captures that calls the function or builds the variant, converting the arguments and result when the expected function type differs. MIR no longer has function constants.
- **BREAKING: function values keep their representation when converted** (D8). A function value other than a named function or variant constructor converts to another function type only through steps that do not change a representation: equal types, or `&mut T` to `&T`. `val g: (Int?) -> Unit = show; each(g)` for `each(f: (Int) -> Unit)` is now a type mismatch, with help suggesting `{ x -> g(x) }`. `each(show)` and closure literals are unaffected. This closes the risk that `typed-tree` D6 left for `closures` and `heap-boxes`.
- **Dropping a function value runs its descriptor's drop entry** (D9). No closure of this change owns its environment, so every entry is empty, but a function value's drop is already dynamic, so `escaping-closures` only fills the entry. Function types stay neither `Copy` nor `Clone`.
- **Closures of generic bodies run as part of their parent's instance** (D11), named `<instance symbol>#closure<i>` in the instance's unit, which `monomorphization` left to this change.
- **Composable closures stay `E6001`** (D10), named as composable functions, until `compose-transform`. Values of composable function types may be built only by them, but are laid out, moved and dropped like any function value.
- **`ROADMAP.md`.** Item 22 is narrowed to this change. A new item 23, `escaping-closures`, holds the second half, and later items are renumbered (D1).
- Not in scope: escaping closures and capture by move, an `escaping` parameter marker, heap environments and the `E5007` revision (`escaping-closures`, D14). Also out of scope: `Copy` or `Clone` function values, adapters for converting arbitrary function values, a null-descriptor niche for nullable function values, sharing adapters between uses of one function, and calling composable function values.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `code-generation`: the representation of function values, descriptors, closure code with its own slots and symbols, environments of borrowing closures, calls of function values, drop glue of function types, closures of generic instances, and the `E6001` list and dumps without function values.
- `execution`: the runtime meaning of calling closures and function values, of closures reading and writing the variables they capture, of function names and variant constructors as values, of function values held in structs, nullables and top-level values, of closures in generic code and default values, and of runtime errors inside closures.
- `mir`: function names and variant constructors used as values are lowered as adapter closures.
- `type-system`: function values convert to other function types only without changing their representation.

## Impact

- `crates/prism-typed-tree`: `Type::converts_in_place`, the representation-preserving conversion relation shared by the type checker and the backend.
- `crates/prism-typechecker`: the D8 restriction in `fit`, with its help.
- `crates/prism-mir`: adapter closures (`build/closure.rs`, `build/place.rs`), the function constants removed (`mir.rs`, `dump.rs`, `types.rs`, `validate.rs`), and closure bodies carried into instances (`instantiate.rs`, from `monomorphization`).
- `crates/prism-codegen`: function types in `representation.rs` and `layout.rs`, closure bodies compiled with their descriptors (`supported.rs`, `symbols.rs`, `unit.rs`, `link.rs`), environments, closure values and calls of function values (`lower.rs`), drop glue of function types (`glue.rs`), instance units holding closures (`instances.rs`), and `Construct::FunctionValues` removed.
- `crates/prism-borrowck`: none in behaviour. Adapter closures are borrow-checked like any closure.
- `crates/prism-cli`: fixtures under `execution/`, `code-generation/`, `mir/` and `type-system/`, every executable one under `--check-allocations`. The `E6001` fixtures that pinned function values move to composable closures and unknown types. `examples/run/` uses closures.
- Specs: the deltas above. The diagnostics catalog row of `E6001` loses function values and closures; it is edited directly.
- `docs/architecture.md`: on archive, §2 and §4 no longer list closures as planned. §3's design note on escaping closures stays for `escaping-closures`.
- **Prerequisites:** `payload-patterns`, `heap-boxes`, `string-templates` and `monomorphization` must be archived first. Several deltas modify requirements as those changes leave them (see `design.md`, "Baseline"). `instance-cache` comes before this change on the roadmap, but nothing here depends on it.
