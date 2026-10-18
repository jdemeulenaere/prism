# Design: codegen-aggregates

## Context

See `proposal.md` for motivation. This is step 15 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17. The orchestrator fixed the scope (structs, enums with and without payloads, nullables, references to them and to fields, `when`, `?.` and `?:`, constructors, field reads and writes, `Copy` aggregates by value, the layouts of types from transitively imported modules) and what stays out (`String`, drop glue and allocation for `runtime-strings-and-drops`, generic bodies for `monomorphization`). Decisions marked **(autonomous)** were made by the proposer and are logged in `docs/decisions-for-review.md`.

Current state (after `codegen-core`):

- **The backend compiles scalars only.** `crates/prism-codegen/src/supported.rs` rejects every struct, enum and nullable with `E6001` (`classify`, `BodyChecker`), and `Drop` terminators unconditionally. `representation.rs` maps `Int`, `Double`, `Bool`, `Char` and references to scalars to one machine value each; a `Range` local is special-cased as three Cranelift variables (`Representation::Range`, `Storage::Range` in `lower.rs`), and a `Range` anywhere else is rejected ("'Range' values outside local variables"). Borrowed scalar locals live in stack slots (`borrowed_locals`), the rest in variables. `lower.rs` handles the `Deref` projection only (and `Field` of a `Range` local).
- **MIR already does the control flow.** `?:`, `?.`, the `(&T)?` view of a `&(T?)`, comparisons with `null` and `when` over variants are lowered to `Discriminant`, `SwitchInt`, `Payload` projections, `WrapNullable` and `Constant::Null` (the `mir` spec). A `when` over payload-free variants switches on the discriminant; any other `when` compares with `==`. `==` over a type that is not `Copy` compares two `Operator` borrows (`_0 = move _3 == move _4` with `_3: &Shape`), so the backend compares referents. Variant payloads are only ever written (`Aggregate(Variant)`): the language has no destructuring yet, so no projection reads a payload field.
- **Non-`Copy` aggregates have drops.** A struct without `: Copy`, even one holding only `Int`s, needs a drop (`prism_mir::needs_drop`), so its elaborated MIR has `Drop` terminators, some guarded by drop flags.
- **A file's type table is incomplete.** `MirFile::types` (a `TypeTable`) holds the file's own structs and enums, the exported ones of its direct imports, and, for a type an imported signature mentions from a module the file does not import, only its marker (`NominalHeader` in `crates/prism-abi/src/abi_type.rs`, the "Markers of Referenced Nominal Types" requirement). A public struct may have a field of a private type (`public struct Outer { val s: Secret }` passes `prism check`), whose definition no importer sees. `E4014` gates member access by the defining module being a direct import (`TypeChecker::is_unimported`), not by whether a definition is known.
- **Phase 1 is per file, unordered.** `extract_abis` (`crates/prism-cli/src/workspace.rs`) reads each file's exported struct and enum headers from its AST (Phase 1a, `declared_types`), then extracts each ABI binding names against its own declarations and its direct imports' headers (Phase 1b, `FileAbi::extract`). No ABI depends on another ABI, so import cycles need no ordering.
- **Infinitely sized types pass the checker.** `struct A { val b: B? }` with `struct B { val a: A }`, or `enum E { X(E), Y }`, typecheck (`ROADMAP.md`, from `fix-cli-workspace-audit`).
- **`println` takes scalars and `String`.** A struct, enum or nullable argument is `E4004` (the `type-system` spec, "Printing an unsupported value").

## Goals / Non-Goals

**Goals:**
- Run programs over structs, enums, nullables and references to them, with every value in its final layout, so later changes add drop glue, strings, instances and closures without changing how aggregates are stored or passed.
- Keep each file's code a function of its own MIR and its direct imports' ABIs, including the layouts of types from modules it does not import, and keep the build cache's keys sufficient.
- A layout dump that tests and later changes (`hot-reload`, `compose-runtime`) can read.
- Every scenario has an executable or layout fixture.

**Non-Goals:**
- Optimizing layouts or code beyond field reordering: no niche filling other than null pointers, no scalar replacement, no aggregates in registers.
- Drop glue, `String`, `Heap<T>` construction, generic bodies, closures, printing aggregates.
- Keeping live values across a layout change during hot reload.

## Decisions

### D1. What the backend compiles after this change (autonomous)

