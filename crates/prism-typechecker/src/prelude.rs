//! The prelude scope (D4 of `standard-library`): the public declarations of the prelude modules
//! (`/prism/core` and `/prism/collections`), converted once per process into one shared scope
//! that every workspace file's checker consults last, after its own declarations and its
//! imports.
//!
//! Building the scope is the conversion `ingest_file_abi` does, linear in the prelude's ABI and
//! done once. A file then pays a pointer copy to install it, plus one hash lookup on each name
//! or extension lookup that misses everything else: nothing proportional to the prelude's size.

use crate::checker::{Extension, ExtensionsOfName, ReceiverKey, TypeChecker};
use crate::scope::Symbol;
use crate::types::{NominalId, BUILTIN_MODULE};
use prism_abi::FileAbi;
use prism_typed_tree::types::NominalMarker;
use prism_typed_tree::{BuiltinFunction, DeclarationPath, TypeDef, TypeTable};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The names, extensions and members of the prelude modules, shared by every checker of a
/// process.
#[derive(Debug)]
pub struct PreludeScope {
    /// The prelude modules' paths (`prism/core`), in prelude order.
    modules: Vec<String>,
    /// Every public name of the prelude modules, by bare name.
    names: Arc<HashMap<String, Symbol>>,
    /// Their extensions that are not members, by receiver type constructor and name, one per
    /// context (D8 of `receiver-lambdas`).
    extensions: HashMap<(ReceiverKey, String), ExtensionsOfName>,
    /// Their intrinsic extensions and properties, members of their receiver types.
    members: HashMap<(ReceiverKey, String), Extension>,
    /// Their plain intrinsic functions, by module and name, with their built-ins.
    intrinsic_functions: HashMap<(String, String), BuiltinFunction>,
    /// Their external functions.
    externals: Vec<DeclarationPath>,
    /// The structs and enums they declare or reach, with their definitions and markers.
    types: Vec<(NominalId, Option<TypeDef>, Option<NominalMarker>)>,
    /// Every struct and enum their ABIs name, by bare name.
    seen_nominals: HashMap<String, Vec<NominalId>>,
    /// Every struct and enum they export.
    exported_nominals: HashSet<NominalId>,
}

impl PreludeScope {
    /// The scope of the prelude modules whose ABIs are `abis`, in prelude order. Names the
    /// modules share are kept from the first; the library's own test checks there are none.
    pub fn from_abis(abis: &[&FileAbi]) -> Self {
        let mut checker = TypeChecker::new(None, "<prelude>");
        for abi in abis {
            checker.ingest_file_abi(abi);
        }
        let (
            names,
            extensions,
            members,
            intrinsic_functions,
            externals,
            table,
            seen_nominals,
            exported_nominals,
            _,
        ) = checker.into_prelude_parts();
        Self {
            modules: abis.iter().map(|abi| abi.module.clone()).collect(),
            names: Arc::new(names),
            extensions,
            members,
            intrinsic_functions,
            externals,
            types: prelude_types(&table),
            seen_nominals,
            exported_nominals,
        }
    }

    /// The prelude modules' paths, in prelude order.
    pub fn modules(&self) -> &[String] {
        &self.modules
    }

    /// The public names of the prelude modules.
    pub fn names(&self) -> Arc<HashMap<String, Symbol>> {
        Arc::clone(&self.names)
    }

    /// Whether the prelude has a name, a member or an extension named `name`.
    pub fn declares(&self, name: &str) -> bool {
        self.names.contains_key(name)
            || self.members.keys().any(|(_, member)| member == name)
            || self
                .extensions
                .keys()
                .any(|(_, extension)| extension == name)
    }

    pub(crate) fn extensions(&self, key: &(ReceiverKey, String)) -> Option<&ExtensionsOfName> {
        self.extensions.get(key)
    }

    pub(crate) fn member(&self, key: &(ReceiverKey, String)) -> Option<&Extension> {
        self.members.get(key)
    }

