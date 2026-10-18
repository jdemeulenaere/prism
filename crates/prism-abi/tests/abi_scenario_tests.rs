//! Scenario coverage for `openspec/specs/compilation-and-abi`: every form of a scenario's WHEN
//! that `abi_tests.rs` does not already pin down. Hash scenarios live here, not in the
//! data-driven fixtures of `prism-cli`, because they compare two extractions.

use prism_abi::{declared_types, no_imports, AbiError, AbiType, ExportedSymbol, FileAbi};
use prism_ast::{ReceiverMode, TypeMarker, Visibility};
use prism_parser::parse_source;

fn extract(src: &str) -> FileAbi {
    FileAbi::extract(
        &parse_source(src).expect("source must parse"),
        "test/file",
        &no_imports,
    )
    .expect("ABI must extract")
}

fn extract_err(src: &str) -> AbiError {
    FileAbi::extract(
        &parse_source(src).expect("source must parse"),
        "test/file",
        &no_imports,
    )
    .expect_err("ABI extraction must fail")
}

fn assert_hash_differs(before: &str, after: &str, what: &str) {
    assert_ne!(
        extract(before).abi_hash,
        extract(after).abi_hash,
        "{} must change the ABI hash",
        what
    );
}

fn assert_hash_equal(before: &str, after: &str, what: &str) {
    assert_eq!(
        extract(before).abi_hash,
        extract(after).abi_hash,
        "{} must not change the ABI hash",
        what
    );
}

// --- Phase 1 surface extraction

#[test]
fn surface_extraction_records_package_and_every_exported_kind() {
    let abi = extract(
        r#"
        package demo.math

        public struct Point {
            val x: Double
            var y: Double
        }

        package enum Shape {
            Dot
            Circle(Double)
        }

        public fun area(p: Point, scale: Int = 2): Double = 1.0

        package fun helper(): Int = 1

        public val origin: Point = Point(0.0, 0.0)
        "#,
    );

    // `Double` is an alias of `Float`, so the ABI records `Float`.
    assert_eq!(abi.package.as_deref(), Some("demo.math"));
    assert_eq!(abi.symbols.len(), 5);

    let mut seen = Vec::new();
    for symbol in &abi.symbols {
        match symbol {
            ExportedSymbol::Struct {
                visibility,
                name,
                fields,
                ..
            } => {
                assert_eq!((name.as_str(), *visibility), ("Point", Visibility::Public));
                let shapes: Vec<_> = fields
                    .iter()
                    .map(|(n, is_var, ty)| (n.as_str(), *is_var, ty.to_string()))
                    .collect();
                assert_eq!(
                    shapes,
                    vec![
                        ("x", false, "Float".to_string()),
                        ("y", true, "Float".to_string())
                    ]
                );
                seen.push("struct");
            }
            ExportedSymbol::Enum {
                visibility,
                name,
                variants,
                ..
            } => {
                assert_eq!((name.as_str(), *visibility), ("Shape", Visibility::Package));
                let shapes: Vec<_> = variants
                    .iter()
                    .map(|(n, payload)| {
                        (
                            n.as_str(),
                            payload.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                        )
                    })
                    .collect();
                assert_eq!(
                    shapes,
                    vec![("Dot", vec![]), ("Circle", vec!["Float".to_string()])]
                );
                seen.push("enum");
            }
            ExportedSymbol::Function {
                visibility,
                name,
                params,
                return_type,
                ..
            } if name == "area" => {
                assert_eq!(*visibility, Visibility::Public);
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].0, "p");
                assert_eq!(params[1].1.to_string(), "Int");
                assert!(params[1].2.is_some(), "the default value is recorded");
                assert_eq!(return_type.to_string(), "Float");
                seen.push("area");
            }
            ExportedSymbol::Function {
                visibility, name, ..
            } => {
                assert_eq!(
                    (name.as_str(), *visibility),
                    ("helper", Visibility::Package)
                );
                seen.push("helper");
            }
            ExportedSymbol::Var {
                visibility, name, ..
            } => {
                assert_eq!((name.as_str(), *visibility), ("origin", Visibility::Public));
                seen.push("var");
            }
        }
    }
    seen.sort();
    assert_eq!(seen, vec!["area", "enum", "helper", "struct", "var"]);
}

