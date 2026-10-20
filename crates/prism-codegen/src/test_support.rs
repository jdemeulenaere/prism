//! Building composed MIR from source for the crate's tests, through the real front end: parse,
//! Phase 1 ABIs of the earlier files, type checking, lowering and borrow checking.

use prism_abi::{declared_types, FileAbi};
use prism_mir::MirFile;

/// The composed MIR of `source` as the root module `main`.
pub fn lower_source(source: &str) -> MirFile {
    lower_files(&[("main", source)]).remove(0)
}

/// The composed MIR of each of `files`, given as `(module path, source)`. A file may import an
/// earlier file by its root-anchored path (`import /lib/math`); the files must be listed in
/// dependency order, except that each file sees the ABI of every file of the list, so two files
/// can import each other.
pub fn lower_files(files: &[(&str, &str)]) -> Vec<MirFile> {
    let parsed: Vec<_> = files
        .iter()
        .map(|(module, source)| {
            let file = prism_parser::parse_source(source)
                .unwrap_or_else(|error| panic!("{} does not parse: {:?}", module, error));
            (*module, file)
        })
        .collect();
    let imports_of = |file: &prism_ast::SourceFile| -> Vec<String> {
        file.imports
            .iter()
            .map(|import| import.path.join("/"))
            .collect()
    };
    let abis: Vec<FileAbi> = parsed
        .iter()
        .map(|(module, file)| {
            let imports = imports_of(file);
            let bind = |name: &str| {
                parsed
                    .iter()
                    .filter(|(other, _)| imports.iter().any(|import| import == other))
                    .find_map(|(other, other_file)| {
                        let header = declared_types(other_file, true).get(name)?.clone();
                        Some((header, other.to_string()))
                    })
            };
            // A module under `prism/` is a library module, which may declare externals and
            // intrinsics (D12 of `standard-library`).
            let no_definition = |_: &str, _: &str| None;
            FileAbi::extract_with_origin(
                file,
                module,
                &bind,
                &no_definition,
                module.starts_with("prism/"),
            )
            .unwrap_or_else(|error| panic!("{} has no ABI: {:?}", module, error))
        })
        .collect();
    parsed
        .iter()
        .map(|(module, file)| {
            let package = module
                .rsplit_once('/')
                .map(|(package, _)| package.replace('/', "."));
            // The member module itself sees no library module: its own members are its
            // declarations (D3 of `core-library-in-prism`).
            let mut checker = if *module == "prism/core" {
                prism_typechecker::TypeChecker::new(package, *module)
            } else {
                prism_typechecker::TypeChecker::new(package, *module).with_library_imports()
            };
            let imports = imports_of(file);
            for abi in &abis {
                if imports.contains(&abi.module) {
                    checker.ingest_file_abi(abi);
                }
            }
            let tree = checker.check_module(file);
            assert!(
                checker.errors.is_empty(),
                "{} has type errors: {:?}",
                module,
                checker.errors
            );
            let mir = prism_mir::lower_file(&tree);
            let elaborated = prism_borrowck::check_file(&mir)
                .unwrap_or_else(|errors| panic!("{} has borrow errors: {:?}", module, errors));
            // The backend compiles composed MIR (D2 of `compose-transform`).
            prism_compose_transform::compose_file(&elaborated)
        })
        .collect()
}
