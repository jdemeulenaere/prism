#[cfg(test)]
mod tests {
    use prism_ast::*;

    #[test]
    fn test_default_visibility_is_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    #[test]
    fn test_type_construction() {
        let dummy = Span::dummy();
        let int_ty = Type::named("Int", dummy);
        let nullable_str = Type::nullable(Type::named("String", dummy), dummy);
        let mut_user = Type::reference_mut(Type::named("User", dummy), dummy);
        let heap_expr = Type::generic("Heap", vec![Type::named("Expr", dummy)], dummy);

        assert_eq!(int_ty.to_string(), "Int");
        assert_eq!(nullable_str.to_string(), "String?");
        assert_eq!(mut_user.to_string(), "&mut User");
        assert_eq!(heap_expr.to_string(), "Heap<Expr>");
    }

    #[test]
    fn test_package_decl() {
        let dummy = Span::dummy();
        let pkg = PackageDecl {
            path: vec!["demo".to_string(), "math".to_string()],
            span: dummy,
        };
        assert_eq!(pkg.package_string(), "demo.math");
    }

    #[test]
    fn test_import_decl() {
        let dummy = Span::dummy();
        let root_import = ImportDecl {
            is_root: true,
            path: vec!["core".to_string(), "fmt".to_string()],
            span: dummy,
        };
        let sibling_import = ImportDecl {
            is_root: false,
            path: vec!["doo".to_string()],
            span: dummy,
        };
        assert_eq!(root_import.path_string(), "/core/fmt");
        assert_eq!(sibling_import.path_string(), "doo");
    }
}
