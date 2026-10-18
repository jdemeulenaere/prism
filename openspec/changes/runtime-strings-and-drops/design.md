# Design: runtime-strings-and-drops

## Context

See `proposal.md` for motivation. This is step 16 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17; the orchestrator gave the scope (owned `String`, drop glue, `clone()`, `Heap<T>` construction and unboxing, converting scalars to text, leak and double-free verification, panics) and allowed a split. Decisions marked **(autonomous)** were made by the proposer and are logged in `docs/decisions-for-review.md`.

This change builds on `codegen-aggregates`, which was archived (b5b1499, 5e234da) before this change was implemented; see "Reconciliation with the archived `codegen-aggregates`" below for the delta-spec adjustments that followed. Current state, after `codegen-aggregates`:

- **Values have layouts and live in memory.** `crates/prism-codegen/src/layout.rs` lays out structs, enums and nullables (`LayoutTable::layout`), and fails for `String` and function types with the construct they wait for. Every aggregate local is a stack slot; aggregates are passed by address and returned through a caller address (its D10); moves and copies are bitwise copies (D12); `==` is expanded inline per operator (D13); `clone()` of a value that owns nothing is a copy (D15). A type *owns storage* when it is a `String`, a `Heap<T>` or a function type, or holds one through fields, payloads or nullables; every such type is still rejected with `E6001` (`supported.rs`, `classify`), and the `Drop` terminators of every other type compile to a jump (D14).
- **The MIR already decides every drop.** Lowering places a `Drop` where each scope ends, for each overwritten value of an assignment (`*r = v` drops the old referent) and for each temporary; elaboration (`crates/prism-borrowck/src/elaborate.rs`) removes the drops of moved values and guards the others with drop flags, ordinary `Bool` locals (the `mir` spec). `prism_mir::needs_drop` is "not `Copy`", so non-`Copy` aggregates of scalars have drops, which do nothing.
- **Strings exist up to the MIR.** `Constant::String(text)` holds a literal's decoded text; `a + b`, `==` and `!=` over strings are `Rvalue::Binary` over two `Operator` borrows (`&String`); `s += t` is lowered as `tmp = &s + &t; drop s; s = move tmp` (the `mir` spec, "String compound assignment drops the old value"); `print(s)` passes a `&String`. `s[i]` is a place with a `Projection::Index` whose type is `Char` (`projection_type`), so `&s[i]` borrows `s` itself, and `codegen-core` rejects it (`ROADMAP.md`: "`String` indexing and `Char`"). `String` has no members: `s.length` and `n.toString()` are `E4006`. `s[i] = c` is already `E4003`, with the help "use 'var' to declare a mutable binding or field", which is wrong for it.
- **The runtime has no allocator.** `crates/prism-runtime` exports the output, panic and stack functions of `codegen-core` through `RUNTIME_FUNCTIONS` (D17 of `codegen-core`); its state is process-wide behind a `Mutex`, bracketed by `start` and `finish`. A runtime error flushes, reports and exits with status 101 without unwinding.
- **Top-level values** have `#storage`, a `#state` byte and a lazy accessor `#get` (D18 of `codegen-core`, D17 of `codegen-aggregates`); nothing ever drops them.

## Reconciliation with the archived `codegen-aggregates`

This change was proposed while `codegen-aggregates` was still active, so its deltas were written against that change's delta specs. When implementation started, `codegen-aggregates` had been archived and its requirements were in `openspec/specs/`. Every MODIFIED block of this change was compared with the main specs: the requirement texts and scenarios this change keeps are identical to the archived ones, so no wording drifted. One block could not be applied as written: the MODIFIED "Drops of Values That Own Nothing Do Nothing" in `code-generation` replaced the scenario "A struct holding a string" (which pinned strings as unsupported) with "A struct holding a heap value", and a MODIFIED requirement may not drop a scenario of the main spec (`openspec validate --strict` rejects it, since archive would lose it). The scenario's name also belongs, in this change, to the layout scenario of the new "Strings Are a Pointer, Two Lengths and a Capacity". So the delta now **removes** "Drops of Values That Own Nothing Do Nothing" and moves its parts where they belong: its rule for values that own nothing, and its scenario "A non-Copy struct is moved and dropped", into "Drops Run Drop Glue" (a type owning no storage has no glue, and its drops run no code); its rejection of aggregates holding a `Heap<T>` or a function value, with the scenario "A struct holding a heap value", into "Unsupported Constructs Are Rejected Before Code Generation". No behaviour changes; the fixture `code-generation/a-struct-holding-a-string` now pins the layout scenario, as D18 already planned.