#[test]
fn surface_extraction_does_not_typecheck_bodies() {
    // The body is ill-typed and mentions an unknown name; Phase 1 never reads it.
    let abi = extract("public fun broken(): Int = missing + \"s\"\n");
    assert_eq!(abi.symbols.len(), 1);
}

// --- Private symbols excluded

#[test]
fn private_symbols_of_every_kind_are_excluded() {
    let with_private = extract(
        r#"
        package demo
        struct HiddenStruct { val a: Int }
        enum HiddenEnum { A, B }
        val hiddenVar: Int = 1
        fun hiddenFun(): Int = 1
        "#,
    );
    assert!(with_private.symbols.is_empty());
    assert_eq!(
        with_private.abi_hash,
        extract("package demo\n").abi_hash,
        "private declarations of any kind leave the hash of an empty surface"
    );
}

#[test]
fn private_struct_and_enum_changes_do_not_alter_the_hash() {
    assert_hash_equal(
        "package demo\npublic fun f(): Int = 1\nstruct S { val a: Int }\nenum E { A }\n",
        "package demo\npublic fun f(): Int = 1\nstruct S { val a: String, val b: Int }\nenum E { A, B(Int) }\n",
        "changing private structs and enums",
    );
}

// --- Exported function without explicit return type

#[test]
fn exported_expression_body_without_return_type_is_rejected_for_both_visibilities() {
    for (src, visibility) in [
        ("public fun add(a: Int, b: Int) = a + b\n", "public"),
        ("package fun add(a: Int, b: Int) = a + b\n", "package"),
    ] {
        match extract_err(src) {
            AbiError::MissingExplicitReturnType {
                function_name,
                visibility: found,
                ..
            } => {
                assert_eq!(function_name, "add");
                assert_eq!(found, visibility);
            }
            other => panic!("expected MissingExplicitReturnType, got {:?}", other),
        }
    }
}

#[test]
fn private_expression_body_without_return_type_is_accepted() {
    assert!(extract("fun add(a: Int, b: Int) = a + b\n")
        .symbols
        .is_empty());
}

#[test]
fn exported_variable_without_type_is_rejected() {
    match extract_err("package fun helper(): Int = 1\npublic val limit = 10\n") {
        AbiError::MissingExplicitVarType {
            var_name,
            visibility,
            ..
        } => {
            assert_eq!(var_name, "limit");
            assert_eq!(visibility, "public");
        }
        other => panic!("expected MissingExplicitVarType, got {:?}", other),
    }
}

// --- Modifying a public function signature, one component at a time

#[test]
fn changing_only_a_parameter_type_changes_the_hash() {
    assert_hash_differs(
        "public fun f(x: Int): Int = 1\n",
        "public fun f(x: String): Int = 1\n",
        "a parameter type",
    );
}

#[test]
fn changing_only_the_return_type_changes_the_hash() {
    assert_hash_differs(
        "public fun f(x: Int): Int = 1\n",
        "public fun f(x: Int): Double = 1.0\n",
        "the return type",
    );
}

#[test]
fn adding_or_removing_a_parameter_changes_the_hash() {
    assert_hash_differs(
        "public fun f(x: Int): Int = 1\n",
        "public fun f(x: Int, y: Int): Int = 1\n",
        "adding a parameter",
    );
}

#[test]
fn making_a_return_type_nullable_changes_the_hash() {
    assert_hash_differs(
        "public fun f(): Int = 1\n",
        "public fun f(): Int? = 1\n",
        "a nullable return type",
    );
}

#[test]
fn changing_the_visibility_of_an_export_changes_the_hash() {
    assert_hash_differs(
        "public fun f(): Int = 1\n",
        "package fun f(): Int = 1\n",
        "public to package",
    );
}

#[test]
fn changing_a_struct_field_type_or_an_enum_payload_changes_the_hash() {
    assert_hash_differs(
        "public struct S { val a: Int }\n",
        "public struct S { val a: String }\n",
        "a struct field type",
    );
    assert_hash_differs(
        "public enum E { A(Int) }\n",
        "public enum E { A(String) }\n",
        "an enum payload type",
    );
}

// --- Private helpers