Every non-generic, non-closure body whose values are scalars, `Unit`, structs, enums, nullables, `Range`, and references to any of these, and in which no struct, enum or nullable holds a `String`, a `Heap<T>` or a function value. Concretely, beyond `codegen-core`: `Rvalue::Aggregate` of structs and variants, `Field` and `Payload` projections (also through `Deref`), borrows of any place in memory (`&p`, `&p.x`, `&((*_4) as payload)`), assignments to fields and through references (`(*_1).y = const 3`, `(*_1) = move _2`), `Discriminant` of enums and nullables, `WrapNullable`, `Constant::Null`, the `Subtype` steps that keep a representation (D11), `==` and `!=` over every supported type (D13), `Constant::VariantConstructor` only as the callee of an immediate construction (it does not reach the backend otherwise; as a value it is a function value, `closures`), `Builtin(Clone)` of any supported type (D15), `Drop` of any supported type (D14), and top-level `val`s of any supported type (D17). The built-in `Range` loses its special case and is the struct `<builtin>::Range(start, end, endInclusive)` of the MIR's type table, so it can be a parameter, result, borrow or top-level value.

**Concrete instances of generic types are supported**: a body of non-generic code holding a `Pair<Int, Bool>` or an `Option<Point>` has a concrete type whose layout is its definition's with the arguments substituted (`TypeTable::instantiated_fields`, `instantiated_variants`; `Type::substitute` already flattens `T?` for a nullable `T`). The flattening question `monomorphization` must answer arises only inside generic bodies, which stay uncompiled. Rejected: waiting for `monomorphization` for generic *types* as well as bodies; the layout code is the same either way, and user enums like `Option<T>` would otherwise stay unusable for two more changes.

### D2. The ABI carries the definitions of reachable types (autonomous)

This settles `codegen-core`'s D24 and the `mir-lowering` follow-up "Layouts and drop glue of imported types whose contents are not visible from the direct imports' ABIs". A file's ABI gains **reachable type definitions**: for every struct and enum that a value reached through the file's exports can hold, other than the file's own exported structs and enums (already recorded as symbols), its definition: defining module and name, kind, type parameters (count and bounds), marker and marker parameters, and its fields (name, `var`, type) or variants (name, payload types), with every type bound as signature types are. *Reachable* is the closure, over definitions, of the nominal types occurring anywhere in an exported symbol: parameter, receiver, return and top-level value types, the fields and payloads of exported structs and enums, type arguments, and the insides of nullables, references, `Heap<T>` and function types; the closure follows the fields and payloads of each reached definition in turn. Private and `package` types are included when reached; the built-in `Range` never is. The records are sorted by module and name.

The interface hash covers the records, after the symbols. So when a type changes, the interface hash changes for its own file and for every file whose exports reach it, and the build cache's existing key (the importer's source and its direct imports' interface hashes) re-checks exactly the importers of those files: a file's artifact still depends only on its content, its direct imports' ABI hashes, the compiler and the target (the deterministic hermetic builds invariant), and editing a body still changes no ABI. An importer lays out every type it can hold from its own declarations and its direct imports' ABIs alone.

Rejected:
- **Per-program layout at link time**: a unit's code would depend on files it does not import, which breaks per-file artifacts and their cache.
- **Resilient layouts queried at run time** (Swift's library evolution): field offsets and sizes through indirection on every access, which is not zero-cost, for a stability Prism does not promise between files of one build.
- **Only a size, an alignment and a "needs drop" bit per referenced type**: enough to move a value along, but an importer that reads a field of an imported struct whose own field has a private type must know where that inner value sits and, from `runtime-strings-and-drops` on, how to drop it, so the whole shape is needed; recording layouts would also make the ABI depend on the target.
- **Symbolic layout and drop-glue functions per type, called through the function table**: indirect calls for every copy and field access.

This does not revisit the type system's rule that members of a type need its module imported (its D9, which rejected embedding shapes to let importers *use* members). The definitions are recorded so the backend can lay values out; the type checker keeps reporting `E4014` for a member of a type whose module is not a direct import (D4).

### D3. Phase 1a reads every declared type; Phase 1b walks the closure (autonomous)

Phase 1a, besides the exported headers it reads today, reads each file's **type definitions**: every struct and enum the file declares, private ones included, with field and payload types lowered to `AbiType` exactly as `FileAbi::extract` binds signature types (the file's own declarations first, then the one direct import exporting the name). A definition is a function of its file's AST and its direct imports' headers, so the definitions of all files can be read in any order, import cycles included. Phase 1b's `FileAbi::extract` then walks the closure of D2 through a lookup over every file's definitions by module and name (the graph holds every file the program reaches). A name bound to no module (`AbiType::Unbound`) has no definition and ends the walk there. `prism abi`, which has no workspace, walks the file's own definitions only.