## Goals / Non-Goals

**Goals:**
- Run programs over strings and over structs, enums and nullables holding strings, freeing every allocation exactly once, where the elaborated MIR drops its owner.
- Fix the representation of `String`, the shape and placement of drop and clone glue, and the allocator interface, so that `heap-boxes`, `closures` (whose environments are heap-allocated) and `compose-runtime` reuse them unchanged.
- Make "no allocation outlives its owner, none is freed twice" a property every executable fixture checks, from now on and through Phase 4.
- Let programs turn numbers into text (`n.toString()`), which UI code needs before string templates exist.

**Non-Goals:**
- `Heap<T>` construction, unboxing, writes and covariance (`heap-boxes`, D19); string templates (`string-templates`, D20).
- Optimizing strings: in-place append (D15), a small-string representation, a null-pointer niche for `String?` (D2), interning.
- A string library beyond `length`, indexing, `+`, equality and `toString()` of scalars.
- Destructors running user code (a `Drop` trait), which waits for traits.

## Decisions

### D1. Three changes: strings and drops, then heap boxes, then templates (autonomous)

The orchestrator's scope holds three designs that one can disagree with independently, each with front-end and back-end work:

- **Owned strings, drop and clone glue, the allocator and allocation checking** (this change): the representation every later value with owned memory follows, and the runtime machinery to free it.
- **`Heap<T>`** (`heap-boxes`, item 17): a constructor syntax, reading and writing through a box, moving out, the covariance question `ROADMAP.md` records, and the typed-tree and MIR changes they need. Its drop and clone glue are one more case of this change's mechanism (D9, D10), but no `Heap<T>` value can exist before its construction does, so testing them belongs there.
- **String templates** (`string-templates`, item 18): lexing `$name` and `${expression}` inside literals, a new expression node, formatting, typing each part and lowering the whole to the runtime, all on top of this change's strings and `toString()`.

Each is a reviewable change of the usual size; in one change, the `Heap<T>` language decisions would wait on the runtime work, or the reverse. Rejected: one change for everything (the orchestrator's first framing, too large for one review); templates inside this change (all of the front end would be new work in a backend change, and UI code needs templates only by Phase 4); `heap-boxes` after `monomorphization` (recursive data types are basic, and `compose-runtime`'s node trees may want them). The two new items go right after this one, before `monomorphization`; the later items are renumbered in `ROADMAP.md`, and `docs/decisions-for-review.md` logs the reorder (P5).

### D2. A string is a pointer, two lengths and a capacity (autonomous)

A `String` is 32 bytes, aligned like a pointer, laid out like a struct of four words:

| Offset | Field | Meaning |
|---|---|---|
| 0 | pointer | the first byte of the UTF-8 text, never null |
| 8 | byte length | the text's length in bytes |
| 16 | character count | the number of Unicode scalar values in the text |
| 24 | capacity | the size of the owned buffer in bytes, or 0 when the value owns none |

Invariants: a capacity of 0 means the value owns no memory, its text being static data (a literal, a runtime constant such as `"true"`) or empty (pointing at a static empty text, never null); a positive capacity means the value alone owns a buffer of exactly that many bytes, allocated through `prism_rt_allocate` with alignment 1, holding its text at the start. The character count equals the byte length exactly when the text is ASCII, which the runtime uses as a fast path (D4). The runtime defines it as `#[repr(C)] struct PrismString` in `crates/prism-runtime/src/string.rs`; the backend's layout table gives `String` this fixed layout, so `String?` is 40 bytes (tag byte, payload at 8) by `codegen-aggregates`' nullable rule, and a struct holding a `String` places it like any 32-byte, 8-aligned field.

