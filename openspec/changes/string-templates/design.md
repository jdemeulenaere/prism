# Design: string-templates

## Context

See `proposal.md` for motivation. This is item 19 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17. Its starting point is D20 of `runtime-strings-and-drops`: Kotlin's syntax; a `String` part borrowed, a scalar part converted by the built-in `toString()`, any other type by a visible `toString()` extension or rejected; and one runtime call that sizes the result once. Decisions marked **(autonomous)** were made by the proposer and are logged in `docs/decisions-for-review.md`.

This change lands after `runtime-strings-and-drops`, `payload-patterns` and `heap-boxes`, which are all open (proposed, not archived) as this is written. It relies on them as follows:

- **`runtime-strings-and-drops`** (being implemented). `String` is a 32-byte `PrismString` (pointer, byte length, character count, capacity; its D2), literals are static data owning no buffer (D3), string operations are runtime functions in `crates/prism-runtime/src/string.rs` taking strings by address (D7), and `prism_rt_int_to_string`, `prism_rt_double_to_string`, `prism_rt_bool_to_string` and `prism_rt_char_to_string` give the text `print` writes. Every buffer goes through `prism_rt_allocate` (D8). Every executable fixture runs under `prism run --check-allocations` (D13), and codegen tests read `prism_runtime::allocation_statistics()`. The built-in `toString()` of `Int`, `Double`, `Bool` and `Char` wins over extensions, and on every other type `toString` is an ordinary name that an extension may declare (D6).
- **`payload-patterns`** takes `E4024` and `E4025`, so this change's new code is `E4026`.
- **`heap-boxes`** makes `print`, `toString()` and operators read through `Heap<T>` layers (its D4), which template parts follow.

Current state of the front end:

- **Lexer** (`crates/prism-lexer/src/lexer.rs`). It is a pull lexer. `next_token` skips whitespace and comments, then dispatches on the next byte, and `lex_string` decodes a literal into one `TokenKind::String(text)`, rejecting a raw newline. `lex_escape` knows `n r t \ " ' 0 u` and rejects any other escape. `$` outside a literal is "Unexpected character". The comment-collecting mode (`tokenize_with_comments`) yields the same tokens. No source under `examples/` or the fixtures holds a `$` in a string. The one formatter golden file that does (`formatting-formatted-code--every-construct`, `"many $big"`) becomes a template, which only needs to format as before.
- **Parser** (`crates/prism-parser/src/parser.rs`). It is a Pratt parser over a `Vec<Token>`. `parse_atom` turns a `String` token into `ExprKind::Literal(Literal::String)`. `with_newlines(false, ...)` makes newlines insignificant, as inside parentheses.
- **Formatter** (`crates/prism-formatter`). It prints the AST through a document algebra (`document.rs`: text, lines, groups, `if_break`, comments, and `Document::forces_break`), reading literal spellings back from the tokens (`literal_text`). It checks on every run that the output parses to the same tree (`verify.rs`, `erase_expression`). The corpus tests (`tests/corpus_tests.rs`) build variants by inserting comments before every token, squeezing whitespace between tokens and lengthening identifiers.
- **Type checker** (`crates/prism-typechecker/src/checker.rs`). `check_printed_argument` already classifies `print`'s argument: a scalar by value through references, a `String` borrowed, anything else `E4004`. `infer_method_call` resolves `x.name(...)`: a field first, then the built-ins, then `find_extension`/`call_extension`, with `pass_receiver` passing the receiver by its mode.
- **Typed tree and MIR.** An operator over strings is a typed-tree `Binary` over operands read in place. MIR lowers it in `build/comparison.rs` (`lower_binary`) to `Rvalue::Binary` over shared `Operator` borrows (`comparison_operand`), while `Copy` operands go through `ordered_operands`, which stabilizes a read that a later operand may write. Calls are terminators, and string concatenation is an rvalue that the backend lowers to a runtime call.

## Goals / Non-Goals