#[test]
fn adding_removing_or_editing_a_private_helper_leaves_the_hash() {
    let base = "public fun f(): Int = 1\n";
    assert_hash_equal(
        base,
        "public fun f(): Int = 1\nfun helper(): Int = 2\n",
        "adding a private helper",
    );
    assert_hash_equal(
        "public fun f(): Int = 1\nfun helper(): Int = 2\n",
        "public fun f(): Int = 1\nfun helper(x: String): String = x\n",
        "rewriting a private helper",
    );
    assert_hash_equal(
        "public fun f(): Int = 1\nfun helper(): Int = 2\n",
        base,
        "removing a private helper",
    );
}

// --- Reordering

#[test]
fn reordering_declarations_of_mixed_kinds_leaves_the_hash() {
    assert_hash_equal(
        "package demo\npublic struct S { val a: Int }\npublic enum E { A, B }\npublic val v: Int = 1\npublic fun f(): Int = 1\n",
        "package demo\npublic fun f(): Int = 1\npublic val v: Int = 1\npublic enum E { A, B }\npublic struct S { val a: Int }\n",
        "reordering structs, enums, variables and functions",
    );
}

#[test]
fn reordering_private_declarations_among_exports_leaves_the_hash() {
    assert_hash_equal(
        "fun p(): Int = 1\npublic fun f(): Int = 1\npublic fun g(): Int = 2\n",
        "public fun g(): Int = 2\nfun p(): Int = 1\npublic fun f(): Int = 1\n",
        "moving private declarations between exports",
    );
}

// --- Generic bodies

#[test]
fn changing_a_generic_function_body_changes_the_monomorphization_and_abi_hashes() {
    let before = extract("public fun <T> id(x: T): T {\n    return x\n}\n");
    let after = extract("public fun <T> id(x: T): T {\n    val y = x\n    return y\n}\n");

    let hash_of = |abi: &FileAbi| match &abi.symbols[0] {
        ExportedSymbol::Function {
            generic_body_hash, ..
        } => *generic_body_hash,
        other => panic!("expected a function, got {:?}", other),
    };
    let (b, a) = (hash_of(&before), hash_of(&after));
    assert!(b.is_some() && a.is_some(), "generic bodies record a hash");
    assert_ne!(b, a);
    assert_ne!(before.abi_hash, after.abi_hash);
}

#[test]
fn a_non_generic_function_records_no_monomorphization_hash() {
    let abi = extract("public fun f(x: Int): Int = x\n");
    assert!(matches!(
        &abi.symbols[0],
        ExportedSymbol::Function {
            generic_body_hash: None,
            ..
        }
    ));
}

#[test]
fn generic_body_hash_ignores_comments_blank_lines_and_indentation() {
    assert_hash_equal(
        "public fun <T> id(x: T): T {\n    return x\n}\n",
        "\n\n// leading comment\npublic fun <T> id(x: T): T {\n\n        /* inner */\n        return    x\n\n}\n",
        "comments, blank lines and whitespace in a generic body",
    );
}

// --- Copy and Clone markers

/// Extracts `api` (module `shop/api`), binding type names to the types `models` (module
/// `shop/models`) exports, as Phase 1 does for a direct import.
fn extract_importing(models: &str, api: &str) -> FileAbi {
    let models = parse_source(models).expect("models must parse");
    let exported = declared_types(&models, true);
    let bind = |name: &str| {
        exported
            .get(name)
            .map(|header| (*header, "shop/models".to_string()))
    };
    FileAbi::extract(
        &parse_source(api).expect("api must parse"),
        "shop/api",
        &bind,
    )
    .expect("ABI must extract")
}

fn struct_field_type<'a>(abi: &'a FileAbi, struct_name: &str, field: &str) -> &'a AbiType {
    abi.symbols
        .iter()
        .find_map(|s| match s {
            ExportedSymbol::Struct { name, fields, .. } if name == struct_name => fields
                .iter()
                .find(|(f, _, _)| f == field)
                .map(|(_, _, ty)| ty),
            _ => None,
        })
        .expect("field must exist")
}

#[test]
fn adding_or_changing_a_marker_changes_the_abi_hash() {
    let plain = "public struct Point { val x: Int }\n";
    let clone = "public struct Point : Clone { val x: Int }\n";
    let copy = "public struct Point : Copy { val x: Int }\n";
    assert_hash_differs(plain, clone, "adding ': Clone'");
    assert_hash_differs(clone, copy, "changing ': Clone' to ': Copy'");
    assert_hash_differs(
        "public enum Color { Red }\n",
        "public enum Color : Copy { Red }\n",
        "adding ': Copy' to an enum",
    );
    assert!(matches!(
        &extract(copy).symbols[0],
        ExportedSymbol::Struct {
            marker: Some(TypeMarker::Copy),
            ..
        }
    ));
    assert!(matches!(
        &extract("public enum Color : Clone { Red }\n").symbols[0],
        ExportedSymbol::Enum {
            marker: Some(TypeMarker::Clone),
            ..
        }
    ));
}

