# Proposal: monomorphization

## Why

`generics` checks every generic declaration once into elaborated polymorphic MIR, but the backend still rejects every call of a generic function with `E6001` ("calls of generic functions"), so no generic code runs. Milestone 2 is built on generics: `State<T>`, `remember<T>`, `mutableStateOf<T>`, `List<T>` (`collections`) and every generic helper an app writes. This change, item 20 of "Then: the road to Milestone 2" in `ROADMAP.md`, makes generic code run in the Tier 1 backend. Each generic body is instantiated per concrete type argument list, in the same file and across files. The change decides what an exporter ships so an importer can instantiate a generic without its source, where instances live, how they are named and called, and what the instance-specific rewrites mean: drops of type parameters, flattened `T?`, and `?:` on a `T` that turns out nullable. It also rejects polymorphic recursion, which would need infinitely many instances. Caching instances across runs, with the Monomorphization Hash moved onto the shipped MIR, is split into the next item, `instance-cache` (D1).

## What Changes

- **Generic code runs.** `prism run` compiles a separate instance of each generic function, extension and default value generator for every list of concrete type arguments the program uses. That covers calls from the same file, from importers, and from inside other instances, including generics of modules the caller does not import. An instance runs the checked generic body with its type arguments substituted, and nothing in it is checked again. Uncalled generics are still never compiled.
- **What an exporter ships: generic records.** For every generic function of a file, private ones included, the compiler keeps a record: its elaborated polymorphic MIR (the body, its default generators and its closures) and the definitions of the types those bodies mention. An instance is built from the generic's record and the definitions of its type arguments alone, never from another body of the defining file. Non-generic helpers, private ones included, are called by symbol through the function table, so their bodies are never shipped (D2).
- **Where instances live: one instance unit per instance.** Each instance (generic, type arguments) is compiled into its own code unit, once per program however many callers use it. It has its own slot in the function table, named `lib/box::keep<Int>`, `lib/box::lib/box.Box.get<Int>` or `main::f<Int>#default(y)`. Callers call it through the slot like any function, so a caller's code never depends on a generic body, and `hot-reload` can later regenerate a changed generic's instances and repoint their slots. Drop and clone glue that an instance needs is local to its unit (D3, D4).
- **Substitution over MIR.** Every type in an instance is substituted. A drop whose type no longer needs one becomes a jump, and the others run the instance's glue. A `clone()` of a `T : Clone` runs the clone glue of the argument. A `T?` whose argument is itself nullable flattens: wrapping such a value is the value itself, and reading its payload reads the value (D6).
- **BREAKING: `?:` on a type parameter tests for null.** `a ?: c` with `a: T` takes the fallback when an instance's `T` is nullable and `a` is `null`, as in Kotlin and as `val w: T? = a; w ?: c` already does after flattening. It used to lower to `a` alone, so the fallback never ran (D7).
- **Polymorphic recursion is rejected.** A generic function that instantiates itself with a growing type argument, directly or through other generics of its file (`fun <T> f(x: T) { f(Box(x)) }`), or a generic type that reaches itself through a `Heap` with a growing argument, is a new `E4027` (`PolymorphicRecursion`) at its declaration, whether or not anything instantiates it. A cycle through generics of several files, which only an import cycle allows, is found when the program is compiled: a new `E6004` (`PolymorphicRecursionAcrossFiles`) (D8).
- **Backend checks on instances.** The `E6001` check runs on each instance, and reports a construct the backend lacks (a closure inside a generic body, for example) once, at the generic's source. "Calls of generic functions" leaves the `E6001` list (D10).
- **Dumps.** `prism dump-mir --instances <file>` prints the elaborated MIR of every instance the program from that entry needs, and `prism dump-clif --instances <file>` their Cranelift IR. Fixtures pin substitution with `instances-mir.snap` (D11).
- **`ROADMAP.md`.** A new item 21, `instance-cache`, follows this one. It ships the records through the build cache, moves the Monomorphization Hash from the ABI onto the record, keys instances by their type arguments' definitions too, and reports instances per program in `--explain-cache`. Later items are renumbered (D1, D13).
- Not in scope: caching instance code or records across runs, and changing `prism check`'s instance records or the Phase 1 Monomorphization Hash (`instance-cache`); closures inside generic bodies (`closures`, which instantiates them with their parent); sharing one instance between type arguments of the same layout; generic functions as values (`E4013`).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `generics`: instances run the checked body, `?:` on a type parameter tests for null, and polymorphic recursion is rejected within a file (`E4027`) and across files (`E6004`).
- `mir`: `?:` on a type-parameter left side lowers through a `T?` temporary, instances are substituted and flattened bodies, and `prism dump-mir --instances`.
- `code-generation`: generic bodies are compiled as instances, instance units with their own slots and symbols, glue local to instance units, the `E6001` check on instances, and `prism dump-clif --instances`.
- `compilation-and-abi`: Phase 3 builds an instance from the generic's record and its type arguments' definitions alone, so a file's code never depends on a generic body.
- `execution`: the runtime meaning of instances, among them drops and clones of type-parameter values, flattened nullables, `?:` on a type parameter and runtime errors located in a generic body.

## Impact

- `crates/prism-typed-tree`: a growing-cycle search over instantiation edges (`instantiation.rs`), shared by the type checker and the backend, and `TypeTable::restricted_to`.
- `crates/prism-typechecker`: the `E4027` check over a file's generic calls and generic type definitions (`recursion.rs`), and `TypeError::PolymorphicRecursion`.
- `crates/prism-mir`: `Body::type_arguments`, the `?:` lowering in `build/control_flow.rs`, generic records (`record.rs`), body instantiation with the flattening rewrites (`instantiate.rs`), and the dump of instance headers.
- `crates/prism-codegen`: instance collection and the `E6004` check (`instances.rs`), instance symbols (`symbols.rs`), code units named after their instance (`unit.rs`, `glue.rs`), `Construct::GenericCalls` removed (`supported.rs`), instances linked (`link.rs`, `program.rs`) and dumped (`dump.rs`).
- `crates/prism-cli`: `run.rs` compiles instances, `--instances` for `dump-mir` and `dump-clif`, rendering of `E4027` and `E6004`, `instances-mir.snap` in the fixture runner, and fixtures under `generics/`, `mir/`, `code-generation/`, `compilation-and-abi/` and `execution/`, every executable one under `--check-allocations`.
- Specs: the deltas above. The diagnostics catalog gains `E4027` and `E6004`, and `E6001` loses generic calls; it is edited directly.
- `docs/architecture.md` §1: on archive, the open decision on where instances live is settled, and "Re-monomorphization on Generic Body Change" stays for `instance-cache`.
- **Prerequisites:** `runtime-strings-and-drops` (glue and `--check-allocations`), `payload-patterns` and `heap-boxes` (the generic list fixture) and `string-templates` must be archived first. Several deltas modify requirements that those changes add or modify (see `design.md`, "Baseline").
