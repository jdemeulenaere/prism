#[cfg(test)]
mod tests {
    use prism_abi::{
        no_imports, AbiError, AbiType, ExportedSymbol, FileAbi, NominalHeader, NominalKind,
    };
    use prism_parser::parse_source;

    #[test]
    fn test_private_symbols_excluded_from_abi() {
        let src1 = r#"
            package demo.math

            // Private function
            fun internalSecret(): Int = 42

            public fun add(a: Int, b: Int): Int {
                return a + b
            }
        "#;

        let src2 = r#"
            package demo.math

            // Private function changed completely!
            fun differentSecret(x: String): String = x

            public fun add(a: Int, b: Int): Int {
                return a + b
            }
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        // The private changes MUST NOT alter the exported ABI hash!
        assert_eq!(abi1.symbols.len(), 1);
        assert_eq!(abi2.symbols.len(), 1);
        assert_eq!(abi1.abi_hash, abi2.abi_hash);
        assert_eq!(abi1.abi_hash_hex(), abi2.abi_hash_hex());
    }

    #[test]
    fn test_body_changes_do_not_change_abi_hash() {
        let src1 = r#"
            package demo.calc

            public fun compute(x: Int): Int {
                val a = x * 2
                return a + 1
            }
        "#;

        let src2 = r#"
            package demo.calc

            public fun compute(x: Int): Int {
                // Completely rewritten implementation!
                return x + x + 1
            }
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        // Public non-generic body edits produce zero ABI change (Compilation avoidance!)
        assert_eq!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_signature_changes_alter_abi_hash() {
        let src1 = r#"
            package demo.calc

            public fun compute(x: Int): Int = x + 1
        "#;

        let src2 = r#"
            package demo.calc

            // Signature changed from Int to Double
            public fun compute(x: Double): Double = x + 1.0
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        assert_ne!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_missing_explicit_return_type_triggers_abi_error() {
        let src = r#"
            package demo.bad

            // Missing explicit return type on public expression body!
            public fun add(a: Int, b: Int) = a + b
        "#;

        let ast = parse_source(src).expect("Parse");
        let err = FileAbi::extract(&ast, "test/file", &no_imports).unwrap_err();

        match err {
            AbiError::MissingExplicitReturnType {
                function_name,
                visibility,
                ..
            } => {
                assert_eq!(function_name, "add");
                assert_eq!(visibility, "public");
            }
            _ => panic!("Expected MissingExplicitReturnType"),
        }
    }

    #[test]
    fn test_enum_variant_shape_collision_regression() {
        let src1 = r#"
            public enum E {
                A(B, C)
                D(E)
            }
        "#;

        let src2 = r#"
            public enum E {
                A(B)
                C(D, E)
            }
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        // The two enums have different memory layouts and variant shapes.
        // Their ABI hashes MUST be distinct!
        assert_ne!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_generic_body_whitespace_and_comment_independence() {
        let src1 = r#"
            public fun <T> id(x: T): T {
                return x
            }
        "#;

        let src2 = r#"
            // Added leading comments and blank lines

            public fun <T> id(x: T): T {
                // Inner comment
                return x
            }
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        // Source Span line/column shifts MUST NOT alter generic body monomorphization hash!
        assert_eq!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_default_param_value_change_alters_abi_hash() {
        let src1 = r#"
            public fun foo(x: Int = 0): Int = x
        "#;

        let src2 = r#"
            public fun foo(x: Int = 100): Int = x
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        assert_ne!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_default_value_pattern_and_guard_changes_alter_abi_hash() {
        let hash = |src: &str| {
            let ast = parse_source(src).expect("Parse");
            FileAbi::extract(&ast, "test/file", &no_imports)
                .expect("ABI")
                .abi_hash
        };
        let base = "public fun foo(x: Int = when (1) { val n -> n }): Int = x\n";
        let renamed = "public fun foo(x: Int = when (1) { val m -> m }): Int = x\n";
        let guarded =
            "public fun foo(x: Int = when (1) { val n if n > 0 -> n; else -> 0 }): Int = x\n";
        let moved = "public fun foo(x: Int =\n    when (1) { val n -> n }): Int = x\n";
        assert_ne!(hash(base), hash(renamed));
        assert_ne!(hash(base), hash(guarded));
        assert_eq!(hash(base), hash(moved));
    }

    #[test]
    fn test_function_reordering_independence() {
        let src1 = r#"
            public fun first(): Int = 1
            public fun second(): Int = 2
        "#;

        let src2 = r#"
            public fun second(): Int = 2
            public fun first(): Int = 1
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file", &no_imports).expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file", &no_imports).expect("ABI 2");

        // Independent symbol reordering within a file should not invalidate the exported ABI
        assert_eq!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_composable_function_type_in_exported_signature() {
        let compose_src = "public fun Column(content: compose () -> Unit): Unit { }";
        let plain_src = "public fun Column(content: () -> Unit): Unit { }";

        let compose_abi = FileAbi::extract(
            &parse_source(compose_src).unwrap(),
            "test/file",
            &no_imports,
        )
        .unwrap();
        let plain_abi =
            FileAbi::extract(&parse_source(plain_src).unwrap(), "test/file", &no_imports).unwrap();

        let prism_abi::ExportedSymbol::Function { params, .. } = &compose_abi.symbols[0] else {
            panic!("Expected exported function");
        };
        assert_eq!(params[0].1.to_string(), "compose () -> Unit");
        // The compose flag is part of the call contract, so it is part of the ABI.
        assert_ne!(compose_abi.abi_hash, plain_abi.abi_hash);
    }

    fn extract(src: &str) -> FileAbi {
        FileAbi::extract(&parse_source(src).unwrap(), "test/file", &no_imports).unwrap()
    }

    fn first_param(abi: &FileAbi) -> &AbiType {
        let ExportedSymbol::Function { params, .. } = &abi.symbols[0] else {
            panic!("Expected exported function");
        };
        &params[0].1
    }

    #[test]
    fn test_higher_order_parameter_type_recorded_structurally() {
        let abi = extract("public fun hof(f: ((Int) -> Int) -> Int): Int = 1\n");
        let AbiType::Fn { params, ret, .. } = first_param(&abi) else {
            panic!("Expected a function type");
        };
        assert!(matches!(&params[0], AbiType::Fn { params, .. } if params.len() == 1));
        assert_eq!(params[0].to_string(), "(Int) -> Int");
        assert_eq!(ret.to_string(), "Int");
    }

    #[test]
    fn test_nullable_return_distinguished_from_nullable_function_type() {
        let returns_nullable = extract("public fun pick(f: (Int) -> Int?): Int = 1\n");
        let nullable_fn = extract("public fun pick(f: ((Int) -> Int)?): Int = 1\n");
        assert!(
            matches!(first_param(&returns_nullable), AbiType::Fn { ret, .. } if matches!(**ret, AbiType::Nullable(_)))
        );
        assert!(
            matches!(first_param(&nullable_fn), AbiType::Nullable(inner) if matches!(**inner, AbiType::Fn { .. }))
        );
        assert_ne!(returns_nullable.abi_hash, nullable_fn.abi_hash);
    }

    #[test]
    fn test_locally_declared_type_bound_to_the_file_module() {
        let abi = FileAbi::extract(
            &parse_source("package a\npublic struct User {\n    val name: String\n}\npublic struct Order {\n    val user: User\n}\n").unwrap(),
            "a/lib",
            &no_imports,
        )
        .unwrap();
        let order = abi
            .symbols
            .iter()
            .find_map(|s| match s {
                ExportedSymbol::Struct { name, fields, .. } if name == "Order" => Some(fields),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            order[0].2,
            AbiType::Nominal {
                kind: NominalKind::Struct,
                marker: None,
                marker_parameters: Vec::new(),
                module: "a/lib".to_string(),
                name: "User".to_string(),
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn test_imported_type_bound_to_the_exporting_module() {
        let file = parse_source("package a\npublic fun first(): Item = Item(1)\n").unwrap();
        let bind = |name: &str| (name == "Item").then(|| (STRUCT, "model/types".to_string()));
        let abi = FileAbi::extract(&file, "a/svc", &bind).unwrap();
        let ExportedSymbol::Function { return_type, .. } = &abi.symbols[0] else {
            panic!("Expected exported function");
        };
        assert!(matches!(return_type, AbiType::Nominal { module, .. } if module == "model/types"));
        // Binding to another module changes the ABI, so it changes the hash.
        let unbound = FileAbi::extract(&file, "a/svc", &no_imports).unwrap();
        assert!(matches!(
            &unbound.symbols[0],
            ExportedSymbol::Function {
                return_type: AbiType::Unbound { .. },
                ..
            }
        ));
        assert_ne!(abi.abi_hash, unbound.abi_hash);
    }

    #[test]
    fn test_binding_does_not_depend_on_function_bodies() {
        let file1 = parse_source("package a\npublic fun first(): Item = Item(1)\n").unwrap();
        let file2 =
            parse_source("package a\npublic fun first(): Item = Item(2)\nfun helper() = 3\n")
                .unwrap();
        let bind = |name: &str| (name == "Item").then(|| (STRUCT, "model/types".to_string()));
        assert_eq!(
            FileAbi::extract(&file1, "a/svc", &bind).unwrap().abi_hash,
            FileAbi::extract(&file2, "a/svc", &bind).unwrap().abi_hash
        );
    }

    #[test]
    fn test_unknown_type_name_recorded_unbound() {
        let abi = extract("public fun f(x: Mystery): Int = 1\n");
        assert!(matches!(first_param(&abi), AbiType::Unbound { name, .. } if name == "Mystery"));
    }

    #[test]
    fn test_type_definitions_bind_private_imported_and_unknown_types() {
        use prism_abi::{type_definitions, AbiTypeMembers};
        let file = parse_source(
            "package lib\n\
             struct Secret : Copy { val k: Int }\n\
             public struct Outer { val s: Secret, val v: Vec, val m: Mystery }\n\
             enum Choice<T> { Some(T), None }\n",
        )
        .unwrap();
        let bind = |name: &str| (name == "Vec").then(|| (STRUCT, "geo/core".to_string()));
        let definitions = type_definitions(&file, "lib/a", &bind);
        let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Secret", "Outer", "Choice"]);
        assert!(definitions.iter().all(|d| d.module == "lib/a"));
        let AbiTypeMembers::Fields(fields) = &definitions[1].members else {
            panic!("Outer is a struct");
        };
        let module_of = |ty: &AbiType| match ty {
            AbiType::Nominal { module, .. } => Some(module.clone()),
            _ => None,
        };
        assert_eq!(module_of(&fields[0].2).as_deref(), Some("lib/a"));
        assert_eq!(module_of(&fields[1].2).as_deref(), Some("geo/core"));
        assert!(matches!(&fields[2].2, AbiType::Unbound { name, .. } if name == "Mystery"));
        assert_eq!(definitions[0].marker, Some(prism_ast::TypeMarker::Copy));
        let AbiTypeMembers::Variants(variants) = &definitions[2].members else {
            panic!("Choice is an enum");
        };
        assert!(matches!(
            variants[0].1[0],
            AbiType::Parameter { index: 0, .. }
        ));
        assert_eq!(definitions[2].kind(), NominalKind::Enum);
    }

    const STRUCT: NominalHeader = NominalHeader {
        kind: NominalKind::Struct,
        marker: None,
        marker_parameters: Vec::new(),
    };
}
