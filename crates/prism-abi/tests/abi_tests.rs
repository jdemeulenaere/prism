#[cfg(test)]
mod tests {
    use prism_abi::{AbiError, FileAbi};
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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

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
        let err = FileAbi::extract(&ast, "test/file").unwrap_err();

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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

        // The two enums have different memory layouts and variant shapes.
        // Their ABI hashes MUST be distinct!
        assert_ne!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_generic_body_whitespace_and_comment_independence() {
        let src1 = r#"
            public fun id<T>(x: T): T {
                return x
            }
        "#;

        let src2 = r#"
            // Added leading comments and blank lines

            public fun id<T>(x: T): T {
                // Inner comment
                return x
            }
        "#;

        let ast1 = parse_source(src1).expect("Parse 1");
        let ast2 = parse_source(src2).expect("Parse 2");

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

        assert_ne!(abi1.abi_hash, abi2.abi_hash);
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

        let abi1 = FileAbi::extract(&ast1, "test/file").expect("ABI 1");
        let abi2 = FileAbi::extract(&ast2, "test/file").expect("ABI 2");

        // Independent symbol reordering within a file should not invalidate the exported ABI
        assert_eq!(abi1.abi_hash, abi2.abi_hash);
    }

    #[test]
    fn test_composable_function_type_in_exported_signature() {
        let compose_src = "public fun Column(content: compose () -> Unit): Unit { }";
        let plain_src = "public fun Column(content: () -> Unit): Unit { }";

        let compose_abi =
            FileAbi::extract(&parse_source(compose_src).unwrap(), "test/file").unwrap();
        let plain_abi = FileAbi::extract(&parse_source(plain_src).unwrap(), "test/file").unwrap();

        let prism_abi::ExportedSymbol::Function { params, .. } = &compose_abi.symbols[0] else {
            panic!("Expected exported function");
        };
        assert_eq!(params[0].1, "compose () -> Unit");
        // The compose flag is part of the call contract, so it is part of the ABI.
        assert_ne!(compose_abi.abi_hash, plain_abi.abi_hash);
    }
}