Records keep field and variant names: an importer's layout dump can show them, and they cost nothing but a rebuild of importers when a private field reached through an export is renamed, which is rare. Rejected: anonymous records (positions only), which make `prism dump-layout` of an importer unreadable for a saving no one needs; computing the closure from the direct imports' *ABIs* in dependency order, which needs a fixpoint for import cycles.

### D4. The type checker records reachable definitions for layout only

`TypeChecker::ingest_file_abi` inserts each reachable definition into the file's type table (`insert_definition`, `insert_marker`), with its types converted as signature types are, and defines no name: a reachable private type stays unnameable, and `E4014` stays decided by `is_unimported`, by module. The type table therefore holds, for every nominal type a body of the file can hold a value of, its definition, which travels into `MirFile::types`; the backend needs nothing else. The checker's member lookups already ask `is_unimported` before reading a definition; the `E4014` fixtures under `type-system/` are the regression check.

### D5. Struct layout: inline, fields by decreasing alignment (autonomous)

A struct is stored inline, its fields in its own storage, never behind a pointer. Its fields are placed in order of decreasing alignment, fields of equal alignment in declaration order, each at the next offset that is a multiple of its alignment; the struct's alignment is the largest of its fields' (1 when it has none), and its size is the end of the last field rounded up to that alignment. Field order is invisible to programs: evaluation, construction and equality follow declaration order. Since every size is a multiple of its alignment, decreasing alignment leaves no padding between fields: `struct S { val a: Bool, val n: Int, val b: Bool }` takes 16 bytes (`n` at 0, `a` at 8, `b` at 9) instead of C's 24.

Rejected: C's declaration order (padding for no benefit while Prism has no FFI; a later `C`-layout attribute can opt into it for FFI types); Rust's unspecified order (a stable, documented rule makes the dump readable and lets tests pin it); optimizing the order further (the greedy rule is already optimal for power-of-two sizes and alignments).

### D6. Enum layout: the smallest tag, payloads after it (autonomous)

An enum with *n* variants stores its variant's index (declaration order, which `Discriminant` already uses) in the smallest unsigned integer holding *n − 1*: `u8` up to 256 variants, then `u16`, then `u32`, at offset 0. Each variant's payload is laid out like a struct's fields (D5), starting right after the tag, so a `Bool` payload sits at offset 1 while a `Double` waits for offset 8. The enum's alignment is the largest of its tag's and its payloads', and its size the largest variant end rounded up to it; all variants share the storage after the tag. A payload-free enum is just its tag (`enum Color { Red, Green }` is one byte); an enum with no variants is uninhabited and has no representation, like `Nothing`. Bytes outside the current variant's payload are undefined and never read (equality compares the tag first, D13).

Rejected: a tag as wide as an `Int` (wasted bytes in every `State` record later); a common payload offset for all variants (simpler, but loses the small-payload packing for no gain); niche filling for enums (Rust stores the tag in unused bit patterns of a payload) as premature; the tag after the payloads.

### D7. Nullables: a tag byte, or the null pointer (autonomous)

