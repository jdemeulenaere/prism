//! The bundled library's own checks (the `standard-library` spec, "The Library Passes Its Own
//! Checks"; D13 of `standard-library`): every embedded module passes the compiler's checks and
//! compiles, every external matches the runtime's entry point, every module is formatted, the
//! member modules declare members of built-in types only, and the embedded module list is
//! exactly the library directory.

use prism_library::{Library, MEMBER_MODULES, MODULES};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The library's sources: the top-level `library/prism` of the repository (D5 of
/// `explicit-library`).
fn library_directory() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/prism")
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
            "{} is not formatted: run `prism fmt library/prism`",
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
                // A context parameter's line is part of the declaration below it (D10 of
                // `receiver-lambdas`), so the comment is above it.
                let index = if index > 0 && lines[index - 1].starts_with("context(") {
                    index - 1
                } else {
                    index
                };
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

/// The member module declares the members of the built-in types (D2 of `explicit-library`, D3
/// of `core-library-in-prism`): every extension it exports, plain or intrinsic, function or
/// property, has a built-in receiver, since the member scope gives every workspace file each of
/// them, and it exports no other function than `print` and `println`.
#[test]
fn the_member_modules_declare_members_of_built_in_types() {
    let library = Library::bundled();
    assert_eq!(MEMBER_MODULES, ["prism/core"]);
    for module in MEMBER_MODULES {
        let source = &library.module(module).expect("a member module").source;
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
        let mut members = 0;
        for symbol in &abi.symbols {
            let receiver = match symbol {
                prism_abi::ExportedSymbol::Function {
                    receiver: Some(receiver),
                    ..
                } => prism_abi::receiver_constructor(&receiver.ty),
                prism_abi::ExportedSymbol::ExtensionProperty { receiver, .. } => {
                    prism_abi::receiver_constructor(receiver)
                }
                prism_abi::ExportedSymbol::Function { name, .. } => {
                    assert!(
                        name == "print" || name == "println",
                        "{} exports the function '{}', which is not a member",
                        module,
                        name
                    );
                    continue;
                }
                other => panic!("{} exports {:?}, which is not a member", module, other),
            };
            // `List<T>` without its type arguments.
            let receiver = receiver.split('<').next().unwrap_or_default().to_string();
            assert!(
                ["Int", "Double", "Bool", "Char", "String", "List"].contains(&receiver.as_str()),
                "{} declares a member of '{}', which is not a built-in type",
                module,
                receiver
            );
            members += 1;
        }
        assert!(members > 0, "{} declares no member", module);
    }
}

/// standard-library: "Declarations of the core module", and compilation-and-abi: "The ABI of
/// a library module": `/prism/core` exports intrinsic and ordinary members, each entry with its
/// kind, and no external; changing an entry from intrinsic to ordinary changes its ABI hash.
#[test]
fn declarations_of_the_core_module() {
    let library = Library::bundled();
    let source = &library
        .module("prism/core")
        .expect("the core module")
        .source;
    let abi_of = |source: &str| {
        let file = prism_parser::parse_source(source).expect("the module parses");
        let no_definition = |_: &str, _: &str| None;
        prism_abi::FileAbi::extract_with_origin(
            &file,
            "prism/core",
            &prism_abi::no_imports,
            &no_definition,
            true,
        )
        .expect("the module's ABI")
    };
    let abi = abi_of(source);
    let texts: BTreeSet<String> = abi
        .symbols
        .iter()
        .filter_map(prism_abi::ExportedSymbol::declaration_text)
        .collect();
    for expected in [
        "public fun Int.toString(): String",
        "public intrinsic val String.length: Int",
        "public intrinsic val <T> List<T>.size: Int",
        "public fun <T> List<T>.isEmpty(): Bool",
        "public val <T> List<T>.indices: Range",
        "public intrinsic mut fun <T> List<T>.add(element: T): Unit",
    ] {
        assert!(
            texts.contains(expected),
            "{} is missing from {:?}",
            expected,
            texts
        );
    }
    assert!(
        !texts.iter().any(|text| text.contains("external")),
        "{:?}",
        texts
    );
    let ordinary = abi_of(&source.replace(
        "public intrinsic mut fun <T> List<T>.add(element: T): Unit",
        "public mut fun <T> List<T>.add(element: T): Unit {}",
    ));
    assert_ne!(abi.abi_hash, ordinary.abi_hash);
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
        "MODULES must list every file of library/prism"
    );
    assert_eq!(embedded, on_disk);
    for module in MEMBER_MODULES {
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
        "public intrinsic fun composition(content: attached compose () -> Unit): Composition",
        "public intrinsic fun Composition.setContent(content: attached compose () -> Unit): Unit",
        "public intrinsic fun Composition.recompose(): Bool",
        "public intrinsic fun Composition.dump(): String",
        "public intrinsic compose fun <T> remember(calculation: () -> T): T",
        "public intrinsic compose fun key(value: Int, content: compose () -> Unit): Unit",
        "public intrinsic compose fun currentRecomposeScope(): RecomposeScope",
        "public intrinsic fun RecomposeScope.invalidate(): Unit",
        // standard-library: "Signatures of the state declarations" (`owned-state`).
        "public intrinsic struct State<T>",
        "public intrinsic struct MutableState<T>",
        "public intrinsic struct WeakReference<T> : Copy",
        "public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>",
        "public intrinsic val <T> State<T>.value: T",
        "public intrinsic var <T> MutableState<T>.value: T",
        "public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T): Unit",
        "public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>",
        "public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>",
        "public intrinsic fun <T> WeakReference<T>.get(): (&T)?",
        "public intrinsic compose fun SideEffect(effect: attached () -> Unit): Unit",
    ] {
        assert!(
            texts.contains(expected),
            "{} is missing from {:?}",
            expected,
            texts
        );
    }
    assert_eq!(texts.len(), 21, "{:?}", texts);
}

