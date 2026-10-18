# Compilation and ABI Specification Delta

## MODIFIED Requirements

### Requirement: Deterministic ABI Hash
The compiler MUST compute a deterministic 256-bit BLAKE3 hash of each file's exported ABI that covers exported signatures, enum variant shapes and default parameter values, and excludes private symbols, non-generic function bodies and source positions.

#### Scenario: Modifying non-generic function body
- **WHEN** a developer modifies the internal implementation of a public function without changing its signature
- **THEN** the file's exported ABI hash remains unchanged

#### Scenario: Modifying public function signature
- **WHEN** a developer changes the parameter types or return type of a public function
- **THEN** the file's exported ABI hash changes

#### Scenario: Modifying private helper function
- **WHEN** a private helper function is added, removed, or modified in a source file
- **THEN** the exported ABI hash is unaffected

#### Scenario: Changing a default parameter value
- **WHEN** the default value of a parameter of an exported function changes
- **THEN** the exported ABI hash changes

#### Scenario: Distinct enum variant shapes
- **WHEN** two exported enums with the same name have different variant payload shapes
- **THEN** their ABI hashes differ

#### Scenario: Reordering exported declarations
- **WHEN** exported declarations are reordered within a file without other changes
- **THEN** the exported ABI hash remains unchanged

#### Scenario: Hashes printed in full
- **WHEN** a developer runs `prism abi` on a source file, or `prism check` on a workspace
- **THEN** each ABI hash is printed as `0x` followed by 64 lowercase hexadecimal digits

## ADDED Requirements

### Requirement: Interface Hash
The compiler MUST compute, for each file, an interface hash: a deterministic 256-bit BLAKE3 hash of its exported ABI with the Monomorphization Hashes of its generic bodies left out. It MUST cover everything else the ABI hash covers, and the ABI hash MUST be a function of the interface hash and the Monomorphization Hashes.

#### Scenario: Changing a generic body keeps the interface hash
- **WHEN** the body of the exported generic function `public fun <T> keep(x: T): T` changes
- **THEN** the file's ABI hash changes and its interface hash is unchanged

#### Scenario: Changing a signature changes the interface hash
- **WHEN** a parameter type of an exported function changes, or an exported struct gains a field
- **THEN** the file's interface hash and its ABI hash both change

#### Scenario: Interface hash printed by `prism abi`
- **WHEN** a developer runs `prism abi` on a source file
- **THEN** it prints the file's interface hash after its ABI hash, in the same format

### Requirement: Compilation Avoidance via Interface Hashes
`prism check` MUST skip Phase 2 (type checking, MIR lowering and borrow checking) for a file whose content, module path and direct imports' interface hashes are those of an earlier successful check by the same compiler, recorded in the build cache (the `build-cache` spec). It MUST run Phase 2 for every other file. The report and the exit status MUST be the same either way.

#### Scenario: Modifying non-generic function body
- **WHEN** `lib/text.prism` exports `public fun shout(s: &String): String`, `app/main.prism` imports `/lib/text`, both have been checked, and the body of `shout` changes
- **THEN** the next `prism check` re-checks `lib/text` only, and reuses `app/main`: zero downstream files that import it are recompiled

#### Scenario: Modifying public function signature
- **WHEN** in that workspace `shout` gains a parameter with a default value, `n: Int = 1`, and `app/main`, whose exported signatures do not mention `lib/text`, is itself imported by `app/cli`
- **THEN** the next `prism check` re-checks `lib/text` and its direct importer `app/main`, and reuses `app/cli`, whose direct import's interface hash is unchanged

#### Scenario: An incompatible signature change is reported in the importer
- **WHEN** in that workspace the parameter of `shout` becomes `s: Int`, and nothing else changes
- **THEN** the next `prism check` re-checks `app/main` and reports its `E4002`, although `app/main` itself was not edited

#### Scenario: Modifying private helper function
- **WHEN** a private helper function of `lib/text.prism` is added, removed or modified
- **THEN** the next `prism check` re-checks `lib/text` only, and downstream files remain cached

#### Scenario: A failing file is checked again
- **WHEN** a file has type or borrow errors, and `prism check` runs again with no edit
- **THEN** the file is checked again and reports the same diagnostics, because a file with errors is never recorded

#### Scenario: Cached and uncached checks agree
- **WHEN** a workspace is checked once with the cache disabled and once from a warm cache
- **THEN** both runs print the same report and exit with the same status

### Requirement: Instantiation Records for Generic Body Changes
When a file that passes Phase 2 calls an exported generic function or extension of a direct import with type arguments that mention no type parameter of the caller, `prism check` MUST record an instance for that generic, its Monomorphization Hash and those type arguments. An instance already recorded MUST be reported as reused, any other as one to instantiate.

#### Scenario: Changing generic function body
- **WHEN** `lib/box.prism` exports `public fun <T> keep(x: T): T` and `public fun <T> drop(x: T): Int`, `app/a.prism` calls `keep(1)`, `app/b.prism` calls `drop("s")`, the workspace has been checked, and the body of `keep` changes
- **THEN** the next `prism check` re-checks `lib/box` only, reports that `app/a` must instantiate `lib/box::keep<Int>` again, and reports the instance `lib/box::drop<String>` of `app/b` as reused: re-monomorphization is triggered only in downstream files that instantiate that generic

#### Scenario: Polymorphic calls are not recorded
- **WHEN** `app/a.prism` declares `fun <T> wrap(x: T): T = keep(x)`, calling the imported generic `keep` with its own type parameter
- **THEN** no instance is recorded for that call, since its type arguments are known only when `wrap` is instantiated