**Goals:**
- Kotlin's line string templates, with the same meaning for every template that has one in Prism, so UI code reads as in Jetpack Compose.
- A template allocates exactly once, whatever its parts: none for each scalar, and none for the pieces of a concatenation chain.
- Templates borrow their parts and never move them: `"$name!"` leaves `name` usable.
- A template whose part has no text form gets one diagnostic that says how to fix it.
- `prism fmt` keeps every template's text exactly as written.

**Non-Goals:**
- Raw and multi-line strings (`"""`, `trimIndent`, `trimMargin`) (D1).
- Format specifiers and padding (`"%.2f"`, `padStart`), which need a formatting library.
- Printing structs and enums with `print`/`println` through their `toString()` (D6).
- Evaluating templates at compile time, and caching their text across recompositions.

## Decisions

### D1. Line templates only; raw and multi-line strings later (autonomous)

This change covers templates in ordinary (line) string literals only. Kotlin's raw strings (`"""..."""`) bring their own decisions: no escapes, `${'$'}` for a dollar sign, newlines in the text, and indentation stripping (`trimIndent`), which is a library function in Kotlin and would be a compile-time rule here. The mode stack of D3 extends to raw strings with one more mode, so deferring them costs nothing now. `"""` keeps its meaning today: an empty literal followed by the start of another literal. Rejected: raw strings in this change (they double the lexer and formatter scenarios for a feature UI text rarely needs). Follow-ups for `ROADMAP.md`: raw and multi-line strings; format specifiers; `print`/`println` of any type with a text form (D6); folding templates whose parts are all constant.

### D2. `$name`, `${expression}`, and when `$` is literal (autonomous)

Kotlin's rules, which are deterministic at the lexer:

- `$` followed by an identifier start (`a`-`z`, `A`-`Z`, `_`) begins a **name part**. The name is the longest run of identifier characters, so `"$a.b"` inserts `a` followed by the text `.b`, and `"$name's"` inserts `name` followed by `'s`. The name is an identifier or `this` (`"$this"` inserts the receiver). Any other keyword (`"$if"`, `"$true"`, `"$null"`) is a lexer error, `E2001`, saying to write `\$if` for the text or `${...}` for an expression. Kotlin accepts some of these, but none means anything useful in a template, and rejecting them keeps "a `$name` part is the expression `name`" exact.
- `$` followed by `{` begins an **expression part**, which ends at the matching `}` (D3, D4). `"${}"` is a syntax error, since an expression is required.
- `$` followed by anything else is the character `$`: `"$5"`, `"a $ b"`, `"$"`, `"$$x"` (a dollar sign, then the part `$x`), and `"$é"` (identifiers are ASCII).
- `\$` is a new escape for `$`, valid in string and `Char` literals (`"\$name"`, `'\$'`), as in Kotlin. It joins the escapes the `syntax` spec lists, so `"\$"` is no longer an unknown escape.

A `$` followed by a newline is a literal `$`, and the newline then ends the literal with the usual "raw unescaped newline" error. Rejected: Swift's `\(expression)` and JavaScript's backquoted literals (foreign to Kotlin code); keeping `$` literal unless braced (`"${x}"` only), which breaks the common Kotlin form.

### D3. The lexer emits flat template tokens and keeps a mode stack (autonomous)

A template is lexed into a flat token run, as Kotlin's own lexer does:

| Token | Source | Span |
|---|---|---|
| `TemplateStart` | the opening `"` | the quote |
| `TemplateText(text)` | a run of text between parts, escapes decoded | the run as written |
| `TemplateName(name)` | `$name` | `$` and the name |
| `TemplateExpressionStart` | `${` | the two characters |
| ordinary tokens | the expression | as usual |
| `TemplateExpressionEnd` | the `}` matching `${` | the brace |
| `TemplateEnd` | the closing `"` | the quote |

So `"a $b ${c + d}!"` is `TemplateStart`, `TemplateText("a ")`, `TemplateName("b")`, `TemplateText(" ")`, `TemplateExpressionStart`, `Ident(c)`, `Plus`, `Ident(d)`, `TemplateExpressionEnd`, `TemplateText("!")`, `TemplateEnd`. A text run is never empty, so two parts can be adjacent (`"$a$b"`). A literal without any part is still one `String(text)` token, so plain strings and all their consumers are unchanged.

