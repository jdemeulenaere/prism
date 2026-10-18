//! Instance keys and the program-wide definitions they cover (D6 and D13 of `instance-cache`).
//!
//! An instance's code depends on its generic's record, whose Monomorphization Hash covers its
//! body, the private types it holds and its callees' signatures, and on the definitions of the
//! structs and enums its type arguments reach, which may come from modules neither the generic's
//! file nor the requester imports. Phase 1a builds the definition of every struct and enum of the
//! program from syntax trees alone ([`ProgramDefinitions`]), so both the hash of the definitions
//! an instance reaches ([`DefinitionsHasher`]) and the type table its unit is built over
//! ([`program_types`]) need no file's MIR.

use crate::cache::BuildCache;
use prism_abi::{AbiTypeDefinition, ContentHash, DeterministicHasher};
use prism_codegen::PlannedInstance;
use prism_typed_tree::{DeclarationPath, NominalId, Type, TypeTable};
use std::collections::{BTreeMap, HashMap};

/// The definition of every struct and enum of a program, private ones included, by defining
/// module and name, as Phase 1a builds them (`extract_abis`).
pub type ProgramDefinitions = HashMap<(String, String), AbiTypeDefinition>;

/// A definition's structural hash and the types its members mention, by module and name.
type DefinitionEntry = (ContentHash, Vec<(String, String)>);

/// Hashes the definitions the type arguments of instances reach, memoizing each definition's
/// own hash, so that a program's instances cost time linear in the definitions each reaches.
pub struct DefinitionsHasher<'a> {
    definitions: &'a ProgramDefinitions,
    /// Each reached definition's structural hash and the types its members mention.
    memo: HashMap<(String, String), DefinitionEntry>,
}

impl<'a> DefinitionsHasher<'a> {
    pub fn new(definitions: &'a ProgramDefinitions) -> Self {
        Self {
            definitions,
            memo: HashMap::new(),
        }
    }

    /// The hash of the definitions, markers and marker parameters included, of every struct and
    /// enum `type_arguments` reach through fields and payloads, over every module, sorted by
    /// module and name (D6 of `instance-cache`). A type with no definition (the built-in
    /// `Range`) hashes by its name; built-in types are covered by the instance's symbol.
    pub fn hash(&mut self, type_arguments: &[Type]) -> ContentHash {
        let mut pending: Vec<(String, String)> = Vec::new();
        for argument in type_arguments {
            let mut ids: Vec<NominalId> = Vec::new();
            argument.collect_nominal_ids(&mut ids);
            pending.extend(ids.into_iter().map(|id| (id.module, id.name)));
        }
        let mut reached: BTreeMap<(String, String), Option<ContentHash>> = BTreeMap::new();
        while let Some(key) = pending.pop() {
            if reached.contains_key(&key) {
                continue;
            }
            let entry = self.entry(&key);
            match entry {
                Some((hash, references)) => {
                    pending.extend(references.iter().cloned());
                    reached.insert(key, Some(*hash));
                }
                None => {
                    reached.insert(key, None);
                }
            }
        }
        let mut hasher = DeterministicHasher::new();
        hasher.write_str("definitions");
        hasher.write_u64(reached.len() as u64);
        for ((module, name), hash) in &reached {
            hasher.write_str(module);
            hasher.write_str(name);
            match hash {
                Some(hash) => {
                    hasher.write_u8(1);
                    hasher.write_hash(hash);
                }
                None => hasher.write_u8(0),
            }
        }
        hasher.finish()
    }

    fn entry(&mut self, key: &(String, String)) -> Option<&DefinitionEntry> {
        if !self.memo.contains_key(key) {
            let definition = self.definitions.get(key)?;
            let references = definition
                .referenced_types()
                .into_iter()
                .map(|(module, name)| (module.to_string(), name.to_string()))
                .collect();
            self.memo
                .insert(key.clone(), (definition.hash(), references));
        }
        self.memo.get(key)
    }
}

