//! The bundled library's own checks (the `standard-library` spec, "The Library Passes Its Own
//! Checks"; D13 of `standard-library`): every embedded module passes the compiler's checks and
//! compiles, every external matches the runtime's entry point, every module is formatted, the
//! prelude modules export distinct names, and the embedded module list is exactly the library
//! directory.

use prism_library::{Library, MODULES, PRELUDE};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn library_directory() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../prism-library/prism")
}

/// standard-library: "The library is checked by the compiler's tests".
#[test]
fn every_module_passes_the_compilers_checks() {
    let files = match prism_cli::check_library(Library::bundled()) {
        Ok(files) => files,
        Err(error) => panic!("the bundled library has errors:\n{}", error.render()),
    };
    let checked: BTreeSet<String> = files.iter().map(|file| file.module.clone()).collect();
    let embedded: BTreeSet<String> = MODULES.iter().map(|(path, _)| path.to_string()).collect();
    assert_eq!(checked, embedded, "every module is checked");
}

#[test]
fn every_module_is_formatted() {
    for (path, source) in MODULES {
        let formatted = prism_formatter::format_source(source)
            .unwrap_or_else(|error| panic!("{} does not format: {:?}", path, error));
        assert_eq!(
            &formatted, source,
            "{} is not formatted: run `prism fmt crates/prism-library/prism`",
            path
        );
    }
}

#[test]
fn every_declaration_says_what_it_does() {
    for (path, source) in MODULES {
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let declares = [
                "public ",
                "package ",
                "private ",
                "fun ",
                "external ",
                "intrinsic ",
            ]
            .iter()
            .any(|start| line.starts_with(start));
            if declares {
                assert!(
                    index > 0 && lines[index - 1].starts_with("//"),
                    "{}:{}: the declaration has no comment above it: {}",
                    path,
                    index + 1,
                    line
                );
            }
        }
    }
}

#[test]
fn the_prelude_modules_export_distinct_names() {
    let library = Library::bundled();
    let mut owners: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for module in PRELUDE {
        let source = &library.module(module).expect("a prelude module").source;
        let file = prism_parser::parse_source(source).expect("the module parses");
        let no_definition = |_: &str, _: &str| None;
        let abi = prism_abi::FileAbi::extract_with_origin(
            &file,
            module,
            &prism_abi::no_imports,
            &no_definition,
            true,
        )
        .expect("the module's ABI");
        let mut names = BTreeSet::new();
        for symbol in &abi.symbols {
            // An extension is named with its receiver: `List.map`.
            let name = match symbol {
                prism_abi::ExportedSymbol::Function {
                    receiver: Some(receiver),
                    name,
                    ..
                } => format!("{}.{}", prism_abi::receiver_constructor(&receiver.ty), name),
                prism_abi::ExportedSymbol::ExtensionProperty { receiver, name, .. } => {
                    format!("{}.{}", prism_abi::receiver_constructor(receiver), name)
                }
                prism_abi::ExportedSymbol::Function { name, .. }
                | prism_abi::ExportedSymbol::Struct { name, .. }
                | prism_abi::ExportedSymbol::Enum { name, .. }
                | prism_abi::ExportedSymbol::Var { name, .. } => name.clone(),
            };
            names.insert(name);
        }
        for name in names {
            owners.entry(name).or_default().push(module);
        }
    }
    let shared: Vec<_> = owners
        .iter()
        .filter(|(_, modules)| modules.len() > 1)
        .collect();
    assert!(shared.is_empty(), "names exported twice: {:?}", shared);
}

#[test]
fn the_embedded_modules_are_the_library_directory() {
    let mut on_disk = BTreeMap::new();
    for entry in std::fs::read_dir(library_directory()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "prism") {
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            on_disk.insert(
                format!("prism/{}", stem),
                std::fs::read_to_string(&path).unwrap(),
            );
        }
    }
    let embedded: BTreeMap<String, String> = MODULES
        .iter()
        .map(|(path, source)| (path.to_string(), source.to_string()))
        .collect();
    assert_eq!(
        embedded.keys().collect::<Vec<_>>(),
        on_disk.keys().collect::<Vec<_>>(),
        "MODULES must list every file of crates/prism-library/prism"
    );
    assert_eq!(embedded, on_disk);
    for module in PRELUDE {
        assert!(embedded.contains_key(*module), "{} is a module", module);
    }
}

/// An injected module breaking a rule fails the check: what the library test reports for the
/// bundled modules.
#[test]
fn a_wrong_external_fails_the_library_check() {
    let library = Library::with_modules(vec![(
        "prism/test".to_string(),
        "package prism\n\n// Writes an Int.\nexternal fun prism_rt_print_int(value: Double): Unit\n"
            .to_string(),
    )]);
    let error = prism_cli::check_library(&library).expect_err("the external is wrong");
    let rendered = error.render();
    assert!(rendered.contains("error[E6005]"), "{}", rendered);
    assert!(rendered.contains("prism/test.prism"), "{}", rendered);
}

/// package-resolution: "A diagnostic in a library file".
#[test]
fn a_diagnostic_in_a_library_file_names_its_module_path() {
    let dir = std::env::temp_dir().join(format!(
        "prism_library_diagnostic_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.prism"),
        "import /prism/test\nfun main() {}\n",
    )
    .unwrap();
    let options = prism_cli::CheckOptions {
        library: std::sync::Arc::new(Library::with_modules(vec![(
            "prism/test".to_string(),
            "package prism\n// Gives a number.\npublic fun number(): Int = \"one\"\n".to_string(),
        )])),
        ..prism_cli::CheckOptions::default()
    };
    let error = prism_cli::check_path_with(&dir.join("main.prism"), &options)
        .expect_err("the library module has a type error");
    let rendered = error.render();
    assert!(rendered.contains("--> prism/test.prism:3:"), "{}", rendered);
    std::fs::remove_dir_all(&dir).ok();
}