The character count is what makes `length` O(1) under character semantics (D4): every string is built either by the compiler (literals, counted at compile time) or by a runtime function that knows the counts of its inputs, so keeping it costs one addition per operation. Rejected: Rust's three words without the count (`length` would scan the text on every call, and indexing could not tell ASCII text in O(1)); a capacity stored in the buffer's header with a flag bit for static text (24 bytes, but every access to the capacity tests a tag); a small-string representation storing up to 31 bytes inline (Swift, C++'s libc++), which puts a branch on every pointer read and is an optimization that a later change can make without changing any program's behaviour; reference-counted or copy-on-write sharing, which would be hidden reference counting, forbidden by the zero-cost memory safety invariant. A null-pointer niche for `String?` (the pointer is never null) would save 8 bytes per optional string; `codegen-aggregates` left every niche but the null pointer of references and boxes out (its D7), and this one is added with the others later.

### D3. Literals are static data; ownership is a property of the value (autonomous)

The `ownership` spec says each string literal evaluates to a fresh owned `String`: a program may move, store or drop it independently of every other evaluation. That is a statement about values, not about where their bytes are. A literal's bytes are emitted once per code unit, as the read-only data object `UnitContext::text` already defines and deduplicates, and evaluating the literal writes the four words (pointer to that data, byte length, character count computed at compile time, capacity 0) into its destination, without a call. Dropping it frees nothing (capacity 0), and cloning it copies the four words, since text no value owns needs no new buffer to be independent: nothing can change or free it. No operation of this change writes into a string's buffer, so no copy-on-write rule is needed yet; an in-place operation (D15) must first copy text it does not own, which the capacity tells it.

This makes literals as cheap as in C and Rust while keeping value semantics. Rejected: allocating a buffer per evaluation (the literal semantics taken literally, an allocation per `println("...")`); a separate `StaticString` type (Rust's `&'static str`), which Kotlin-style code would have to convert everywhere. The architecture's "A `String` clone allocates a new buffer" becomes "a new buffer when it owns one" in the `execution` spec.

Literal data outlives every value pointing at it only as long as code units are never freed. `hot-reload` defines a reloaded file's unit into a new JIT module; it must keep the old modules mapped while the program runs, because a top-level value or slot-table state may hold a literal of old code (recorded in `ROADMAP.md` for `hot-reload`).

### D4. `length` and indexing count characters (autonomous)

`ROADMAP.md` asks this change to decide whether strings index by byte or by character. Prism's `Char` is a Unicode scalar value (`codegen-core`), and `s[i]` already has type `Char`, so byte indexing would give a `Char` that is not a character, or need a byte type that does not exist. So `s.length` is the number of characters and `s[i]` the character at character index `i`, counted from 0, as a Kotlin programmer expects of `length` and `[]`, with scalar values in place of Kotlin's UTF-16 code units (`"🚀".length` is 1, not 2). `length` reads the stored count (D2); `s[i]` is O(1) on ASCII text (character count equal to byte length) and a scan from the start otherwise. An index outside `0..<length`, negative ones included, is the runtime error "string index 3 out of range for length 3" at the indexing expression (D17).

Rejected: byte semantics (Rust's `len()`; meaningless with a scalar `Char` and surprising for Kotlin code); UTF-16 code units (Kotlin's and Java's; would need a UTF-16 representation or a second count, and splits emoji); grapheme clusters (Swift's `count`; needs Unicode tables in the runtime and is O(n) for every count); no integer indexing at all, iteration only (Swift), since the language already has `s[i]` and programs written now expect it; a stored index of character offsets for non-ASCII text (memory per string for a rare operation).

### D5. A string's character is a value, not a place (autonomous)

A `Char` decoded from UTF-8 has no address, so `s[i]` cannot be a place that a borrow points at. The type checker types `s[i]` on a `String` as a call of a new built-in, `StringCharacter`, with a shared receiver borrow of the string and the index as its argument, like `clone()` (`infer_clone_call`). Consequences:

- `&s[i]`, and `s[i]` passed to a `&Char` parameter, borrow a temporary holding the character, by the existing rules for non-places; `val r = &s[0]; s = "cd"` is accepted, since `r` does not borrow `s`. This is **BREAKING** for programs that relied on `&s[i]` borrowing `s`, which nothing could run.
- `s[i] = c` stays `E4003`, with help saying that the characters of a string cannot be assigned, because strings are immutable.
- The MIR no longer produces `Projection::Index`: `projection_type` loses its `String` case and the MIR validator rejects any index projection. `Projection::Index` and the typed tree's `ExpressionKind::Index` stay defined, unused, for collections whose elements are stored (`List<T>` will index places).
- `TypeChecker::is_place` returns false for an index expression: only strings are indexable today, and indexing anything else is already `E4002`.

Rejected: keeping the place and having the backend compute a read with a runtime call while rejecting borrows of it (a place that cannot be borrowed is a special case in every analysis); keeping the place and making the MIR builder copy the character into a temporary when borrowed (the borrow checker would still see a borrow of `s` for code that never borrows it).

### D6. Built-in `length` and `toString()` (autonomous)

- **`s.length`** is a built-in member of `String`, read without parentheses as in Kotlin, through any `&`, `&mut` and `Heap<T>` layers (as field access is), and through `?.` (`s?.length: Int?`). It is a call of the new built-in `StringLength` with a shared receiver borrow. It counts as a field of `String` for member precedence: `s.length()` is "type 'Int' cannot be called as a function" (`E4004`), and an extension `fun String.length()` could never be called, so it is `E4010` like an extension named like a field.
- **`x.toString()`** is a built-in member function of `Int`, `Double`, `Bool` and `Char`, through `&` and `&mut`, and through `?.`, giving the text `print(x)` writes (the `execution` spec's "Double Text Form" for `Double`). It is a call of the new built-in `ToString` with the receiver copied out of its references, taking no arguments (`E4004` otherwise). It wins over extensions, like `clone()` on a `Clone` type, so an extension named `toString` on those four types is `E4010`. On every other type `toString` is an ordinary name: a user may declare `fun Point.toString(): String`, which `string-templates` can call to display a struct, so printing aggregates later needs no new mechanism.

`StringLength`, `StringCharacter` and `ToString` join `BuiltinFunction` in `crates/prism-typed-tree/src/tree.rs`, named `length`, `get` and `toString` in dumps (`get` being the name Kotlin gives the `[]` operator), and `has_receiver` is true for them as for `clone`. Their results own no borrow, so elision rule 2 ties nothing to the receiver in practice.

Rejected: `String.toString()` (a clone under another name, redundant until templates call it generically); `toString()` on every type with a generated text form (Kotlin's data classes; `codegen-aggregates` D16 rejected committing to a text form for aggregates); a library `fun Int.toString()` (no prelude exists, and the runtime formats `Double`); `length` as a function `s.length()` (Rust's `len()`), which Kotlin code does not write; `size` (Kotlin uses `length` for strings).

### D7. String operations are runtime functions

Generated code inlines only what needs no loop or allocation: building a literal (four stores) and `length` (one load at offset 16). Everything else is a direct call of a runtime function in `crates/prism-runtime/src/string.rs`, taking strings by address (`*const PrismString`) and writing results to a destination address, as aggregates are returned (`codegen-aggregates` D10):

| Function | Use |
|---|---|
| `prism_rt_string_concatenate(destination, left, right)` | `a + b`; a result that would be empty is the static empty string, and an empty operand makes the result a clone of the other |
| `prism_rt_string_equals(left, right) -> i8` | `==`, `!=`, `when` over strings, and string fields in structural equality: byte lengths, then bytes |
| `prism_rt_string_character(string, index, file, file_length, line, column) -> i32` | `s[i]`, with the bounds check and its runtime error at the given location |
| `prism_rt_string_clone(destination, source)` | `clone()` of a `String` (D10) |
| `prism_rt_string_drop(string)` | dropping a `String` (D9) |
| `prism_rt_print_string(string)` | `print` and `println` of a `String` |
| `prism_rt_int_to_string`, `prism_rt_double_to_string`, `prism_rt_bool_to_string`, `prism_rt_char_to_string` `(destination, value)` | `toString()`; `Bool` gives static text and allocates nothing |

Every string the runtime builds is exactly sized (capacity equal to byte length), since nothing appends to it later in this change. The runtime decodes nothing it does not need to: concatenation adds both counts, equality compares bytes (UTF-8 is canonical for a sequence of scalar values, so equal texts have equal bytes). Rejected: generating string loops in Cranelift (more IR per use and per backend tier, for code that the runtime writes once in Rust); C's `strcmp`-style null termination (strings may contain `\0`).

### D8. The allocator: the runtime over Rust's global allocator, with sized frees (autonomous)

`crates/prism-runtime/src/allocation.rs` exports `prism_rt_allocate(size, alignment) -> pointer` and `prism_rt_free(pointer, size, alignment)`, implemented with `std::alloc::alloc` and `dealloc` over a `Layout`, that is, Rust's global allocator, which is the platform's `malloc` today. Every allocation of a program goes through them: the runtime's string functions call them, and later changes' generated code (the box of `heap-boxes`, closure environments) will call them directly from glue. A failed allocation, or a size that does not fit a `Layout` (more than `isize::MAX`), is the runtime error "out of memory" (D17). Freeing passes the size and alignment back, as Rust's allocator interface does, so the allocator can be replaced by a size-class allocator (mimalloc, a per-thread cache for compose) without changing any caller, and so the allocation check (D13) can verify them.

This is a malloc-like allocator with deterministic free, as the zero-cost memory safety invariant requires: each allocation is freed by the drop of its single owner. Rejected: an arena or bump allocator freed at exit (a long-running UI would grow without bound, and nothing would check that drops run); calling the C library's `malloc`/`free` from generated code (ties code to a platform symbol, gives up sized frees and the check); Prism's own allocator now (Rust's global allocator is fast enough for a development tier and is the hook to swap one in); counting allocations in normal runs (an atomic per allocation for nothing; tracking only happens under `--check-allocations`).

### D9. Drop glue is generated per code unit, per type (autonomous)

What a drop does depends only on the dropped type:

- a type that owns no storage has no glue: its `Drop` stays a jump (`codegen-aggregates` D14);
- a `String` is dropped by `prism_rt_string_drop`, which frees the buffer when the capacity is positive;
- a struct, an enum or a nullable that owns storage is dropped by a generated **drop glue function** taking the address of the value: a struct's glue drops each owning field in declaration order (Rust's order), an enum's glue switches on the tag and drops the owning payloads of that variant in order, and a nullable's glue tests the tag and drops the payload. A part is dropped by calling its own glue (the runtime function for a `String`), so glue is a set of small functions, one per type.

`codegen-core`'s lowering of a `Drop { place, target }` becomes: the address of `place` (a stack slot, an argument's memory, or a dereference), a direct call of its type's glue, and a jump to `target`. Drop flags need nothing new: they are `Bool` locals that the elaborated MIR sets, clears and tests with `SwitchInt` before the `Drop`. An assignment's drop of the old value, `*r = v`'s drop of the old referent, and the drops of temporaries at the end of their statements are ordinary `Drop` terminators already, so they run glue the same way.

**Where glue lives.** Each code unit generates the glue of every type its bodies drop, and of the parts those reach, in a `GlueTable` of its `UnitContext` (new `crates/prism-codegen/src/glue.rs`) that memoizes a function per type and lowers the queued functions after the unit's bodies. Glue is declared with local linkage under the name `<unit module>#drop(<type>)`, for instance `app/main#drop(lib/user::User)` (unique within the JIT module that a load's units share), and called directly, never through the function table. A unit's code then depends only on its own MIR, whose type table holds the definition of every type it can hold (`codegen-aggregates` D2), and instances of generic types (`Pair<String, Int>`) get glue where they are used, without asking where instances live (`monomorphization`'s question). The price is duplicate glue across units, a few instructions per type; Tier 2 can merge identical glue at link time, as C++ and Rust merge their `linkonce` functions. Hot reload stays sound: a unit is recompiled exactly when a type it handles changes layout (`codegen-aggregates` D10), and its new code calls its new glue, while frames of old code keep calling the old glue matching the old layout.

Rejected: glue expanded inline at each drop (code grows with the type's size at every drop, and the recursive types of `heap-boxes` cannot be expanded); glue in the defining file's unit, called through the function table (an indirect call per drop, and no file defines the glue of `Pair<String, Int>`); one runtime function interpreting a layout description at run time (slow, and the description would be one more representation to keep in sync); unnamed (anonymous) Cranelift functions (`prism dump-clif` could not name them).

Glue does not check the stack: nothing recursive can be dropped yet. `heap-boxes` makes glue recursive (dropping a `Heap<Expr>` drops an `Expr`), and must add the stack check of `codegen-core` D27, or drop long chains iteratively.

### D10. Clone glue follows drop glue

The built-in `clone()` already writes a fresh value of the receiver's referent type to its destination. A type that owns no storage is copied (`codegen-aggregates` D15). A `String` is cloned by `prism_rt_string_clone`: a value owning no buffer is copied (D3), one owning a buffer gets a new, exactly sized buffer with the same text. A struct, enum or nullable that owns storage is cloned by a generated **clone glue function** `<unit module>#clone(<type>)` taking the source and destination addresses: each field in declaration order, copied when it owns nothing and otherwise cloned by its own glue; an enum writes the tag and clones the payloads of that variant; a nullable writes `null` or clones its payload. Glue is generated only for `Clone` types, since the type checker rejects every other `clone()`, and lives in the unit's `GlueTable` like drop glue.

Cloning evaluates no user code (custom clone bodies wait for traits, the `ownership` spec), and a clone shares no owned memory with its source, which the allocation check (D13) makes observable: dropping both frees each buffer once.

### D11. Equality stays inline; strings compare in the runtime

`codegen-aggregates` D13 expands `==` inline and left moving it into glue functions to this change. Without `Heap<T>` no type reaches itself, so inline expansion still terminates: a `String` field or payload is compared by a call of `prism_rt_string_equals`, and `when` over a string subject compares with `==` per pattern, as the MIR already lowers it. `heap-boxes` must turn equality of types that reach themselves through boxes into `<unit module>#equals(<type>)` glue functions of the same kind as D9's (D19). Rejected: equality glue now for every type (a change of `codegen-aggregates`' code for no behaviour this change needs).

### D12. Which values are dropped, and when; top-level values never are (autonomous)

The MIR decides every drop (Context), so the runtime meaning follows: a value's owned memory is freed where its owner's scope ends, where an assignment or a write through a reference overwrites it, at the end of the statement for a temporary, or by whatever code it was moved into. Each allocation is freed exactly once, which is what drop elaboration guarantees and what the allocation check (D13) verifies in every executable fixture.

**Top-level values are never dropped.** A top-level `val` lives until the program ends, like a Rust `static`: its accessor builds it once, borrows of it are valid everywhere (`borrowck-elision`), and running drop glue at exit would cost time for memory the process gives back anyway, and, once destructors can run user code, would bring C++'s static destruction order problems. The allocation check accounts for their memory instead (D13). Rejected: dropping initialized top-level values at exit in reverse order of initialization (C++'s static destructors).

### D13. Allocation checking with `prism run --check-allocations` (autonomous)

`prism run --check-allocations <file>` makes the runtime track every allocation in a map from address to size and alignment, behind its state lock; without the option, `prism_rt_allocate` and `prism_rt_free` test one relaxed atomic flag and track nothing.

- A free of an address the map does not hold, or with another size or alignment, flushes the output, writes `allocation check: freed memory that is not allocated` to stderr and exits with status 102. A double free and a free of static text are both caught.
- When `main` returns, `finish` first releases the memory of initialized top-level values (D12): each accessor whose value's type owns storage calls `prism_rt_register_top_level_value(storage, drop glue)` once, after its initializer returns, and in check mode `finish` runs those glues in reverse order of registration; this runs no user code. Then, if the map is not empty, it writes `allocation check: allocations still live at exit: <n> (<b> bytes)` after flushing the output, and the exit status is 102.
- A runtime error ends the process before any check, since no drop runs on a panic (D14).
- Status 102 is distinct from 0 (`main` returned), 1 (did not compile) and 101 (runtime error), so a test harness tells a compiler bug from a program error without reading stderr.

The fixture runner (`crates/prism-cli/tests/fixture_tests.rs`) runs every executable case with `--check-allocations`, so every program fixture, now and later, asserts that it leaks nothing and frees nothing twice; this is also Phase 4's "no allocation outlives its owner" verification, which `reactive-app-verification` can reuse for long-running compose programs. Codegen tests read `prism_runtime::allocation_statistics()` (total allocations, live allocations and live bytes, maintained in check mode) to assert that literals and `Bool` text allocate nothing.

Rejected: an environment variable (`codegen-core` D14 preferred options for `--timings`, and a variable leaks into child processes); counting only, without the map (a double free and a leak cancel out); checking at every drop (per-drop overhead for the same result); always-on checking (a map operation per allocation in normal runs); exit status 101 (a panic and a compiler bug would look alike); not registering top-level values and counting their allocations as leaks (every program with a computed top-level string would fail the check).

### D14. Panics do not unwind, so values are not dropped on a panic

Confirmed from `codegen-core` D12: a runtime error flushes the output, reports and exits the process with status 101, with no landing pads or unwind tables, and no drop runs; the operating system reclaims the memory. The `execution` spec already says "no drop MUST run". `--check-allocations` therefore does not check a program that panics (D13). If `prism test` later needs recoverable failures in process, unwinding would add cleanup edges to the MIR's calls, which drop elaboration and glue already support in shape (a `Drop` per live value); that is the `ROADMAP.md` follow-up "Panics that unwind".

### D15. `s += t` stays a concatenation; in-place append is a follow-up (autonomous)

`compound-assignment` (its D3) allowed `s += t` to append in place. Doing it soundly needs a MIR form for the append: a mutable borrow of `s` taken after `t` is evaluated, a call of an append built-in, and the old concatenation kept for `s += s`, whose operand is the target. But `val r = &s; s += r` is accepted today, and becomes a borrow conflict under such a form (a mutable borrow of `s` while `r` is used), so the language's rule "`s += t` means `s = s + t`" would no longer hold for the borrow checker. Making the append a post-borrow-checking rewrite of the MIR's `tmp = &s + &t; drop s; s = move tmp` keeps the rule but is a pattern match across blocks and drop flags. Neither is needed now: UI code builds short strings, and Kotlin's `+=` on strings also builds a new string. So `s += t` keeps its MIR and runs as a concatenation, the old value being dropped; the representation already has a capacity, so the runtime can grow a buffer in place once a later change adds the append (recorded in `ROADMAP.md` with both designs). Rejected: the append form with its borrow-checker narrowing now; a `StringBuilder` type now (needs a library).

### D16. Strings in the calling convention

`String` gets `Representation::Memory` with its fixed layout, so every rule of `codegen-aggregates` applies unchanged: a `String` local lives in a stack slot, a parameter is the address of caller-owned memory (a `Move` operand passes its place, a constant is first materialized in a slot), a result is written through the caller's result address, and a move is a 32-byte copy that leaves the source untouched. A top-level `String` value's `#storage` is 32 bytes, filled by `#init` through its result address. The machine signature of `fun greet(name: String, n: Int): String` is therefore the result address, the address of `name` and an `i64`, like any aggregate's.

### D17. Runtime errors of strings and allocation (autonomous)

Two new panic kinds: `StringIndexOutOfRange`, whose message is `string index <i> out of range for length <n>`, reported at the indexing expression like an arithmetic error (the runtime function receives the location); and `OutOfMemory`, whose message is `out of memory`, reported without a location line, because an allocation can fail inside the runtime, far from any expression, and passing a location to every allocating function would cost an argument everywhere for an error programs rarely see. The `execution` spec's "Runtime Errors End the Program" gains that exception. Rejected: aborting on out of memory as Rust does (the output would be lost); Kotlin's exception text (`index: 3, length: 3`), less readable; reporting an index error at the call of the runtime function's caller frame (no frame information exists).

### D18. What the backend still rejects

`Construct::Strings` disappears: strings and every type holding them are supported, and `LayoutTable::layout` lays out `String` (D2). `classify` keeps reporting the first construct a type needs in field order: `'Heap' values` (now pointing at `heap-boxes`), `function values and closures` (`closures`), `calls of generic functions` (`monomorphization`), `composable functions` (`compose-transform`) and values of unknown type. The fixtures that pinned strings as unsupported move to `Heap<T>` (`code-generation/a-struct-is-rejected`, `unsupported-constructs-in-several-forms--*`, `one-error-per-body`, `dumping-a-file-the-backend-rejects`, `a-struct-holding-a-string`, which becomes `a-struct-holding-a-heap-value`), and the layout dump lists a struct holding a function value as the type without a layout yet.

### D19. Recommended design for `heap-boxes` (deferred, not decided here)

So that `heap-boxes` starts from a coherent plan, this change records what fits its mechanisms; `heap-boxes` decides:

- **Construction**: `Heap(value)`, a built-in generic constructor written like a struct constructor (`Heap<Int>(1)` with an explicit argument), allocating `size_of(T)` bytes with `T`'s alignment through `prism_rt_allocate` and moving the value in; a zero-sized `T` allocates nothing and uses an aligned dangling pointer.
- **Reading** stays automatic through `Heap<T>` layers, as fields and receivers already are (`Projection::Deref` of a `Heap<T>` in MIR); operators reading through boxes (`h + 1`, `E4002` today) are worth deciding with it.
- **Writing the contents**: `*h = v` for a mutable `Heap<T>` place, dropping the old contents and keeping the allocation, like `*r = v` for references (`E4019` today).
- **Moving out**: a consuming built-in `h.unbox(): T` that moves the contents out and frees the box; a `consuming fun` receiver through a `Heap<T>` (`E4017` today) can then be written `h.unbox().f()`.
- **Covariance**: `Heap<U>` to `Heap<T>` changes the representation of the contents (`Heap<Int>` given for `Heap<Int?>`), so boxes should be invariant like references (`reference-access`), with only representation-preserving steps allowed; this removes the last layout-changing `Subtype` of the typed tree besides function types.
- **Glue**: dropping a box runs the contents' drop glue and frees the box; cloning allocates a new box and clones the contents; equality of types that reach themselves through boxes moves into `#equals` glue functions (D11); glue that can recurse checks the stack (D9).

### D20. Recommended design for `string-templates` (deferred, not decided here)

Kotlin's syntax: `"Count: $count"` for a name and `"${a + b}"` for an expression, with `\$` for a literal dollar sign (an unknown escape today). Each part converts as this change allows: a `String` is borrowed, an `Int`, `Double`, `Bool` or `Char` uses the built-in `toString()`, and any other type calls a visible `toString()` extension or is rejected with the help that names it. A template lowers to one runtime call that sizes the result once (a builder taking the parts), not to a chain of `+`, so it allocates the result once.

### D21. No new capability

Strings and glue extend `code-generation` (representation, glue, allocator) and `execution` (runtime meaning, `--check-allocations`); the new built-in members extend `type-system`, `typed-tree`, `mir`, `references` and `extension-functions`; `E4010` and `E6001` change catalog rows in `diagnostics`. Rejected: a `strings` capability (its rules would be split from the operators, members and runtime rules they belong with) and a `memory-management` capability (drops are already in `mir` and `ownership`; what glue frees is the backend's contract).

## Risks / Trade-offs

- [32 bytes per string is more than Rust's 24 and Swift's 16, in every struct holding one] → the character count buys O(1) `length` and ASCII indexing under character semantics; a small-string or packed representation can come later without changing programs (D2).
- [Glue is duplicated in every unit that drops a type] → a few instructions per type per unit; identical glue can be merged by Tier 2's linker (D9).
- [Non-ASCII indexing is O(n), so a loop over `s[i]` is quadratic on such text] → documented; a character iterator with `for` over strings (iteration trait, `ROADMAP.md`) is the efficient way, and ASCII text stays O(1) (D4).
- [Static literal data must outlive every string pointing at it] → code units are never freed today; `hot-reload` must keep old modules mapped (D3, recorded in `ROADMAP.md`).
- [`--check-allocations` relies on top-level values registering themselves; a missed registration reports a false leak] → every fixture with a computed top-level string (the `execution` scenario "Top-level values are kept") exercises it.
- [`s += t` in a loop is quadratic] → as in Kotlin and Java; in-place append is a recorded follow-up (D15).
- [The `execution` and `code-generation` deltas modify `codegen-aggregates` requirements] → `codegen-aggregates` must be archived before this change; `openspec validate` reports the dependency as information until then.

## Migration Plan

No external users. Programs that borrowed `&s[i]` now borrow a temporary (D5). Fixtures that pinned `E6001` for strings move to `Heap<T>` (D18). The `E4003` help for `s[i] = c` changes. Every executable fixture is re-run with `--check-allocations`; one that fails reveals a missing drop or a double free in the backend, to fix before landing. Rollback is a revert.

## Open Questions

- Whether `prism_rt_string_drop` should be inlined (a load and a test before the call) once profiles show drops of static strings; it is an internal detail of the glue.
- The exact wording of the `E4003` help for `s[i] = c` can be tuned during implementation.
