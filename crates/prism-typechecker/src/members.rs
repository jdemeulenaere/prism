//! The member scope (D2 of `explicit-library`): the intrinsic extensions the member modules
//! (`/prism/core` and `/prism/collections`) declare on the built-in types, which are members of
//! those types in every workspace file without an import (`42.toString()`, `s.length`,
//! `xs.size`, `xs.add(1)`), converted once per program into one shared scope. Nothing else of
//! the member modules is in scope without an import: `println`, `listOf` and the helpers are
//! imported like any module's names.
//!
//! Building the scope is the conversion `ingest_file_abi` does, linear in the member modules'
//! ABIs and done once. A file then pays a pointer copy to install it, plus one hash lookup on
//! each member lookup its own and imported members miss.

use crate::checker::{Extension, ReceiverKey, TypeChecker};
use prism_abi::FileAbi;
use std::collections::HashMap;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

/// The members of the built-in types that the member modules declare, shared by every checker
/// of a program.
#[derive(Debug)]
pub struct MemberScope {
    /// The intrinsic extensions and properties, by receiver type constructor and name.
    members: HashMap<(ReceiverKey, String), Extension>,
}

impl MemberScope {
    /// The members declared by the member modules whose ABIs are `abis`.
    pub fn from_abis(abis: &[&FileAbi]) -> Self {
        let mut checker = TypeChecker::new(None, "<members>");
        for abi in abis {
            checker.ingest_file_abi(abi);
        }
        Self {
            members: checker.into_members(),
        }
    }

    /// Whether the scope has a member named `name`, on any receiver.
    pub fn declares(&self, name: &str) -> bool {
        self.members.keys().any(|(_, member)| member == name)
    }

    pub(crate) fn member(&self, key: &(ReceiverKey, String)) -> Option<&Extension> {
        self.members.get(key)
    }
}

/// The ABIs of the bundled library's modules `modules`, parsed and extracted with no import
/// bound, for tests that build a checker directly rather than through the CLI's driver.
#[cfg(any(test, feature = "test-support"))]
fn bundled_abis(modules: &[&str]) -> Vec<FileAbi> {
    let library = prism_library::Library::bundled();
    modules
        .iter()
        .map(|module| {
            let source = &library.module(module).expect("a bundled module").source;
            let file = prism_parser::parse_source(source).expect("the module parses");
            let no_definition = |_: &str, _: &str| None;
            FileAbi::extract_with_origin(
                &file,
                module,
                &prism_abi::no_imports,
                &no_definition,
                true,
            )
            .expect("the module's ABI")
        })
        .collect()
}

/// The member scope of the bundled library, built once per process.
#[cfg(any(test, feature = "test-support"))]
pub fn test_members() -> Arc<MemberScope> {
    static MEMBERS: std::sync::OnceLock<Arc<MemberScope>> = std::sync::OnceLock::new();
    Arc::clone(MEMBERS.get_or_init(|| {
        let abis = bundled_abis(prism_library::MEMBER_MODULES);
        let abis: Vec<&FileAbi> = abis.iter().collect();
        Arc::new(MemberScope::from_abis(&abis))
    }))
}

/// The library modules that tests building a checker directly import (D11 of
/// `explicit-library`): `/prism/core`, `/prism/collections` and `/prism/compose/runtime`, the
/// modules of the former prelude, whose ABIs are parsed once per process. Their inline sources
/// test the checker and the stages after it, not name resolution, which the fixtures cover with
/// real imports.
#[cfg(any(test, feature = "test-support"))]
pub fn test_library_imports() -> &'static [FileAbi] {
    static ABIS: std::sync::OnceLock<Vec<FileAbi>> = std::sync::OnceLock::new();
    ABIS.get_or_init(|| bundled_abis(&["prism/core", "prism/collections", "prism/compose/runtime"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_member_scope_holds_the_members_of_built_in_types_only() {
        let members = test_members();
        assert!(members.declares("size") && members.declares("toString"));
        assert!(members.declares("length") && members.declares("add"));
        // Functions and helpers are imported, not members.
        assert!(!members.declares("println") && !members.declares("listOf"));
        assert!(!members.declares("map") && !members.declares("forEach"));
    }

    #[test]
    fn installing_the_scope_copies_a_pointer() {
        let members = test_members();
        let checker = TypeChecker::new(None, "main").with_members(Arc::clone(&members));
        assert!(Arc::strong_count(&members) >= 2);
        drop(checker);
    }
}