    pub(crate) fn intrinsic_function(&self, key: &(String, String)) -> Option<BuiltinFunction> {
        self.intrinsic_functions.get(key).copied()
    }

    pub(crate) fn externals(&self) -> &[DeclarationPath] {
        &self.externals
    }

    pub(crate) fn seen_nominals(&self) -> &HashMap<String, Vec<NominalId>> {
        &self.seen_nominals
    }

    pub(crate) fn exported_nominals(&self) -> &HashSet<NominalId> {
        &self.exported_nominals
    }

    /// Records the prelude's structs and enums in `table`: work proportional to the number of
    /// types the prelude declares, none today.
    pub(crate) fn install_types(&self, table: &mut TypeTable) {
        for (id, definition, marker) in &self.types {
            if let Some(definition) = definition {
                table.insert_definition(id.clone(), definition.clone());
            }
            if let Some(marker) = marker {
                table.insert_marker(id.clone(), marker.marker, marker.parameters.clone());
            }
        }
    }
}

/// The structs and enums of `table` other than the built-in ones.
fn prelude_types(table: &TypeTable) -> Vec<(NominalId, Option<TypeDef>, Option<NominalMarker>)> {
    table
        .ids()
        .filter(|id| id.module != BUILTIN_MODULE)
        .map(|id| {
            (
                id.clone(),
                table.definition(id).cloned(),
                table.marker_entry(id).cloned(),
            )
        })
        .collect()
}

/// The prelude scope of the bundled library, built once per process: its prelude modules
/// parsed and their ABIs extracted, for tests that build a checker directly rather than
/// through the CLI's driver.
#[cfg(any(test, feature = "test-support"))]
pub fn test_prelude() -> Arc<PreludeScope> {
    static PRELUDE: std::sync::OnceLock<Arc<PreludeScope>> = std::sync::OnceLock::new();
    Arc::clone(PRELUDE.get_or_init(|| {
        let library = prism_library::Library::bundled();
        let abis: Vec<FileAbi> = library
            .prelude()
            .iter()
            .map(|module| {
                let source = &library.module(module).expect("a prelude module").source;
                let file = prism_parser::parse_source(source).expect("the prelude parses");
                let no_definition = |_: &str, _: &str| None;
                FileAbi::extract_with_origin(
                    &file,
                    module,
                    &prism_abi::no_imports,
                    &no_definition,
                    true,
                )
                .expect("the prelude's ABI")
            })
            .collect();
        let abis: Vec<&FileAbi> = abis.iter().collect();
        Arc::new(PreludeScope::from_abis(&abis))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_the_scope_twice_gives_equal_scopes() {
        let first = test_prelude();
        let library = prism_library::Library::bundled();
        let abis: Vec<FileAbi> = library
            .prelude()
            .iter()
            .map(|module| {
                let file =
                    prism_parser::parse_source(&library.module(module).unwrap().source).unwrap();
                let no_definition = |_: &str, _: &str| None;
                FileAbi::extract_with_origin(
                    &file,
                    module,
                    &prism_abi::no_imports,
                    &no_definition,
                    true,
                )
                .unwrap()
            })
            .collect();
        let second = PreludeScope::from_abis(&abis.iter().collect::<Vec<_>>());
        let mut first_names: Vec<&String> = first.names.keys().collect();
        let mut second_names: Vec<&String> = second.names.keys().collect();
        first_names.sort();
        second_names.sort();
        assert_eq!(first_names, second_names);
        assert_eq!(first.names, second.names);
        assert_eq!(first.members.len(), second.members.len());
        assert_eq!(first.intrinsic_functions, second.intrinsic_functions);
        assert!(first.declares("println") && first.declares("size"));
    }

    #[test]
    fn installing_the_scope_ingests_nothing() {
        // A checker with the prelude shares its names: installing it copies a pointer.
        let prelude = test_prelude();
        let checker = TypeChecker::new(None, "main").with_prelude(Arc::clone(&prelude));
        assert!(Arc::strong_count(&prelude.names) >= 2);
        drop(checker);
    }
}
