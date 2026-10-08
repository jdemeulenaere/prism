#[cfg(test)]
mod tests {
    use prism_ast::*;
    use prism_parser::parse_source;

    #[test]
    fn test_parse_math_sample() {
        let src = r#"
            package demo.math

            import /core/fmt

            fun square(x: Int): Int = x * x

            package fun internalHelper(x: Int): Int = square(x) + 1

            public fun hypotenuse(a: Double, b: Double): Double {
                val a2 = a * a
                val b2 = b * b
                return (a2 + b2).sqrt()
            }

            public fun describe(value: Int): String = when (value) {
                0 -> "zero"
                1, 2, 3 -> "small number"
                in 4..10 -> "medium number"
                else -> "large number"
            }
        "#;

        let file = parse_source(src).expect("Failed to parse math sample");

        // Verify package
        assert_eq!(file.package.unwrap().package_string(), "demo.math");

        // Verify imports
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].path_string(), "/core/fmt");
        assert!(file.imports[0].is_root);

        // Verify declarations
        assert_eq!(file.declarations.len(), 4);

        // 1. fun square - private by default!
        match &file.declarations[0] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "square");
                assert_eq!(f.visibility, Visibility::Private);
                assert!(!f.is_compose);
            }
            _ => panic!("Expected function"),
        }

        // 2. package fun internalHelper
        match &file.declarations[1] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "internalHelper");
                assert_eq!(f.visibility, Visibility::Package);
            }
            _ => panic!("Expected function"),
        }

        // 3. public fun hypotenuse
        match &file.declarations[2] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "hypotenuse");
                assert_eq!(f.visibility, Visibility::Public);
                assert_eq!(f.params.len(), 2);
            }
            _ => panic!("Expected function"),
        }

        // 4. public fun describe with when expression
        match &file.declarations[3] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "describe");
                assert_eq!(f.visibility, Visibility::Public);
                if let Some(FunctionBody::Expr(e)) = &f.body {
                    if let ExprKind::When { branches, .. } = &e.kind {
                        assert_eq!(branches.len(), 4);
                    } else {
                        panic!("Expected When expression");
                    }
                } else {
                    panic!("Expected Expr body");
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_parse_structs_enums_and_borrows() {
        let src = r#"
            package demo.models

            public struct Point {
                val x: Double
                val y: Double
            }

            public struct User {
                val id: UInt64
                var name: String
                val email: String?
            }

            public fun updateName(user: &mut User, newName: String) {
                user.name = newName
            }

            public enum Expr {
                Literal(Int)
                Add(Heap<Expr>, Heap<Expr>)
                Multiply(Heap<Expr>, Heap<Expr>)
            }
        "#;

        let file = parse_source(src).expect("Failed to parse models sample");
        assert_eq!(file.declarations.len(), 4);

        // User struct
        match &file.declarations[1] {
            Declaration::Struct(s) => {
                assert_eq!(s.name, "User");
                assert_eq!(s.visibility, Visibility::Public);
                assert_eq!(s.fields.len(), 3);
                assert_eq!(s.fields[2].name, "email");
                assert_eq!(s.fields[2].ty.to_string(), "String?");
            }
            _ => panic!("Expected User struct"),
        }

        // updateName borrow parameter
        match &file.declarations[2] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "updateName");
                assert_eq!(f.params[0].ty.to_string(), "&mut User");
            }
            _ => panic!("Expected updateName function"),
        }

        // Enum with Heap<Expr>
        match &file.declarations[3] {
            Declaration::Enum(e) => {
                assert_eq!(e.name, "Expr");
                assert_eq!(e.visibility, Visibility::Public);
                assert_eq!(e.variants.len(), 3);
                assert_eq!(e.variants[1].name, "Add");
                assert_eq!(e.variants[1].fields.len(), 2);
                assert_eq!(e.variants[1].fields[0].to_string(), "Heap<Expr>");
            }
            _ => panic!("Expected Expr enum"),
        }
    }

    #[test]
    fn test_parse_compose_function() {
        let src = r#"
            public compose fun CounterView(initialCount: Int = 0) {
                val count = remember(initialCount)
            }
        "#;

        let file = parse_source(src).expect("Failed to parse compose function");
        assert_eq!(file.declarations.len(), 1);

        match &file.declarations[0] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "CounterView");
                assert_eq!(f.visibility, Visibility::Public);
                assert!(f.is_compose);
                assert_eq!(f.params.len(), 1);
                assert!(f.params[0].default_value.is_some());
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_elvis_and_safe_nav() {
        let src = r#"
            fun getLen(name: String?): Int = name?.length ?: 0
        "#;

        let file = parse_source(src).expect("Failed to parse elvis and safe nav");
        match &file.declarations[0] {
            Declaration::Function(f) => {
                assert_eq!(f.name, "getLen");
                assert_eq!(f.visibility, Visibility::Private);
            }
            _ => panic!("Expected function"),
        }
    }
}
