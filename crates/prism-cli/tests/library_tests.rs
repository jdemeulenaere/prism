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
            // The prelude layer keeps one symbol per name, so no prelude module may give a
            // struct a companion value (D4 of `ui-toolkit`), whose struct it would lose.
            assert!(
                names.insert(name.clone()),
                "{} exports '{}' twice",
                module,
                name
            );
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
    // Every `.prism` file of the directory and its subdirectories, by module path: the file
    // `compose/runtime.prism` is the module `prism/compose/runtime`.
    let mut on_disk = BTreeMap::new();
    let root = library_directory();
    let mut directories = vec![root.clone()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|e| e == "prism") {
                let relative = path.strip_prefix(&root).unwrap().with_extension("");
                let segments: Vec<String> = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().to_string())
                    .collect();
                on_disk.insert(
                    format!("prism/{}", segments.join("/")),
                    std::fs::read_to_string(&path).unwrap(),
                );
            }
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

/// standard-library: "A writable property the catalog lists as read-only": the message says
/// that the compiler provides `length` as a read-only property (D4 of `state-handles`).
#[test]
fn a_writable_property_the_catalog_lists_as_read_only() {
    let core = Library::bundled()
        .module("prism/core")
        .expect("the core module")
        .source
        .replace(
            "public intrinsic val String.length: Int",
            "public intrinsic var String.length: Int",
        );
    let library = Library::with_modules(vec![("prism/core".to_string(), core)]);
    let error = prism_cli::check_library(&library).expect_err("the property is not writable");
    let rendered = error.render();
    assert!(rendered.contains("error[E4029]"), "{}", rendered);
    assert!(
        rendered.contains("the compiler provides 'length' as a read-only property"),
        "{}",
        rendered
    );
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

/// package-resolution: "A diagnostic in a library file", for a syntax error: the diagnostic
/// shows the line of the embedded source, which is not on disk.
#[test]
fn a_syntax_error_in_a_library_file_shows_its_source() {
    let dir = std::env::temp_dir().join(format!(
        "prism_library_syntax_error_{}",
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
            "package prism\n// Gives a number.\npublic fun number(): Int = )\n".to_string(),
        )])),
        ..prism_cli::CheckOptions::default()
    };
    let error = prism_cli::check_path_with(&dir.join("main.prism"), &options)
        .expect_err("the library module does not parse");
    let rendered = error.render();
    assert!(rendered.contains("error[E2001]"), "{}", rendered);
    assert!(rendered.contains("--> prism/test.prism:3:"), "{}", rendered);
    assert!(
        rendered.contains("public fun number(): Int = )"),
        "the snippet shows the embedded source:\n{}",
        rendered
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// standard-library: "Signatures of the compose module" and "Signatures of the state
/// declarations", and compilation-and-abi: "The ABI of the compose module".
#[test]
fn signatures_of_the_compose_module() {
    let library = Library::bundled();
    let source = &library
        .module("prism/compose/runtime")
        .expect("the compose module")
        .source;
    let file = prism_parser::parse_source(source).expect("the module parses");
    let no_definition = |_: &str, _: &str| None;
    let abi = prism_abi::FileAbi::extract_with_origin(
        &file,
        "prism/compose/runtime",
        &prism_abi::no_imports,
        &no_definition,
        true,
    )
    .expect("the module's ABI");
    let texts: BTreeSet<String> = abi
        .symbols
        .iter()
        .filter_map(prism_abi::ExportedSymbol::declaration_text)
        .collect();
    for expected in [
        "public intrinsic struct Composition",
        "public intrinsic struct RecomposeScope : Copy",
        "public intrinsic fun composition(content: escaping compose () -> Unit): Composition",
        "public intrinsic fun Composition.setContent(content: escaping compose () -> Unit): Unit",
        "public intrinsic fun Composition.recompose(): Bool",
        "public intrinsic fun Composition.dump(): String",
        "public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T",
        "public intrinsic compose fun key(value: Int, content: compose () -> Unit): Unit",
        "public intrinsic compose fun currentRecomposeScope(): RecomposeScope",
        "public intrinsic fun RecomposeScope.invalidate(): Unit",
        // standard-library: "Signatures of the state declarations" (`state-handles`).
        "public intrinsic struct State<T> : Copy",
        "public intrinsic struct MutableState<T> : Copy",
        "public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>",
        "public intrinsic val <T> State<T>.value: T",
        "public intrinsic var <T> MutableState<T>.value: T",
        "public intrinsic fun <T, R> State<T>.read(reader: (&T) -> R): R",
        "public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T): Unit",
        "public intrinsic fun <T> MutableState<T>.edit(editor: (&mut T) -> Unit): Unit",
        "public intrinsic compose fun SideEffect(effect: escaping () -> Unit): Unit",
    ] {
        assert!(
            texts.contains(expected),
            "{} is missing from {:?}",
            expected,
            texts
        );
    }
    assert_eq!(texts.len(), 19, "{:?}", texts);
    assert!(PRELUDE.contains(&"prism/compose/runtime"));
}