#[test]
fn marker_on_a_private_type() {
    let export = "public fun f(): Int = 1\n";
    assert_hash_equal(
        &format!("{export}struct Secret {{ val x: Int }}\n"),
        &format!("{export}struct Secret : Copy {{ val x: Int }}\n"),
        "a marker on an unmentioned private struct",
    );
    // A private type that an exported signature mentions carries its marker in that reference.
    assert_hash_differs(
        "struct Secret { val x: Int }\npublic fun f(s: Secret): Int = 1\n",
        "struct Secret : Copy { val x: Int }\npublic fun f(s: Secret): Int = 1\n",
        "a marker on a private struct named by an exported signature",
    );
}

const MODELS: &str = "package shop\npublic struct Money : Copy { val cents: Int }\npublic struct Note : Clone { val text: String }\n";
const API: &str =
    "package shop\nimport models\npublic struct Order { val price: Money, val note: Note }\n";

#[test]
fn referenced_type_records_its_marker() {
    let abi = extract_importing(MODELS, API);
    assert!(matches!(
        struct_field_type(&abi, "Order", "price"),
        AbiType::Nominal { marker: Some(TypeMarker::Copy), module, name, .. }
            if module == "shop/models" && name == "Money"
    ));
    assert!(matches!(
        struct_field_type(&abi, "Order", "note"),
        AbiType::Nominal { marker: Some(TypeMarker::Clone), name, .. } if name == "Note"
    ));
}

#[test]
fn changing_a_referenced_types_marker() {
    let models_without_copy = MODELS.replace("Money : Copy", "Money");
    assert_hash_differs(MODELS, &models_without_copy, "removing ': Copy' from Money");
    assert_ne!(
        extract_importing(MODELS, API).abi_hash,
        extract_importing(&models_without_copy, API).abi_hash,
        "the importer's ABI hash must change with a referenced type's marker"
    );
}

