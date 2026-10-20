//! `prism-library`: the bundled standard library (D1 of `standard-library`).
//!
//! The library's `.prism` files live in the repository's top-level `library/prism/` (D5 of
//! `explicit-library`), laid out as a workspace whose root holds the package `prism`, and this
//! crate embeds them in the `prism` executable with `include_str!`, so the compiler is one
//! self-contained file that never reads its library from disk. Every module is a file of the
//! reserved root package `prism`, or of a package below it, whose module path is `prism/<name>`
//! (`prism/compose/runtime` for the file `compose/runtime.prism` of the package `prism.compose`)
//! and which a root-anchored import `/prism/<name>` reaches.
//! The prelude modules' public names are in scope in every workspace file without an import
//! (D4).
//!
//! The crate has no dependencies, and no build script: `library_tests.rs` checks that
//! [`MODULES`] lists exactly the files of `library/prism/` and its subdirectories.

use std::sync::{Arc, OnceLock};

/// One module of the library: its module path (`prism/core`) and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryModule {
    pub path: String,
    pub source: String,
}

/// The module path and the embedded source of every bundled module.
pub const MODULES: &[(&str, &str)] = &[
    (
        "prism/core",
        include_str!("../../../library/prism/core.prism"),
    ),
    (
        "prism/collections",
        include_str!("../../../library/prism/collections.prism"),
    ),
    (
        "prism/compose/runtime",
        include_str!("../../../library/prism/compose/runtime.prism"),
    ),
    (
        "prism/compose/ui",
        include_str!("../../../library/prism/compose/ui.prism"),
    ),
    (
        "prism/compose/foundation",
        include_str!("../../../library/prism/compose/foundation.prism"),
    ),
    (
        "prism/compose/layout",
        include_str!("../../../library/prism/compose/layout.prism"),
    ),
    (
        "prism/compose/material3",
        include_str!("../../../library/prism/compose/material3.prism"),
    ),
    (
        "prism/system",
        include_str!("../../../library/prism/system.prism"),
    ),
    (
        "prism/composer",
        include_str!("../../../library/prism/composer.prism"),
    ),
];

/// The prelude modules, in the order a workspace file sees them after its explicit imports.
pub const PRELUDE: &[&str] = &["prism/core", "prism/collections", "prism/compose/runtime"];

/// The root package that names the library, which no workspace file may declare (D2).
pub const ROOT_PACKAGE: &str = "prism";

/// A set of library modules and its prelude: the bundled library, or for tests that library
/// with modules added or replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// Shared, so that cloning a library (as every dependency graph does) copies a pointer.
    modules: Arc<Vec<LibraryModule>>,
    prelude: Vec<String>,
}

impl Library {
    /// The library embedded in the executable, built once per process.
    pub fn bundled() -> &'static Library {
        static BUNDLED: OnceLock<Library> = OnceLock::new();
        BUNDLED.get_or_init(|| Library {
            modules: Arc::new(
                MODULES
                    .iter()
                    .map(|(path, source)| LibraryModule {
                        path: path.to_string(),
                        source: source.to_string(),
                    })
                    .collect(),
            ),
            prelude: PRELUDE.iter().map(|path| path.to_string()).collect(),
        })
    }

    /// The bundled library, shared: what the compiler's options hold by default.
    pub fn shared() -> Arc<Library> {
        static SHARED: OnceLock<Arc<Library>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Library::bundled().clone())))
    }

    /// The bundled library with `extra` modules, as `(module path, source)` pairs: each
    /// replaces the bundled module of its path or is added. For tests only (D13): the `prism`
    /// command line never injects modules, so what a program means never depends on anything
    /// but its sources and the compiler.
    pub fn with_modules(extra: Vec<(String, String)>) -> Library {
        let mut library = Library::bundled().clone();
        let modules = Arc::make_mut(&mut library.modules);
        for (path, source) in extra {
            match modules
                .iter_mut()
                .find(|module| module.path == path)
            {
                Some(module) => module.source = source,
                None => modules.push(LibraryModule { path, source }),
            }
        }
        library
    }

    /// Whether this is the bundled library itself, not one with injected test modules.
    pub fn is_bundled(&self) -> bool {
        Arc::ptr_eq(&self.modules, &Library::bundled().modules)
    }

    /// The module whose path is `path` (`prism/core`), if the library has one.
    pub fn module(&self, path: &str) -> Option<&LibraryModule> {
        self.modules.iter().find(|module| module.path == path)
    }

    /// Every module, bundled ones first, in the order of [`MODULES`].
    pub fn modules(&self) -> &[LibraryModule] {
        &self.modules
    }

    /// The prelude modules' paths, in prelude order.
    pub fn prelude(&self) -> &[String] {
        &self.prelude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_module_is_found_by_its_path() {
        let library = Library::bundled();
        for (path, source) in MODULES {
            let module = library.module(path).expect("a bundled module");
            assert_eq!(&module.source, source);
        }
        assert!(library.module("prism/nothing").is_none());
        for path in library.prelude() {
            assert!(library.module(path).is_some(), "{}", path);
        }
    }

    #[test]
    fn injected_modules_replace_or_add() {
        let library = Library::with_modules(vec![
            ("prism/system".to_string(), "package prism\n".to_string()),
            (
                "prism/test".to_string(),
                "package prism\nfun f() {}\n".to_string(),
            ),
        ]);
        assert_eq!(
            library.module("prism/system").unwrap().source,
            "package prism\n"
        );
        assert_eq!(
            library.module("prism/test").unwrap().source,
            "package prism\nfun f() {}\n"
        );
        assert_eq!(library.modules().len(), MODULES.len() + 1);
        assert!(!library.is_bundled() && Library::bundled().is_bundled());
        assert!(Library::shared().is_bundled());
        // The bundled library is unchanged.
        assert_ne!(
            Library::bundled().module("prism/system").unwrap().source,
            "package prism\n"
        );
    }
}