`T?` for a non-pointer `T` is a struct of a `u8` tag at offset 0 (0 for `null`, 1 for present, matching MIR's `Discriminant` of a nullable) and the payload at the next multiple of `T`'s alignment: `Int?` is 16 bytes, `Bool?` 2, `Point?` (two `Double`s) 24. A `(&T)?`, `(&mut T)?` or `Heap<T>?` is one pointer, `null` being address 0, since a reference or a `Heap<T>` is never null: `Discriminant` is "the pointer is not 0", `WrapNullable` and the `Payload` projection change nothing, and `null` is 0. Optional borrows, which every `&u.nick` view and `?.` chain over a non-`Copy` value produces, therefore cost one pointer, as in Rust and Swift. `Unit?` is its tag alone. `T??` does not exist (the type system flattens it).

Rejected: one representation for every nullable (doubles every optional borrow); general niche filling (`Bool?` in one byte, `Color?` in the tag's spare values) now, because it adds a case to every tag read and write for a saving that a later change can add without changing any program's behaviour; a separate presence bitmap per struct.

### D8. Sizes of the other types

`Int`, `Double` and pointers are 8 bytes with alignment 8, `Char` 4 and 4, `Bool` 1 and 1 (0 or 1). `Unit` and fieldless structs are 0 bytes with alignment 1, but a stack slot or top-level storage is at least one byte, so every value has an address. `Nothing` and enums without variants have no representation. `Heap<T>` is a non-null pointer (8 and 8); its layout is fixed now so that recursive enums have one (the "Recursive enum with Heap indirection" scenario), while values of it stay `E6001` (D14). `String` and function types have no layout yet: their representations are `runtime-strings-and-drops`' and `closures`' to choose, and `prism dump-layout` says so (D18). Pointer size is the host's; every supported host is 64-bit.

### D9. Aggregates live in memory; scalars in variables (autonomous)

`representation.rs` keeps one machine value for scalars, references and pointer-like nullables (D7), and gives every struct, enum and tag-byte nullable a `Representation::Memory(layout)`. In `lower.rs`, a local of memory representation always lives in a stack slot sized and aligned by its layout; a scalar local stays a Cranelift variable unless it is borrowed (`borrowed_locals`, unchanged). An aggregate *argument* is the memory the caller passed (D10), used in place without a copy; an aggregate *return place* is the memory whose address the function received (D10). `Representation::Range` and `Storage::Range` are removed: a `Range` local is a 24-byte slot, read once per loop by the `for` lowering (which computes the last element before the loop), so loops lose nothing measurable.

Rejected: scalar replacement of aggregates that are never borrowed (`codegen-core`'s D7 suggested generalizing the `Range` trick): it is an optimization with no observable effect, it needs a second representation for every aggregate and every path between them, and enum payloads overlap so they cannot be split into variables. It is a follow-up, if Tier 1 run times call for it; Tier 2 gets it from LLVM.

### D10. Aggregates are passed by address, results through a caller address (autonomous)

In a signature, a parameter of memory representation is one pointer parameter, and a result of memory representation is a pointer parameter placed first, before the arguments, with no return value; scalars stay as `codegen-core`'s D6 says. Both callers and bodies derive this from MIR types alone, as today (`signature`, `body_signature`). The caller:

- passes the address of the operand's place for a `Move` operand: the place is dead after the move (the borrow checker forbids moving a borrowed value, and the argument temporaries of MIR are already distinct locals);
- copies a `Copy` operand into a fresh stack slot and passes that slot, so a callee writing through another argument (`f(p, &mut p)` passing `copy p` directly) can never change the value it received;
- passes the destination's address as the result address when the destination is a whole local that no argument operand is rooted at and that is not borrowed; otherwise it passes a fresh slot and copies it into the destination after the call, so the callee never writes memory it is also reading.

The callee owns its argument memory for the call: it may read it, move out of it and, once drop glue exists, drop it, and never frees it. A plain pointer parameter is used rather than Cranelift's `ArgumentPurpose::StructReturn`, whose register (x8 on arm64) and returned pointer (rax on x86-64) follow the C convention, which Prism-to-Prism calls do not need; an FFI layer can adapt at its boundary.

**Hot reload.** A machine signature names a pointer for every aggregate, never its layout, so a slot of the function table keeps its signature when a type gains a field. The code on both sides of the call embeds sizes and offsets, though, so a layout change must recompile every function that touches the type: the file declaring it changes, and through D2 the interface hash of every file whose exports reach the type changes, so `hot-reload`'s "recompile what the build cache would re-check" already finds every such function. What it must add is the treatment of live values of a changed type (top-level values, later slot-table state), which this change does not decide.

Rejected: Cranelift multi-value returns and per-field arguments (signatures would change with layouts, and enums do not split into fields); `StructArgument` byval parameters (x86-64 only); small aggregates in registers by the C rules (a classification per target for a development tier; a follow-up if calls of small structs show up in profiles); the callee allocating its result (needs an allocator, and the caller then copies it anyway).

### D11. Places, projections and conversions in memory

A place's location is computed from its local's storage and its projections, carrying the current type: `Deref` loads the pointer (from a variable or from memory) and continues at that address with the referent type; `Field(i)` adds the field's offset in the current struct's layout; `Payload` adds the payload offset of a tag-byte nullable, or nothing for a pointer-like one (D7); `Index` stays `runtime-strings-and-drops`'. Reading a place of memory representation yields its address; writing one copies the source's bytes into it (D12). A scalar field or payload is loaded or stored at its offset with its machine type.

`Discriminant` loads the enum's tag and zero-extends it to `i64`, or loads a nullable's tag byte, or compares a pointer-like nullable with 0. `Aggregate` writes each operand at its field's offset (for a variant, the tag and then the variant's payload offsets), after evaluating every operand (D12 for overlap). `WrapNullable` writes tag 1 and the payload, or is the identity for a pointer-like nullable. `Constant::Null` writes tag 0 (the payload bytes stay undefined) or is a null pointer. `Subtype` is supported where both sides have one representation: `&mut T` to `&T` at any depth the type system allows, the same step inside `T?`, and `Nothing?` to `T?` (writing `null`); every other `Subtype` the type system can produce involves function types or `Heap<T>`, whose values are already rejected (D19).

### D12. Copy and move are both bitwise copies

At run time a move and a copy of the same value are the same bytes copied: `Copy` only decides, at check time, whether the source may be used again, and no value has drop glue yet (D14). A whole aggregate is copied with Cranelift's `emit_small_memory_copy` (inline loads and stores up to its threshold, `memcpy` above, which `cranelift-jit` resolves from the host's C library), and with `emit_small_memmove` when source and destination may overlap: when they share their root local, or either is reached through a dereference. An `Aggregate` whose operands include a memory-represented place that may overlap the destination (`s = S(s.b, s.a)` for struct-typed fields) first copies those operands into temporary slots. A moved-from place is never touched afterwards, so nothing is zeroed or flagged at run time.

### D13. Structural equality, expanded inline (autonomous)

`==` over a struct compares its fields in declaration order with `==` of their types and is true when all are equal; over an enum, it compares the tags and, when they are equal, the current variant's payloads field by field; over a nullable, two `null`s are equal, `null` and a present value are not, and two present values compare their payloads. `Double` fields compare as `Double`s do (`NaN != NaN`, `0.0 == -0.0`), so a struct holding `NaN` is not equal to itself, as in Rust's derived equality. Operands of reference type (the `Operator` borrows MIR builds for types that are not `Copy`, and optional borrows `(&T)?`) compare their referents, never their addresses, matching the type checker, which types operators after peeling references. `!=` is the negation. Padding and inactive payload bytes are never compared.

Each comparison is expanded inline at its operator, recursively through nested aggregates, with a switch over the tag for enums; types reaching themselves need `Heap<T>` (D20), whose values are rejected, so expansion terminates. Rejected: comparing bytes (`memcmp`), which reads padding and inactive payloads and gets `NaN` and `-0.0` wrong; per-type equality functions now. `runtime-strings-and-drops` needs per-type glue functions for recursive types and strings anyway, and may move equality into them when it designs that mechanism (where glue lives, how it is named and linked); inline expansion keeps this change free of that decision.

### D14. Drops of values that own nothing compile to nothing (autonomous)

A type *owns storage* when it is a `String`, a `Heap<T>` or a function type, or a struct, enum or nullable with a field or payload that owns storage. Every type the backend now accepts owns nothing, so its drop glue is empty: a `Drop` terminator is a jump to its target, and drop flags, being ordinary `Bool` locals, are set, cleared and tested as MIR says, which Cranelift's dead-code elimination mostly removes. This lets non-`Copy` aggregates of scalars run, with their moves and drops checked exactly as later. An aggregate that owns storage is rejected wherever its type appears, named by what it holds: `struct User { val name: String }` reports "strings", `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` reports "'Heap' values", both pointing at `runtime-strings-and-drops`, as a reference is already named by its referent. `Construct::Drops` disappears: no drop of a supported type does anything.

Rejected: rejecting every non-`Copy` aggregate until drop glue exists (most data types of real programs are not `Copy`, and their drops are what `runtime-strings-and-drops` will add code to, not where it will change the MIR); a "structs holding strings" construct of its own (the user needs to know what is missing, which is strings).

### D15. Cloning a value that owns nothing is a copy

The built-in `clone()` of any supported type copies its receiver's referent into the destination (D12), field by field being the same bytes when nothing is owned. This makes `Clone` structs of scalars and `Copy` aggregates clonable now; `clone()` of owning values is `runtime-strings-and-drops`'.

### D16. `print` and `println` stay scalar (autonomous)

The output built-ins keep accepting `Int`, `Double`, `Bool`, `Char` and `String` (and references to them); a struct, enum or nullable argument stays `E4004` with the existing help. Printing a struct or an enum needs a text form (`Point(x=1, y=2)` like Kotlin's data classes, or a user `toString`), which belongs with a `Display`-like design once traits or overloading exist; printing `null` for a nullable alone would be a third special case. Programs print fields, or `n ?: 0`. Rejected: Kotlin's data-class `toString` now (commits to a text form for every struct before the language can let a type choose its own), printing nullables as `null` (left as the `codegen-core` follow-up).

### D17. Top-level values of aggregate types

A top-level `val` of a memory-represented type gets `#storage` sized and aligned by its layout (at least one byte), and its `#init` body returns through the result address of D10: the accessor passes the storage's address as `#init`'s result address, so the value is built in place. The accessor protocol of `codegen-core`'s D18 is otherwise unchanged; reads copy out of, or borrow, the storage as any `(*_t)` place.

### D18. `prism dump-layout` (autonomous)

`prism dump-layout <file.prism>` builds the file's elaborated MIR as `prism dump-mir --elaborated` does (against its direct imports' ABIs), and prints one entry per type: first the structs and enums the file declares, in declaration order, then every other struct, enum and nullable type of a local of one of its compiled bodies (imported and reachable types, instances of generic types, nullables), each once, in the order of their rendered names. Each entry gives the type, its size and alignment, then one line per field with its offset; an enum its tag's width and offset, then each variant with its payload offsets; a nullable its tag or "null pointer" and its payload offset:

```
struct main::Point : Copy (size 16, align 8)
  x: Double at 0
  y: Double at 8
enum main::Expr (size 24, align 8, tag u8 at 0)
  Literal(Int at 8)
  Add(Heap<Expr> at 8, Heap<Expr> at 16)
struct main::User (no layout yet: strings)
struct main::Box<T> (generic: laid out per instance)
nullable (&Point)? (size 8, align 8, null pointer)
nullable Int? (size 16, align 8, tag u8 at 0)
  payload Int at 8
```

A type without a layout yet is listed with the construct it waits for rather than failing the dump, because layouts are what the dump is for; a file with type or borrow errors reports them and prints nothing, like `dump-clif`. Fixtures compare the dump with a `layout.snap` beside the case (the fixture runner gains that snapshot kind, like `mir.snap`), so the layout rules of D5 to D8 are pinned by tests, which `prism dump-clif` (no snapshots, codegen-core's D23) could not do. Rejected: layouts in `dump-clif`'s output (mixed with IR, and unstable across Cranelift upgrades); a `--layouts` flag on `dump-mir` (MIR does not know layouts).

### D19. What `E6001` still rejects

`supported.rs`'s `classify` gains the type table, so it can look into a struct's fields and an enum's payloads (substituted for instances), and reports the first construct a type needs that the backend lacks, in field order. The table of `codegen-core`'s D9 loses the rows "structs", "enums", "nullable values", "'Range' values outside local variables" and "drops"; "strings", "'Heap' values", "function values and closures", "calls of generic functions", "composable functions" and "values of unknown type" stay. "conversions from `A` to `B`" stays as a safety net for a `Subtype` step D11 does not lower, with no planned change named (the type system produces none over supported types). A struct or enum whose definition is missing from the type table, which D4 makes impossible, is an internal error rather than a diagnostic.

### D20. Infinitely sized types are rejected with `E4023` (autonomous)

A layout exists only for types of finite size, so the type checker rejects a struct or enum declaration that contains itself by value: through its fields or payloads, through nullables and through other structs and enums (instances substituted), but not through `Heap<T>`, references or function types. Each of the file's own declarations is walked over its fields with a stack of the types being expanded, keyed by type and type arguments; reaching a type already on the stack is a cycle, and an expansion more than 64 types deep (only a generic type growing its own arguments, `struct W<T> { val w: W<Pair<T, T>>? }`, gets there) counts as one. The error is `E4023` (`InfinitelySizedType`), the next free `E4xxx` code, once per declaration on a cycle, at its name, with a note naming the path (`A` contains `B?`, which contains `A`) and help suggesting a `Heap<...>` field. A cycle through other modules is visible because their definitions are in the type table (D4), and is reported in each file declaring one of its types; a cycle entirely inside other files is reported when those files are checked. This is the `ROADMAP.md` item "Reject recursive struct and enum layouts that lack `Heap<T>` indirection".

*Implementation notes.* A declaration is on a cycle exactly when the walk from it, applied to its own type parameters, reaches a type with its own identity (any arguments); a cycle the walk meets that does not lead back to the declaration (`struct C { val a: A }` beside `A` and `B` above) is not expanded twice and reports nothing at `C`, since `A` and `B` are reported. The depth limit is therefore only a termination guard, together with a size limit (a type of more than 256 nodes is not expanded): `W<T>` above reaches `W<Pair<T, T>>` at once and is reported there, while a struct merely holding a `W<Int>` would otherwise expand types that double at each step. Struct and enum declarations record the span of their name (`name_span` in the AST) for the diagnostic. `prism check` stops at the first file with errors, so of the two reports of a cycle across modules it shows the first file's; the other file reports its own when checked (the scenario says so).

Rejected: rejecting it in the backend (`E6xxx`), since an infinitely sized type is wrong in every tier and `prism check` should say so; a parse-time check (the parser cannot see imported types); Rust's recursion limit as a user-visible setting.

### D21. Deferred, with the decision path

- **Live values across a layout change** (`hot-reload`): D10 guarantees slots keep their signatures and D2 finds every function to recompile; what happens to a top-level value or slot-table state whose type changed (reset, migrate, refuse the reload) is `hot-reload`'s.
- **Equality and drop glue as functions** (`runtime-strings-and-drops`): D13.
- **Layout optimizations** (follow-ups): niche filling beyond null pointers (D7), scalar replacement (D9), small aggregates in registers (D10), and a C-layout attribute with FFI (D5).
- **Printing aggregates** (follow-up, with a `Display` story): D16.

### D22. No new capability (autonomous)

Layouts, the calling convention and `prism dump-layout` extend `code-generation` (the backend's contract, which `codegen-core`'s D25 said `codegen-aggregates` would extend); the runtime meaning of aggregate operations extends `execution`; the ABI records extend `compilation-and-abi`; `E4023` extends `type-system` and `diagnostics`. Rejected: a `layout` capability of its own, which would split the backend's representation rules across two specs.

## Risks / Trade-offs

- [Every aggregate lives in memory, so Tier 1 code over small structs does loads and stores Cranelift does not promote] → acceptable for a development tier; scalar replacement is a contained follow-up (D9), and Tier 2 optimizes.
- [ABIs grow with the definitions of every reachable type, duplicated across the ABIs that reach them] → definitions are small and hashed once per ABI; the closure stops at types already recorded and at the file's own exported types.
- [A private field rename rebuilds the importers of every file whose exports reach that type] → rare, and the price of exact invalidation; D3 records it.
- [Inline equality expands large nested types at every `==`] → bounded by the type's size, and moved into glue functions when `runtime-strings-and-drops` designs them (D13).
- [Recording definitions in the type table could let a member lookup bypass `E4014`] → every member lookup goes through `is_unimported` first (D4), and the existing `E4014` fixtures must stay green.
- [The 64-type depth limit of `E4023` could reject a legitimately deep finite type] → only a type nesting 64 distinct instances by value, which no program writes by hand; raising the limit changes nothing else.
- [`memcpy` above the inline-copy threshold relies on the host C library through `cranelift-jit`'s symbol lookup] → a codegen test copies a large struct on the implementing host; the runtime can provide its own copy function if a platform lacks it.

## Migration Plan

No external users. The fixtures that pinned `E6001` for structs and nullables (`code-generation/a-struct-is-rejected`, `code-generation/unsupported-constructs-in-several-forms--nullable`) become cases of the updated scenarios, and the `Range`-specific unit tests of `supported.rs` and `representation.rs` change with D9. ABI hashes change for every file whose exports reach a type (new records), which only invalidates build-cache entries; the compiler's identity is in every key anyway. Rollback is a revert.

## Open Questions

- The exact text of `prism dump-layout` can be tuned during implementation, provided it shows every size, alignment, offset and tag of D18.
- Whether `E4023`'s note lists the whole cycle or only its first step can be settled when writing the diagnostic.