The lexer holds a stack of modes: `Template` while lexing a template's text, and `TemplateExpression { open_braces }` while lexing the tokens of an expression part. In `Template` mode, `next_token` skips no whitespace and lexes the next text run or part. In `TemplateExpression` mode it lexes ordinary tokens. A `{` increments `open_braces`, and a `}` with `open_braces` at 0 is the `TemplateExpressionEnd` that pops the mode, while any other `}` decrements it. A `"` in either code mode starts a literal that may push a nested `Template`. Brace counting is exact because braces inside string, `Char` literals and comments are never tokens. When `lex_string` reaches the first part of a literal, it rewinds to just after the opening quote (the cursor, line and column it saved) and returns `TemplateStart`, pushing `Template`. The literal's prefix is thus scanned twice, once.

Errors never panic and always end the mode they occur in. A raw newline in template text is an `Error` token ("Unterminated string template: raw unescaped newline"), and the `Template` mode is popped without consuming the newline. At the end of the file, an open mode yields one `Error` token ("Unterminated string template") before `Eof`. The parser stops at the first error token, as today. The comment-collecting mode yields the same tokens, and collects comments written inside expression parts.

Rejected: one `Template` token holding nested token vectors per part, since the parser would need a sub-parser over each vector, every tool that looks up tokens by offset (the formatter, the future LSP) would have to search a tree, and comments would have to be collected per vector. Also rejected: lexing an expression part by re-running the lexer on a substring, which loses spans and comments and cannot find the matching `}` without lexing anyway. And rejected: making the parser drive the lexer, which would need a parser that pulls tokens, while today it takes a token vector.

### D4. An expression part holds any expression, over lines and with comments (autonomous)

`${...}` holds one expression of any kind, parsed with newlines insignificant (`with_newlines(false, ...)`, as inside parentheses), so `"${a +\n b}"` is `a + b`. Closures, `when` and blocks inside it parse their bodies with their own newline rules, as they do inside parentheses. Nested literals and templates (`"${if (n == 1) "one" else "$n items"}"`) and comments (`"${a /* why */ + b}"`) are ordinary tokens. This matches Kotlin, where `${}` may span lines even in a line string. Rejected: forbidding newlines inside `${}` (Kotlin code that breaks a long `when` inside a template would not parse); forbidding comments there (the lexer would need a mode-specific rule for no gain).

### D5. One tree node for every template

The parser builds `ExprKind::Template(Vec<TemplatePart>)` with `TemplatePart::Text(String)` (the decoded text) and `TemplatePart::Expression(Expr)`. A name part becomes an `Ident` (or `This`) expression whose span covers the name without the `$`, so diagnostics point at the name. `"$x"` and `"${x}"` therefore give the same tree. Which one was written is read back from the tokens, as the formatter already does for literal spellings (D5 of `prism-fmt`). A template is an atom: postfix operators apply to it (`"$a$b".length`), and it is never a place. The span-free expression hash of `prism-abi` covers templates, so editing a template inside a generic body changes its hash, as any other body edit does.

### D6. The text form of a part (autonomous)

A part's text form depends on its type, after reading through `&`, `&mut` and `Heap<T>` layers, as `print` reads its argument (`check_printed_argument`, and D4 of `heap-boxes`):

- **`String`**: its text. The part is read in place, never moved (D11).
- **`Int`, `Double`, `Bool`, `Char`**: the text `print` writes, which is also what the built-in `toString()` gives (`-42`, `1.0E7`, `true`, `é`).
- **`Nothing`**: accepted, since such a part never produces a value (`"${return}"`).
- **Any other type**: the type checker resolves `part.toString()` exactly as the method call `.toString()` would be resolved (a field of function type named `toString`, then an extension `toString` of the file or an import, by the precedence of the `extension-functions` spec, including `E4012` for an ambiguous import), and inserts that call's result. The resolved member must be callable with no arguments and return `String`. If it is an extension, it must take a shared receiver: a `mut fun` or `consuming fun` `toString()` would mutate or move a part that looks borrowed, so it is not called implicitly, and the help suggests the explicit `${c.toString()}`, where the call is visible. The result is a temporary that the template borrows and that is dropped at the end of the statement.
- **Nullable types** are rejected (D7). `Unit`, function types, type parameters (which have no members until traits exist) and structs or enums without a usable `toString()` have no text form. All of them are `E4026` (D8).