/// `ui-toolkit` task 11.1: `prism-ui`'s mirrors of `Modifier`, `ModifierElements` and
/// `ModifierElement` read the offsets `prism dump-layout` gives the declarations of
/// `/prism/compose/ui`.
#[test]
fn the_modifier_mirrors_match_the_layout_of_the_ui_module() {
    let directory =
        std::env::temp_dir().join(format!("prism-modifier-layout-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("main.prism");
    std::fs::write(
        &entry,
        "import /prism/compose/ui\nfun count(m: Modifier, e: ModifierElement): Int = m.count\n",
    )
    .unwrap();
    let layout = prism_cli::dump_layout(&entry)
        .unwrap_or_else(|error| panic!("no layout:\n{}", error.render()));
    std::fs::remove_dir_all(&directory).unwrap();
    for (line, expected) in [
        ("struct prism/compose/ui::Modifier", "(size 648, align 8)"),
        (
            "struct prism/compose/ui::ModifierElements",
            "(size 320, align 8)",
        ),
        (
            "enum prism/compose/ui::ModifierElement",
            "(size 40, align 8, tag u8 at 0)",
        ),
    ] {
        assert!(
            layout.contains(&format!("{} : Copy {}", line, expected)),
            "{} {} in:\n{}",
            line,
            expected,
            layout
        );
    }
    assert!(layout.contains("  first: ModifierElements at 8\n  second: ModifierElements at 328\n"));
    let element = layout
        .split("enum prism/compose/ui::ModifierElement ")
        .nth(1)
        .expect("the element enum");
    let variants: Vec<&str> = element
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(str::trim)
        .collect();
    assert_eq!(variants.len(), prism_ui::modifier::PAYLOAD_OFFSETS.len());
    for (variant, (name, offsets)) in variants.iter().zip(prism_ui::modifier::PAYLOAD_OFFSETS) {
        let printed: Vec<usize> = variant
            .split(" at ")
            .skip(1)
            .map(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap()
            })
            .collect();
        assert!(variant.starts_with(name), "{} is {}", variant, name);
        assert_eq!(printed, offsets, "the payloads of {}", variant);
    }
}

/// standard-library: "The public signature of runApp" (`ui-toolkit`): the ABI of
/// `/prism/compose/ui` exports `runApp` with a default for each of its first three parameters,
/// `HandlerId` and the unit properties, and none of the `package` node layer.
#[test]
fn the_public_signature_of_run_app() {
    use prism_abi::ExportedSymbol;
    use prism_ast::Visibility;
    let library = Library::bundled();
    let source = &library
        .module("prism/compose/ui")
        .expect("the module")
        .source;
    let file = prism_parser::parse_source(source).expect("the module parses");
    let no_definition = |_: &str, _: &str| None;
    let abi = prism_abi::FileAbi::extract_with_origin(
        &file,
        "prism/compose/ui",
        &prism_abi::no_imports,
        &no_definition,
        true,
    )
    .expect("the module's ABI");
    let visibility = |symbol: &ExportedSymbol| match symbol {
        ExportedSymbol::Function { visibility, .. }
        | ExportedSymbol::Struct { visibility, .. }
        | ExportedSymbol::Enum { visibility, .. }
        | ExportedSymbol::Var { visibility, .. }
        | ExportedSymbol::ExtensionProperty { visibility, .. } => *visibility,
    };
    let run_app = abi
        .symbols
        .iter()
        .find_map(|symbol| match symbol {
            ExportedSymbol::Function { name, params, .. } if name == "runApp" => {
                Some((symbol, params))
            }
            _ => None,
        })
        .expect("runApp is exported");
    assert_eq!(
        run_app.0.declaration_text().unwrap(),
        "public intrinsic fun runApp(title: &String, width: Int, height: Int, content: attached compose () -> Unit): Unit"
    );
    let defaults: Vec<bool> = run_app.1.iter().map(|p| p.default.is_some()).collect();
    assert_eq!(defaults, [true, true, true, false]);
    let public: Vec<String> = abi
        .symbols
        .iter()
        .filter(|symbol| visibility(symbol) == Visibility::Public)
        .filter_map(ExportedSymbol::declaration_text)
        .collect();
    for expected in [
        "public intrinsic val Int.dp: Dp",
        "public intrinsic val Int.sp: TextUnit",
        "public intrinsic struct HandlerId : Copy",
    ] {
        assert!(
            public.iter().any(|text| text == expected),
            "{} in {:?}",
            expected,
            public
        );
    }
    for hidden in ["NodeId", "Node", "LeafNode"] {
        let named = |symbol: &&ExportedSymbol| match symbol {
            ExportedSymbol::Struct { name, .. } | ExportedSymbol::Function { name, .. } => {
                name == hidden
            }
            _ => false,
        };
        assert!(
            abi.symbols
                .iter()
                .filter(named)
                .all(|symbol| visibility(symbol) != Visibility::Public),
            "{} is not public",
            hidden
        );
    }
}

/// ui: "The toolkit's handler parameters in the ABI" (`owned-state`, D9): the handlers of the
/// toolkit's clickable, toggleable and editable components are `attached`.
#[test]
fn the_toolkit_s_handler_parameters_in_the_abi() {
    let abis = prism_cli::library_abis(&["prism/compose/material3", "prism/compose/foundation"])
        .unwrap_or_else(|error| panic!("{}", error.render()));
    assert_eq!(abis.len(), 2);
    let texts: Vec<String> = abis
        .iter()
        .flat_map(|abi| abi.symbols.iter())
        .filter_map(prism_abi::ExportedSymbol::declaration_text)
        .collect();
    let declaration = |prefix: &str| -> &String {
        texts
            .iter()
            .find(|text| text.starts_with(prefix))
            .unwrap_or_else(|| panic!("no declaration starting with {} in {:#?}", prefix, texts))
    };
    for (prefix, parameter) in [
        ("public compose fun Button(", "onClick: attached () -> Unit"),
        (
            "public fun Modifier.clickable(",
            "onClick: attached () -> Unit",
        ),
        (
            "public compose fun Checkbox(",
            "onCheckedChange: attached (Bool) -> Unit",
        ),
        (
            "public fun Modifier.toggleable(",
            "onValueChange: attached (Bool) -> Unit",
        ),
        (
            "public compose fun TextField(",
            "onValueChange: attached (String) -> Unit",
        ),
        (
            "public compose fun BasicTextField(",
            "onValueChange: attached (String) -> Unit",
        ),
    ] {
        let text = declaration(prefix);
        assert!(text.contains(parameter), "{} lacks {}", text, parameter);
    }
}

/// standard-library: "listOf is written in Prism" (D10 of `vararg`): an ordinary generic
/// function with a vararg, which the intrinsic catalog no longer lists.
#[test]
fn list_of_is_written_in_prism() {
    let source = MODULES
        .iter()
        .find(|(path, _)| *path == "prism/collections")
        .map(|(_, source)| *source)
        .expect("the collections module");
    assert!(
        source.contains("\npublic fun <T> listOf(vararg elements: T): List<T> = elements\n"),
        "collections declares listOf in Prism"
    );
    assert!(prism_typechecker::intrinsics::CATALOG
        .iter()
        .all(|intrinsic| intrinsic.name != "listOf"));
}