/// The type table of every struct and enum of the program, converted from its Phase 1a
/// definitions as an importer converts them, with the MIR's built-in `Range` (D13 of
/// `instance-cache`). Every [`NominalId`] has one definition per program, so this table agrees
/// with every file's on every type both hold.
pub fn program_types(definitions: &ProgramDefinitions) -> TypeTable {
    let mut types = TypeTable::with_builtins();
    let mut sorted: Vec<&AbiTypeDefinition> = definitions.values().collect();
    sorted.sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
    for definition in sorted {
        let (id, definition, marker) =
            prism_typechecker::types::type_definition_from_abi(definition);
        types.insert_marker(id.clone(), marker.marker, marker.parameters);
        types.insert_definition(id, definition);
    }
    prism_mir::mir_type_table(&types)
}

/// How an instance is shown to the user (D12 of `abi-cache`, D10 of `instance-cache`): its
/// generic's module, `::`, for an extension the name of its receiver type alone and `.`, its
/// name, and its type arguments, each nominal qualified by its module (`lib/box::keep<Int>`,
/// `lib/box::Box.get<app/main::Thing>`), then `#default(<parameter>)` for a default value's
/// generator. The receiver's type arguments name the extension's own type parameters, which no
/// ABI hash covers, so showing them would let a reused file show a stale name.
pub fn instance_display(
    path: &DeclarationPath,
    type_arguments: &[Type],
    default: Option<&str>,
) -> String {
    let receiver = match &path.receiver {
        Some(Type::Struct { id, .. } | Type::Enum { id, .. }) => format!("{}.", id.name),
        Some(Type::List(_)) => "List.".to_string(),
        Some(other) => format!("{}.", other),
        None => String::new(),
    };
    let shown: Vec<String> = type_arguments.iter().map(Type::qualified).collect();
    let default = default.map_or(String::new(), |name| format!("#default({})", name));
    format!(
        "{}::{}{}<{}>{}",
        path.module,
        receiver,
        path.name,
        shown.join(", "),
        default
    )
}

/// The display of the planned instance `instance`.
pub fn planned_display(instance: &PlannedInstance) -> String {
    instance_display(
        &instance.path,
        &instance.type_arguments,
        instance
            .default
            .as_ref()
            .map(|default| default.name.as_str()),
    )
}