Reusing the member-call resolution gives one rule ("a part means what `.toString()` means, with no move or mutation"), rather than a second lookup that could diverge from it: the template conversion is exactly the explicit call. A generic `toString()` extension (`fun <T> Box<T>.toString(): String`) is a generic call, which the backend rejects with `E6001` until `monomorphization`, like every generic call.

Rejected: using only `toString` extensions and ignoring a field named `toString` (a template and an explicit `.toString()` could then call different code); calling `mut` and `consuming` extensions (a hidden move or mutation inside text); a generated text form for structs and enums (D16 of `codegen-aggregates`, D6 of `runtime-strings-and-drops`); extending `print` and `println` to types with a `toString()` extension in this change (a separate decision about `print`'s argument rules, recorded as a follow-up).

### D7. Nullable parts are rejected (autonomous)

Kotlin inserts `null` for a null part. Prism rejects every nullable part, `T?` for any `T` (including `String?` and `null` itself), with `E4026` and help suggesting a fallback: `"Hello ${name ?: "guest"}"`. Reasons: "Hello null" in a UI is a bug users see, and a language with explicit null safety should make the null case visible at the template. Also, accepting nullable parts later breaks no program, while rejecting them after accepting them would. The fallback costs one `?:`. Rejected: Kotlin's `null`; Swift's approach, which accepts the part and warns (Prism has no warnings yet); printing nothing for `null` (silently loses information).

### D8. `E4026` (`NoTextForm`) for parts without a text form (autonomous)

A new type-checker code, the next after `payload-patterns`' `E4024` and `E4025` (if that numbering changes before this lands, the implementer takes the next free `E4xxx` code, per the `diagnostics` spec). Its label is the part (the name of a `$name` part, the expression of a `${}` part), its message names the type and the reason, and its help names a fix:

| Reason | Message | Help |
|---|---|---|
| nullable | "a nullable 'Int?' has no text form in a string template" | "give a fallback for null: '${n ?: 0}'" (with the part's own text) |
| no `toString()` | "'User' has no text form in a string template" | "declare 'fun User.toString(): String' to give it one" |
| no `toString()`, written `$u.name` | as above | also "to insert a member, write '${u.name}'" |
| `toString()` returns another type | "'toString()' of 'Point' returns 'Int', not 'String'" | none |
| `toString()` takes arguments | "'toString' of 'Point' needs arguments" | none |
| `mut` or `consuming` `toString()` | "a string template does not call the 'mut' 'toString()' of 'Counter' implicitly" | "call it explicitly: '${c.toString()}'" |
| `Unit`, function, type parameter | "'Unit' has no text form in a string template" | none |

It is a new code rather than `E4004` (which `print` uses for an unprintable argument) because a template part is not a call argument, and rather than `E4006` (unknown member) because the user never wrote `toString`. Each reason has its own fix, and the code's catalog row lists them all. Rejected: reusing `E4004`; one code per reason (the catalog would grow by six codes for one rule).

### D9. The typed tree: a template node over parts read in place

`ExpressionKind::Template { parts: Vec<TemplatePart> }`, of type `String`, with `TemplatePart::Text(String)` and `TemplatePart::Value(ExpressionId)`. A value part is an expression of type `String`, `Int`, `Double`, `Bool` or `Char`, **read in place**, as the operands of an operator are. A place (a binding, a field, a dereference of a reference or box) is not wrapped in a move or copy. A computed value is a `Temporary`. A `Nothing` part is coerced to `String` (`CoercionKind::Diverge`). A `toString()` part is the normalized call (an extension call with its receiver, or a field call), in a `Temporary`. The typed-tree validator checks the part types. The dump prints the node as `template`, with each text part as `text "..."` and each value part as a child.

Rejected: making the template a call of a built-in, with string literal nodes for the text (text would become argument values, each borrowed from a temporary, and the call would need variadic parameters); converting scalar parts to `ToString` calls in the typed tree (one allocation per scalar part, against the goal of one allocation per template).

### D10. MIR: one `Rvalue::Template` (autonomous)

`Rvalue::Template(Vec<TemplatePart>)` with `TemplatePart::Text(String)` and `TemplatePart::Value(Operand)`, assigned to a `String` place. A value operand is a `&String` from a shared `Operator` borrow (the existing origin for operators that read non-`Copy` operands in place, D14 of `mir-lowering`, whose documentation widens to cover template parts), or an `Int`, `Double`, `Bool` or `Char`. `prism dump-mir` prints it as `template("Count: ", copy _1, "!", move _3)`. A `toString()` part's call is an ordinary `Call` terminator before the rvalue, writing a temporary that the part borrows and that is dropped at the end of the statement. The validator checks each operand's type and the destination's.

An rvalue, like string `+`, rather than a `Call` of a built-in: a template cannot fail except by running out of memory, which needs no block boundary; the borrow checker sees its operands as uses at one point, as it does for `+`; and the text parts are constants of the rvalue, not operands. Rejected: a `Call` terminator of a variadic built-in (every text part would need a materialized string constant operand, and `Call` operands follow declared parameters); lowering to a chain of `+` (an allocation and a copy per step); lowering scalar parts to `toString()` calls (D9).

### D11. Parts are evaluated left to right and borrowed until the template is built

The parts are evaluated in source order. A scalar part is read at its position: when a later part may write it, its value is first copied to a temporary (`ordered_operands` and `stabilize`, as for operator operands), so `"$n ${bump(&mut n)} $n"` with `n == 1` gives `1 2 2`, as in Kotlin. A `String` part is borrowed at its position, and the borrow lasts until the template is built. So a later part that needs a mutable borrow of the same string conflicts with it (`E5003`), exactly as `s + f(&mut s)` does. A template never moves a part: `"$s!"` leaves `s` usable, and a part that is a temporary (`${a + b}`, `${make()}`) is dropped at the end of the statement. Rejected: copying each string part when it is read, in Kotlin's style (a hidden copy per part); moving temporaries into the runtime call to reuse their buffers (an optimization for later, since it needs the runtime to free them).

### D12. The runtime builds the result in one call with one allocation (autonomous)

`crates/prism-runtime/src/string.rs` gains `prism_rt_string_template(destination: *mut PrismString, parts: *const TemplatePart, count: usize)` and the `#[repr(C)] struct TemplatePart`, 32 bytes, a kind word and three payload words:

| Kind | Payload |
|---|---|
| text | pointer to static text, byte length, character count (all compile-time constants) |
| string | address of a `PrismString` |
| `Int`, `Double`, `Bool`, `Char` | the value (`i64`, `f64` bits, `0`/`1`, Unicode scalar) |

The runtime makes two passes. The first measures every part: text and strings from their stored lengths and counts, scalars by formatting them with the formatting `print` uses (`format_int`, `format_double`, `format_bool` and `format_char` in `output.rs` and `double.rs`), into a fixed buffer on the stack where it can (an `Int` text is at most 20 bytes, a `Double` text at most 24). It then allocates exactly the total byte length once, through `prism_rt_allocate` (capacity equal to length, D2 of `runtime-strings-and-drops`). The second pass copies and formats into the buffer, and the result's character count is the sum of the parts'. An empty result is the static empty string and allocates nothing. A total that does not fit `isize` is "out of memory". Scalar parts are formatted twice, which is cheaper than an allocation each. Scratch memory the runtime uses internally is not a program allocation and is not tracked by `--check-allocations`.

Rejected: building the result by concatenation (an allocation and a copy per part); converting scalar parts with `toString()` first (an allocation per scalar part); a growable buffer (reallocations, and a capacity larger than the length, which nothing else produces); passing parts as an array of `PrismString` pointers with scalars converted beforehand (the same extra allocations).

### D13. The backend fills a part table on the stack

`lower.rs` lowers `Rvalue::Template` in `assign`, since its destination is a `String` in memory. It allocates a stack slot of 32 bytes per part, writes each entry (text entries from the unit's read-only text data, which `UnitContext::text` deduplicates, with the counts computed at compile time; string entries from the address of the borrowed operand; scalar entries from the operand's value), then calls `prism_rt_string_template` with the destination's address, the slot's address and the count. `supported.rs` accepts the rvalue whenever its operands are supported. A template therefore costs one call, whatever its parts, and `prism dump-clif` shows it as such. A part table in read-only data, with only the value entries written at run time, is a possible later optimization and needs data relocations.

### D14. The formatter prints templates as written, on one line (autonomous)

- A template's text runs and `$name` parts are printed from their tokens, exactly as written, escapes and unicode sequences included (`"\u{1F680}"` stays). A `${x}` is never rewritten to `$x`, nor the reverse.
- Each `${...}` expression is printed by the ordinary expression printer inside a new `flat` document primitive, which renders its contents in flat mode whatever the width. So `${a+b}` becomes `${a + b}`, and a template is never broken across lines, as Prettier does for JavaScript template literals.
- A template is printed **verbatim** from source, from its opening to its closing quote, when one of its expression parts cannot be printed on one line: it holds a comment, it was written over several lines, or its document forces a break (blocks, `when`, closures with several statements). The comment cursor is moved past the comments inside it, so they are printed once, in order.
- The meaning check (`verify.rs`) compares template parts like other nodes. The corpus variants insert nothing, and squeeze nothing, between the tokens of a template's text level (before a `TemplateText`, `TemplateName`, `TemplateExpressionStart` or `TemplateEnd`), since that would change the text. `with_long_names` also lengthens `TemplateName` tokens.

Rejected: always printing templates verbatim (inconsistent spacing inside `${}`); breaking long templates inside `${}` (the output would depend on the width in a place that readers expect to stay on one line, and would move text); rewriting redundant braces (that is a lint, and it changes how the code was written).

### D15. No new capability

Templates extend `syntax` (lexing and the tree), `type-system` (text forms and `E4026`), `references` (borrowed parts), `typed-tree`, `mir`, `code-generation`, `execution` and `formatting`. All requirements are ADDED except the modified "Lexical Tokenization and Escape Sequences", which no other open change modifies, so the deltas do not conflict with the open changes. Rejected: a `string-templates` capability, which would split the rules from the stages they belong to.

## Risks / Trade-offs

- [Rejecting nullable parts surprises Kotlin programmers] → the help gives the fix with the part's own text; relaxing later breaks nothing (D7).
- [The lexer gains state, and errors inside nested templates must not desynchronize it] → every error pops its mode, and lexer tests cover unterminated text, unterminated parts, nested templates and braces inside parts at the end of the file (D3).
- [Verbatim printing leaves badly spaced code inside multi-line or commented parts] → rare in practice; such parts are better extracted to a `val`, which the formatter then formats (D14).
- [The corpus variants inserted comments before every token and would change template text] → the variants skip the text-level tokens of templates (D14).
- [Brace counting relies on the tokens inside `${}`] → braces inside literals and comments are never tokens, so counting is exact (D3).
- [`toString()` parts allocate a temporary each] → only for user types. Scalars and strings never allocate, and the codegen tests pin one allocation for `"Count: $count"` (D12).
- [Depends on three open changes] → `runtime-strings-and-drops`, `payload-patterns` and `heap-boxes` must be archived before this change starts. `openspec validate` passes now because every requirement here is ADDED, except "Lexical Tokenization and Escape Sequences", which exists in the main `syntax` spec.

## Migration Plan

No external users. A `$` followed by an identifier or `{` inside an existing string literal now starts a template: none in `examples/` or the fixtures does except the formatter golden file `"many $big"`, which formats unchanged. `"\$"` was an unknown escape and is now valid. Rollback is a revert.

## Open Questions

- Whether `$` followed by a keyword should insert the keyword's text instead of being an error; error now, since relaxing is compatible (D2).
- Whether the parts table should live in read-only data with only its value entries written at run time (D13); decided by profiling, without changing behaviour.