#[test]
fn built_in_range_in_an_exported_signature() {
    let abi =
        extract("public struct Span : Copy { val r: Range }\npublic fun f(r: Range): Range = r\n");
    assert_eq!(struct_field_type(&abi, "Span", "r"), &AbiType::Range);
    let user_range =
        extract("public struct Range { val a: Int }\npublic fun f(r: Range): Range = r\n");
    let ExportedSymbol::Function { return_type, .. } = user_range
        .symbols
        .iter()
        .find(|s| matches!(s, ExportedSymbol::Function { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert!(matches!(return_type, AbiType::Nominal { name, .. } if name == "Range"));
}

// --- Extension Functions in the ABI

const USER: &str = "public struct User { var name: String }\npublic struct Team { val id: Int }\n";

/// Extracts `src` as module `models/user`.
fn extract_user_module(src: &str) -> FileAbi {
    FileAbi::extract(
        &parse_source(src).expect("source must parse"),
        "models/user",
        &no_imports,
    )
    .expect("ABI must extract")
}

/// Extracts `ui` (module `ui/ui`), binding type names to the types `models/user` exports.
fn extract_ui(ui: &str) -> FileAbi {
    let models = parse_source(USER).expect("models must parse");
    let exported = declared_types(&models, true);
    let bind = |name: &str| {
        exported
            .get(name)
            .map(|header| (*header, "models/user".to_string()))
    };
    FileAbi::extract(&parse_source(ui).expect("ui must parse"), "ui/ui", &bind)
        .expect("ABI must extract")
}

fn user_record(abi: &FileAbi) -> &ExportedSymbol {
    abi.symbols
        .iter()
        .find(|s| matches!(s, ExportedSymbol::Struct { name, .. } if name == "User"))
        .expect("User is exported")
}

#[test]
fn exported_extension_recorded_with_its_receiver() {
    let abi = extract_user_module(
        "public struct User { var name: String }\npublic mut fun User.rename(n: String): Unit\n",
    );
    let Some(ExportedSymbol::Function {
        receiver: Some(receiver),
        name,
        params,
        return_type,
        ..
    }) = abi
        .symbols
        .iter()
        .find(|s| matches!(s, ExportedSymbol::Function { .. }))
    else {
        panic!("expected an exported extension: {:?}", abi.symbols);
    };
    assert_eq!(name, "rename");
    assert_eq!(params.len(), 1);
    assert_eq!(
        (params[0].0.as_str(), params[0].1.to_string()),
        ("n", "String".to_string())
    );
    assert_eq!(return_type, &AbiType::Unit);
    assert_eq!(receiver.mode, ReceiverMode::Mutable);
    assert!(
        matches!(&receiver.ty, AbiType::Nominal { module, name, .. } if module == "models/user" && name == "User"),
        "{:?}",
        receiver.ty
    );
}

#[test]
fn adding_an_extension_in_another_file() {
    let models_before = extract_user_module(USER);
    let ui_before = extract_ui("import /models/user\npublic fun ping(): Int = 1\n");
    let ui_after = extract_ui(
        "import /models/user\npublic fun ping(): Int = 1\npublic fun User.shout(): String = name\n",
    );
    // The receiver type's file is extracted from its own source alone.
    assert_eq!(models_before.abi_hash, extract_user_module(USER).abi_hash);
    assert_ne!(ui_before.abi_hash, ui_after.abi_hash);
}

#[test]
fn adding_an_extension_in_the_receiver_type_s_file() {
    let before = extract_user_module(USER);
    let after = extract_user_module(&format!("{}public fun User.shout(): String = name\n", USER));
    assert_ne!(before.abi_hash, after.abi_hash);
    assert_eq!(user_record(&before), user_record(&after));
}

#[test]
fn changing_a_receiver_mode_or_type_changes_the_abi_hash() {
    assert_hash_differs(
        &format!("{}public fun User.reset(): Unit {{ }}\n", USER),
        &format!("{}public mut fun User.reset(): Unit {{ }}\n", USER),
        "making a receiver mutable",
    );
    assert_hash_differs(
        &format!("{}public fun User.reset(): Unit {{ }}\n", USER),
        &format!("{}public consuming fun User.reset(): Unit {{ }}\n", USER),
        "making a receiver consuming",
    );
    assert_hash_differs(
        &format!("{}public fun User.size(): Int = 1\n", USER),
        &format!("{}public fun Team.size(): Int = 1\n", USER),
        "changing the receiver type",
    );
    assert_hash_differs(
        &format!("{}public fun size(): Int = 1\n", USER),
        &format!("{}public fun User.size(): Int = 1\n", USER),
        "adding a receiver",
    );
}

#[test]
fn extensions_sharing_a_name() {
    let plain = "public fun describe(): String = \"a\"\n";
    let user = "public fun User.describe(): String = \"b\"\n";
    let team = "public fun Team.describe(): String = \"c\"\n";
    let orders = [
        [plain, user, team],
        [plain, team, user],
        [user, plain, team],
        [user, team, plain],
        [team, plain, user],
        [team, user, plain],
    ];
    let hashes: Vec<u128> = orders
        .iter()
        .map(|order| extract(&format!("{}{}", USER, order.concat())).abi_hash)
        .collect();
    assert!(hashes.iter().all(|h| *h == hashes[0]), "{:?}", hashes);

    let abi = extract(&format!("{}{}", USER, orders[0].concat()));
    let receivers: Vec<Option<String>> = abi
        .symbols
        .iter()
        .filter_map(|s| match s {
            ExportedSymbol::Function { name, receiver, .. } if name == "describe" => {
                Some(receiver.as_ref().map(|r| r.ty.to_string()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        receivers,
        vec![None, Some("Team".to_string()), Some("User".to_string())],
        "three distinct functions, sorted by receiver"
    );
}

#[test]
fn extension_body_or_private_extension_edits() {
    assert_hash_equal(
        &format!("{}public fun User.shout(): String = name\n", USER),
        &format!("{}public fun User.shout(): String = name + \"!\"\n", USER),
        "editing a non-generic extension body",
    );
    assert_hash_equal(
        USER,
        &format!("{}fun User.secret(): Int = 1\n", USER),
        "adding a private extension",
    );
    assert_hash_equal(
        &format!("{}fun User.secret(): Int = 1\n", USER),
        &format!("{}fun User.secret(x: Int): String = \"x\"\n", USER),
        "modifying a private extension",
    );
}