/// The key of the code of `instance` in `cache` for the target `target` (D6 and D8 of
/// `instance-cache`).
pub fn instance_code_key(
    cache: &BuildCache,
    instance: &PlannedInstance,
    definitions: &mut DefinitionsHasher,
    target: &str,
) -> ContentHash {
    cache.instance_key(
        &instance.symbol,
        &ContentHash(instance.record_hash),
        &definitions.hash(&instance.type_arguments),
        target,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Phase 1a definitions of `files`, each `(module, source)`, with every type name bound
    /// to the module of the first file declaring it.
    fn definitions(files: &[(&str, &str)]) -> ProgramDefinitions {
        let parsed: Vec<(String, prism_ast::SourceFile)> = files
            .iter()
            .map(|(module, source)| {
                (
                    module.to_string(),
                    prism_parser::parse_source(source).expect("the source parses"),
                )
            })
            .collect();
        let headers: Vec<(String, HashMap<&str, prism_abi::NominalHeader>)> = parsed
            .iter()
            .map(|(module, file)| (module.clone(), prism_abi::declared_types(file, true)))
            .collect();
        let bind = |name: &str| {
            headers.iter().find_map(|(module, declared)| {
                declared
                    .get(name)
                    .map(|header| (header.clone(), module.clone()))
            })
        };
        let mut out = ProgramDefinitions::new();
        for (module, file) in &parsed {
            for definition in prism_abi::type_definitions(file, module, &bind) {
                out.insert(
                    (definition.module.clone(), definition.name.clone()),
                    definition,
                );
            }
        }
        out
    }

    fn thing() -> Type {
        Type::plain_struct(NominalId::new("app/main", "Thing"))
    }

    fn hash_of(files: &[(&str, &str)], arguments: &[Type]) -> ContentHash {
        DefinitionsHasher::new(&definitions(files)).hash(arguments)
    }

    const LIB: &str = "package lib\npublic struct Inner : Copy { val n: Int }\n";

    #[test]
    fn a_field_marker_or_variant_change_of_a_reached_type_changes_the_hash() {
        let base = [
            ("lib/inner", LIB),
            (
                "app/main",
                "package app\nimport /lib/inner\nstruct Thing(val a: Inner)\nstruct Other(val b: Int)\n",
            ),
        ];
        let before = hash_of(&base, &[thing()]);
        assert_eq!(before, hash_of(&base, &[thing()]));
        for inner in [
            "package lib\npublic struct Inner : Copy { val n: Int, val m: Int }\n",
            "package lib\npublic struct Inner { val n: Int }\n",
            "package lib\npublic struct Inner : Copy { val n: Bool }\n",
        ] {
            let edited = [("lib/inner", inner), base[1]];
            assert_ne!(before, hash_of(&edited, &[thing()]), "{}", inner);
        }
        let enums = |variants: &str| {
            hash_of(
                &[(
                    "app/main",
                    &format!("package app\nenum Thing {{ {} }}\n", variants),
                )],
                &[Type::plain_enum(NominalId::new("app/main", "Thing"))],
            )
        };
        assert_ne!(enums("A, B"), enums("A, B, C"));
        assert_ne!(enums("A(Int)"), enums("A(Bool)"));
    }

    #[test]
    fn an_unreached_type_does_not_change_the_hash() {
        let base = [(
            "app/main",
            "package app\nstruct Thing(val a: Int)\nstruct Other(val b: Int)\n",
        )];
        let edited = [(
            "app/main",
            "package app\nstruct Thing(val a: Int)\nstruct Other(val b: Int, val c: Int)\n",
        )];
        assert_eq!(hash_of(&base, &[thing()]), hash_of(&edited, &[thing()]));
        assert_eq!(hash_of(&base, &[Type::Int]), hash_of(&edited, &[Type::Int]));
    }

    #[test]
    fn the_hash_does_not_depend_on_the_order_definitions_are_reached() {
        let files = [(
            "app/main",
            "package app\nstruct Thing(val a: Int)\nstruct Other(val b: Thing)\n",
        )];
        let other = Type::plain_struct(NominalId::new("app/main", "Other"));
        assert_eq!(
            hash_of(&files, &[thing(), other.clone()]),
            hash_of(&files, &[other.clone(), thing()])
        );
        // `Other` reaches `Thing`, so both lists reach the same definitions.
        assert_eq!(
            hash_of(&files, std::slice::from_ref(&other)),
            hash_of(&files, &[other, thing()])
        );
    }

    #[test]
    fn recursive_types_terminate() {
        let files = [(
            "app/main",
            "package app\nenum Chain { End, Link(Int, Heap<Chain>) }\n",
        )];
        let chain = Type::plain_enum(NominalId::new("app/main", "Chain"));
        assert_eq!(
            hash_of(&files, std::slice::from_ref(&chain)),
            hash_of(&files, &[chain])
        );
    }

    #[test]
    fn instances_are_shown_by_their_identity() {
        let keep = DeclarationPath::new("lib/box", "keep");
        assert_eq!(
            instance_display(&keep, &[Type::Int], None),
            "lib/box::keep<Int>"
        );
        assert_eq!(
            instance_display(&keep, &[thing()], None),
            "lib/box::keep<app/main::Thing>"
        );
        let size = DeclarationPath::extension(
            "lib/box",
            Type::Struct {
                id: NominalId::new("lib/box", "Box"),
                arguments: vec![Type::Parameter(prism_typed_tree::TypeParameter {
                    index: 0,
                    name: "T".to_string(),
                    bound: None,
                })],
            },
            "size",
        );
        assert_eq!(
            instance_display(&size, &[Type::Int], None),
            "lib/box::Box.size<Int>"
        );
        assert_eq!(
            instance_display(&keep, &[Type::Bool], Some("y")),
            "lib/box::keep<Bool>#default(y)"
        );
    }
}
