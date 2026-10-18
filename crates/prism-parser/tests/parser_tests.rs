#[cfg(test)]
mod tests {
    use prism_ast::*;
    use prism_lexer::{Lexer, TokenKind};
    use prism_parser::{parse_source, Parser, MAX_NESTING_DEPTH};

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

    #[test]
    fn test_when_inside_block() {
        let src = r#"
            fun test(): Int {
                val f = {
                    when (1) {
                        1 -> 10
                        else -> 20
                    }
                }
                return 0
            }
        "#;

        let file = parse_source(src).expect("Failed to parse when inside block");
        assert_eq!(file.declarations.len(), 1);
    }

    #[test]
    fn test_general_trailing_lambdas() {
        let src = r#"
            fun render() {
                // Parenthesized call with trailing lambda
                Column(padding = 16) {
                    Text("Hello")
                }

                // Bare call with trailing lambda only
                val state = remember { mutableStateOf(0) }

                // Method chain with trailing lambda
                items.filter { it > 0 }

                // Multi-statement closure with parameters
                items.forEach { item ->
                    val s = item * item
                    println(s)
                }
            }
        "#;

        let file = parse_source(src).expect("Failed to parse trailing lambdas");
        match &file.declarations[0] {
            Declaration::Function(f) => {
                if let Some(FunctionBody::Block(b)) = &f.body {
                    assert_eq!(items(b).len(), 4);

                    // 1. Column(padding = 16) { Text("Hello") }
                    if let ExprKind::Call { callee, args, .. } = &expr_of(&items(b)[0]).kind {
                        assert_eq!(callee.to_string(), "Column");
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[0].name.as_deref(), Some("padding"));
                        // A parameterless trailing lambda is a closure, not a block.
                        assert!(matches!(
                            &args[1].value.kind,
                            ExprKind::Closure { params, body, .. } if params.is_empty() && items(body).len() == 1
                        ));
                    } else {
                        panic!("Expected Call expr for Column");
                    }

                    // 2. val state = remember { mutableStateOf(0) }
                    if let StatementKind::Local { initializer, .. } = &items(b)[1].kind {
                        if let ExprKind::Call { callee, args, .. } = &initializer.kind {
                            assert_eq!(callee.to_string(), "remember");
                            assert_eq!(args.len(), 1);
                        } else {
                            panic!("Expected Call for remember");
                        }
                    } else {
                        panic!("Expected LocalVar for state");
                    }

                    // 4. items.forEach { item -> val s = item * item; println(s) }
                    if let ExprKind::Call { callee, args, .. } = &expr_of(&items(b)[3]).kind {
                        if let ExprKind::MemberAccess { member, .. } = &callee.kind {
                            assert_eq!(member, "forEach");
                        } else {
                            panic!("Expected MemberAccess for forEach");
                        }
                        assert_eq!(args.len(), 1);
                        if let ExprKind::Closure { params, body, .. } = &args[0].value.kind {
                            assert_eq!(names(params), vec!["item"]);
                            assert_eq!(items(body).len(), 2);
                        } else {
                            panic!("Expected Closure in trailing lambda");
                        }
                    } else {
                        panic!("Expected Call expr for forEach");
                    }
                } else {
                    panic!("Expected block body");
                }
            }
            _ => panic!("Expected function"),
        }
    }

    #[test]
    fn test_non_associative_assignment_rejected() {
        let src = r#"
            fun test() {
                a = b = c
            }
        "#;

        let res = parse_source(src);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.message.contains("Chained assignments are not allowed"));
    }

    #[test]
    fn test_operator_precedence_table() {
        // 1. Arithmetic and comparison: a + b * c > d -> ((a + (b * c)) > d)
        let src1 = r#"
            fun test(a: Int, b: Int, c: Int, d: Int): Bool = a + b * c > d
        "#;
        let file1 = parse_source(src1).expect("Failed to parse arithmetic/comparison precedence");
        if let Declaration::Function(f) = &file1.declarations[0] {
            if let Some(FunctionBody::Expr(e)) = &f.body {
                if let ExprKind::Binary { op, left, right: _ } = &e.kind {
                    assert_eq!(*op, BinaryOp::Gt);
                    if let ExprKind::Binary {
                        op: op2,
                        left: _,
                        right: right2,
                    } = &left.kind
                    {
                        assert_eq!(*op2, BinaryOp::Add);
                        if let ExprKind::Binary { op: op3, .. } = &right2.kind {
                            assert_eq!(*op3, BinaryOp::Mul);
                        } else {
                            panic!("Expected b * c");
                        }
                    } else {
                        panic!("Expected a + (b * c)");
                    }
                } else {
                    panic!("Expected > comparison");
                }
            }
        }

        // 2. Elvis tighter than comparison: age ?: 0 >= 18 -> ((age ?: 0) >= 18)
        let src2 = r#"
            fun test(age: Int?): Bool = age ?: 0 >= 18
        "#;
        let file2 = parse_source(src2).expect("Failed to parse elvis vs comparison");
        if let Declaration::Function(f) = &file2.declarations[0] {
            if let Some(FunctionBody::Expr(e)) = &f.body {
                if let ExprKind::Binary { op, left, right: _ } = &e.kind {
                    assert_eq!(*op, BinaryOp::GtEq);
                    if let ExprKind::Binary { op: elvis_op, .. } = &left.kind {
                        assert_eq!(*elvis_op, BinaryOp::Elvis);
                    } else {
                        panic!("Expected (age ?: 0) on left of >=");
                    }
                } else {
                    panic!("Expected >= binary op");
                }
            }
        }

        // 3. Elvis looser than arithmetic: base ?: offset + 10 -> (base ?: (offset + 10))
        let src3 = r#"
            fun test(base: Int?, offset: Int): Int = base ?: offset + 10
        "#;
        let file3 = parse_source(src3).expect("Failed to parse elvis vs sum");
        if let Declaration::Function(f) = &file3.declarations[0] {
            if let Some(FunctionBody::Expr(e)) = &f.body {
                if let ExprKind::Binary { op, left: _, right } = &e.kind {
                    assert_eq!(*op, BinaryOp::Elvis);
                    if let ExprKind::Binary { op: sum_op, .. } = &right.kind {
                        assert_eq!(*sum_op, BinaryOp::Add);
                    } else {
                        panic!("Expected (offset + 10) on right of ?:");
                    }
                } else {
                    panic!("Expected ?: binary op");
                }
            }
        }
    }

    #[test]
    fn test_indexing_operator() {
        let src = r#"
            fun test(list: Vector<Int>): Int {
                return list[0]
            }
        "#;

        let file = parse_source(src).expect("Failed to parse indexing");
        if let Declaration::Function(f) = &file.declarations[0] {
            if let Some(FunctionBody::Block(b)) = &f.body {
                if let ExprKind::Return(Some(val)) = &expr_of(&items(b)[0]).kind {
                    if let ExprKind::Index { target, index } = &val.kind {
                        assert_eq!(target.to_string(), "list");
                        assert_eq!(index.to_string(), "0");
                    } else {
                        panic!("Expected Index expr");
                    }
                }
            }
        }
    }

    #[test]
    fn test_newline_aware_return() {
        // 1. Bare return followed by newline
        let src1 = r#"
            fun test(): Int {
                return
                doWork()
            }
        "#;
        let file1 = parse_source(src1).expect("Failed to parse newline return");
        if let Declaration::Function(f) = &file1.declarations[0] {
            if let Some(FunctionBody::Block(b)) = &f.body {
                assert_eq!(items(b).len(), 2);
                assert!(matches!(expr_of(&items(b)[0]).kind, ExprKind::Return(None)));
                assert!(matches!(expr_of(&items(b)[1]).kind, ExprKind::Call { .. }));
            }
        }

        // 2. Return with expression on same line
        let src2 = r#"
            fun test(): Int {
                return 42
            }
        "#;
        let file2 = parse_source(src2).expect("Failed to parse return with expr");
        if let Declaration::Function(f) = &file2.declarations[0] {
            if let Some(FunctionBody::Block(b)) = &f.body {
                assert_eq!(items(b).len(), 1);
                if let ExprKind::Return(Some(val)) = &expr_of(&items(b)[0]).kind {
                    assert_eq!(val.to_string(), "42");
                } else {
                    panic!("Expected Return(Some(42))");
                }
            }
        }

        // 3. Semicolon-free statement separation
        let src3 = r#"
            fun test() {
                val a = 1
                val b = 2
                val c = 3
            }
        "#;
        let file3 = parse_source(src3).expect("Failed to parse semicolon-free statements");
        if let Declaration::Function(f) = &file3.declarations[0] {
            if let Some(FunctionBody::Block(b)) = &f.body {
                assert_eq!(items(b).len(), 3);
            }
        }
    }

    #[test]
    fn test_lparen_newline_suppression() {
        let src = r#"
            fun test() {
                cleanup()
                (1 + 2).toString()
            }
        "#;

        let file = parse_source(src).expect("Failed to parse newline separated call and grouping");
        if let Declaration::Function(f) = &file.declarations[0] {
            if let Some(FunctionBody::Block(b)) = &f.body {
                assert_eq!(items(b).len(), 2);
                // First stmt is cleanup()
                if let ExprKind::Call { callee, args, .. } = &expr_of(&items(b)[0]).kind {
                    assert_eq!(callee.to_string(), "cleanup");
                    assert_eq!(args.len(), 0);
                } else {
                    panic!("Expected cleanup() call");
                }
                // Second stmt is (1 + 2).toString()
                if let ExprKind::Call { callee, .. } = &expr_of(&items(b)[1]).kind {
                    if let ExprKind::MemberAccess { target, member, .. } = &callee.kind {
                        assert_eq!(member, "toString");
                        assert!(matches!(target.kind, ExprKind::Binary { .. }));
                    } else {
                        panic!("Expected MemberAccess for toString");
                    }
                } else {
                    panic!("Expected Call for toString()");
                }
            }
        }
    }

    #[test]
    fn test_mandatory_parentheses_control_flow() {
        let unparenthesized_if = r#"
            fun test() {
                if isValid { doWork() }
            }
        "#;
        let res = parse_source(unparenthesized_if);
        assert!(res.is_err());
        assert!(res.unwrap_err().message.contains("Expected '('"));

        let unparenthesized_when = r#"
            fun test() {
                when status {
                    0 -> 1
                    else -> 2
                }
            }
        "#;
        let res2 = parse_source(unparenthesized_when);
        assert!(res2.is_err());
        assert!(res2.unwrap_err().message.contains("Expected '(' or '{'"));
    }

    #[test]
    fn test_compose_as_contextual_identifier() {
        let src = r#"
            fun process(pipeline: Pipeline, compose: Step): Step {
                return pipeline.compose(compose)
            }
        "#;

        let file = parse_source(src).expect("Failed to parse compose as contextual identifier");
        if let Declaration::Function(f) = &file.declarations[0] {
            assert_eq!(f.params.len(), 2);
            assert_eq!(f.params[1].name, "compose");
            if let Some(FunctionBody::Block(b)) = &f.body {
                if let ExprKind::Return(Some(val)) = &expr_of(&items(b)[0]).kind {
                    if let ExprKind::Call { callee, args, .. } = &val.kind {
                        if let ExprKind::MemberAccess { member, .. } = &callee.kind {
                            assert_eq!(member, "compose");
                        } else {
                            panic!("Expected member access compose");
                        }
                        assert_eq!(args.len(), 1);
                        assert_eq!(args[0].value.to_string(), "compose");
                    } else {
                        panic!("Expected Call expression");
                    }
                }
            }
        }
    }

    #[test]
    fn test_reject_struct_field_borrow() {
        let src1 = r#"
            struct View {
                val data: &String
            }
        "#;
        let res1 = parse_source(src1);
        assert!(res1.is_err());
        let err1 = res1.unwrap_err();
        assert!(err1.message.contains("borrowed reference types"));
        assert!(err1.message.contains("zero-cost memory-safety invariant"));

        let src2 = r#"
            struct MutableRefHolder(val ptr: &mut Int)
        "#;
        let res2 = parse_source(src2);
        assert!(res2.is_err());
        let err2 = res2.unwrap_err();
        assert!(err2.message.contains("borrowed reference types"));
    }

    #[test]
    fn test_reject_stored_borrows_in_payloads_and_nested_types() {
        // (source, column of the borrow the error points at)
        for (src, column) in [
            ("enum Slot { Some(&String), Empty }", 18),
            ("struct A { val x: (&String)? }", 20),
            ("struct B(val x: Heap<&Int>)", 22),
            ("enum E { X((&Int)?) }", 13),
        ] {
            let err = parse_source(src).expect_err(src);
            assert_eq!(err.code, "E2001", "{src}");
            assert!(err.message.contains("borrowed reference types"), "{err}");
            assert_eq!(err.span.column, column, "{src}");
        }
        // A function value stores no borrow; its parameters only borrow during a call.
        assert!(
            parse_source("struct B { val onClick: (&Int) -> Unit, val pick: (&Int) -> &Int }")
                .is_ok()
        );
    }

    #[test]
    fn test_negative_error_diagnostics() {
        // 1. Missing closing parenthesis in expression
        let src1 = "fun test(): Int { (1 + 2 }";
        let res1 = parse_source(src1);
        assert!(res1.is_err());
        let err1 = res1.unwrap_err();
        assert!(err1.message.contains("Expected ')'"));
        assert_eq!(err1.span.line, 1);

        // 2. Missing parameter colon/type
        let src2 = "fun test(x): Int = x";
        let res2 = parse_source(src2);
        assert!(res2.is_err());
        let err2 = res2.unwrap_err();
        assert!(err2.message.contains("Expected ':'"));

        // 3. Invalid top-level declaration token
        let src3 = "12345";
        let res3 = parse_source(src3);
        assert!(res3.is_err());
        let err3 = res3.unwrap_err();
        assert!(err3.message.contains("Expected declaration"));

        // 4. compose modifier on struct
        let src4 = "compose struct BadStruct { val x: Int }";
        let res4 = parse_source(src4);
        assert!(res4.is_err());
        let err4 = res4.unwrap_err();
        assert!(err4
            .message
            .contains("'compose' modifier is only valid on functions"));

        // 5. Unterminated block struct
        let src5 = "struct OpenStruct { val x: Int";
        let res5 = parse_source(src5);
        assert!(res5.is_err());
        let err5 = res5.unwrap_err();
        assert!(err5.message.contains("Expected '}'"));
    }

    /// Runs `f` on a thread with the 8 MiB stack of the `prism` CLI's main
    /// thread. A debug build needs about 3 MiB to parse up to the nesting
    /// limit, more than the 2 MiB of a default test thread.
    fn with_cli_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn test_deep_nesting_reports_error_instead_of_overflowing() {
        let n = 100_000;
        let r = |s: &str| s.repeat(n);
        let sources = [
            format!("fun f(): Int = {}1{}", r("{"), r("}")),
            format!("fun f(): Int = {}1{}", r("("), r(")")),
            format!("fun f(): Int = {}1", r("- ")),
            format!("fun f(c: Boolean): Int = {}1", r("if (c) 1 else ")),
            format!("fun f(x: {}Int): Int = 1", r("& ")),
            format!("fun f(x: {}Int{}): Int = 1", r("Heap<"), r(">")),
            format!("fun f(): Int = {}1{}", r("g { x -> "), r(" }")),
            format!(
                "fun f(x: Int): Int = {}1{}",
                r("when (x) { else -> "),
                r(" }")
            ),
            // Left-deep chains parse iteratively but would build a tree too
            // deep to drop, clone or type-check recursively.
            format!("fun f(a: Int): Int = {}a", r("a+")),
            format!("fun f(a: Int): Int = a{}", r(".b")),
            format!("fun f(): Int = g{}", r("(1)")),
        ];
        for src in sources {
            let err = with_cli_stack(move || parse_source(&src).unwrap_err());
            assert_eq!(err.code, "E2002", "{}", err);
        }
    }

    #[test]
    fn test_nesting_within_limit_parses() {
        let n = MAX_NESTING_DEPTH - 10;
        let sources = [
            format!("fun f(): Int = {}1{}", "(".repeat(n), ")".repeat(n)),
            format!("fun f(a: Int): Int = {}a", "a+".repeat(n)),
        ];
        for src in sources {
            with_cli_stack(move || parse_source(&src).unwrap());
        }
    }

    #[test]
    fn test_token_stream_without_eof() {
        let file = Parser::new(vec![]).parse_source_file().unwrap();
        assert!(file.declarations.is_empty());

        let mut tokens = Lexer::new("fun f(): Int = 1").tokenize_all();
        assert_eq!(tokens.pop().unwrap().kind, TokenKind::Eof);
        let file = Parser::new(tokens).parse_source_file().unwrap();
        assert_eq!(file.declarations.len(), 1);
    }

    #[test]
    fn test_bare_return_before_non_expression_token() {
        let bare_return = |e: &Expr| matches!(e.kind, ExprKind::Return(None));

        let file = parse_source("fun f(c: Boolean) { if (c) return else g() }").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Block(body)) = &f.body else {
            panic!("Expected block body");
        };
        let body_items = items(body);
        let ExprKind::If {
            then_branch,
            else_branch,
            ..
        } = &expr_of(&body_items[0]).kind
        else {
            panic!("Expected if");
        };
        assert!(bare_return(then_branch));
        assert!(else_branch.is_some());

        let file = parse_source("fun f(x: Int) = when (x) { 0 -> return, else -> 1 }").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::When { branches, .. } = &e.kind else {
            panic!("Expected when");
        };
        assert_eq!(branches.len(), 2);
        assert!(bare_return(&branches[0].body));

        let file = parse_source("fun f() = g(return)").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        assert!(bare_return(&args[0].value));
    }

    #[test]
    fn test_when_branch_without_pattern_rejected() {
        let err = parse_source("fun f(x: Int) = when (x) { -> 1 }").unwrap_err();
        assert!(err.message.contains("Expected a pattern"), "{}", err);
        assert_eq!(err.span.column, 28);

        let err = parse_source("fun f(x: Int) = when (x) { 0 -> 1, -> 2 }").unwrap_err();
        assert!(err.message.contains("Expected a pattern"), "{}", err);
    }

    #[test]
    fn test_invalid_assignment_targets_rejected() {
        for src in [
            "fun f() { a + b = c }",
            "fun f() { 1 = 2 }",
            "fun f() { g() = 1 }",
            "fun f() { a?.b = 1 }",
        ] {
            let err = parse_source(src).unwrap_err();
            assert!(
                err.message.contains("Invalid assignment target"),
                "{}: {}",
                src,
                err
            );
        }
        for src in [
            "fun f() { a = 1 }",
            "fun f() { a.b = 1 }",
            "fun f() { a[0] = 1 }",
            "fun f() { *a = 1 }",
        ] {
            parse_source(src).unwrap();
        }
    }

    #[test]
    fn test_compose_as_named_argument_and_closure_param() {
        let file = parse_source("fun f() = g(compose = 1)").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        assert_eq!(args[0].name.as_deref(), Some("compose"));
        assert!(matches!(
            args[0].value.kind,
            ExprKind::Literal(Literal::Int(1))
        ));

        let file = parse_source("fun f() = g { compose -> compose }").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        let ExprKind::Closure { params, .. } = &args[0].value.kind else {
            panic!("Expected closure");
        };
        assert_eq!(names(params), vec!["compose"]);
    }

    #[test]
    fn test_split_joined_operator_tokens() {
        // `>=` closing a generic argument list is `>` followed by `=`.
        let file = parse_source("fun f(x: Heap<Int>) { val y: Heap<Heap<Int>>= x }").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Block(body)) = &f.body else {
            panic!("Expected block body");
        };
        let body_items = items(body);
        let StatementKind::Local { ty, .. } = &body_items[0].kind else {
            panic!("Expected local var");
        };
        assert_eq!(ty.as_ref().unwrap().to_string(), "Heap<Heap<Int>>");
        assert_eq!(ty.as_ref().unwrap().span.end, 44);

        // `&&` in prefix position is a borrow of a borrow, in types and expressions.
        let file = parse_source("fun f(x: &&Int, y: &&mut Int) = g(&&x, a && b)").unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        assert_eq!(f.params[0].ty.to_string(), "&&Int");
        assert_eq!(f.params[1].ty.to_string(), "&&mut Int");
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        assert_eq!(args[0].value.to_string(), "(&(&x))");
        assert_eq!(args[0].value.span.start, 34);
        let ExprKind::Unary { expr: inner, .. } = &args[0].value.kind else {
            panic!("Expected borrow");
        };
        assert_eq!(inner.span.start, 35);
        assert_eq!(args[1].value.to_string(), "(a && b)");
    }

    #[test]
    fn test_int_min_literal() {
        let body = |src: &str| {
            let file = parse_source(src).unwrap();
            let Declaration::Function(f) = &file.declarations[0] else {
                panic!("Expected function");
            };
            let Some(FunctionBody::Expr(e)) = &f.body else {
                panic!("Expected expression body");
            };
            e.clone()
        };
        let min = body("fun f(): Int = -9223372036854775808");
        assert_eq!(min.kind, ExprKind::Literal(Literal::Int(i64::MIN)));
        assert_eq!((min.span.start, min.span.end), (15, 35));
        assert_eq!(
            body("fun f(): Int = -9223372036854775808 + 1").to_string(),
            "(-9223372036854775808 + 1)"
        );
        assert_eq!(
            body("fun f(): Int = -9223372036854775807").to_string(),
            "(-9223372036854775807)"
        );

        // Without the minus, or with a postfix operator binding tighter than
        // the minus, the magnitude does not fit an Int.
        for src in [
            "fun f(): Int = 9223372036854775808",
            "fun f(): Int = -9223372036854775808.abs()",
        ] {
            let err = parse_source(src).unwrap_err();
            assert!(err.message.contains("out of range"), "{}: {}", src, err);
        }
    }

    #[test]
    fn test_spans_of_bodyless_functions_lambdas_and_closures() {
        let text = |src: &'static str, span: Span| &src[span.start..span.end];

        // S1: a function without a body ends at its signature.
        let src = "fun a(): Int\n\n\nfun b()\nfun c() = 1";
        let file = parse_source(src).unwrap();
        let spans: Vec<Span> = file
            .declarations
            .iter()
            .map(|d| match d {
                Declaration::Function(f) => f.span,
                _ => panic!("Expected function"),
            })
            .collect();
        assert_eq!(text(src, spans[0]), "fun a(): Int");
        assert_eq!(text(src, spans[1]), "fun b()");

        // S2: a trailing lambda argument spans the lambda only, a named
        // argument includes its name.
        let src = "fun f() = g(n = 1) { x -> x }";
        let file = parse_source(src).unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        assert_eq!(text(src, args[0].span), "n = 1");
        assert_eq!(text(src, args[1].span), "{ x -> x }");
        assert_eq!(text(src, e.span), "g(n = 1) { x -> x }");

        let src = "fun f() = run { 1 }";
        let file = parse_source(src).unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        assert_eq!(text(src, args[0].span), "{ 1 }");

        // S3: a closure body block is distinct from its single `val`.
        let src = "fun f() = g { x -> val y = x }";
        let file = parse_source(src).unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        let ExprKind::Call { args, .. } = &e.kind else {
            panic!("Expected call");
        };
        let ExprKind::Closure { body, .. } = &args[0].value.kind else {
            panic!("Expected closure");
        };
        assert_eq!(text(src, body.span), "-> val y = x }");
        assert_eq!(text(src, items(body)[0].span), "val y = x");
    }

    #[test]
    fn test_lexer_error_message_and_span_surfaced() {
        let src = "fun f() {\n    val s = \"bad \\q\"\n}";
        let err = parse_source(src).unwrap_err();
        assert_eq!(err.message, "Unknown escape sequence '\\q'");
        assert_eq!((err.span.line, err.span.column), (2, 13));

        let err = parse_source("fun f(x: Int) = x | 1").unwrap_err();
        assert_eq!(err.message, "Unexpected single '|'");
    }

    // --- harden-parser-grammar: braces, declarations, function types, newlines ---

    /// The names of closure parameters.
    fn names(params: &[ClosureParameter]) -> Vec<&str> {
        params.iter().map(|p| p.name.as_str()).collect()
    }

    /// A block's items in source order as statements, the tail last.
    fn items(b: &Block) -> Vec<Statement> {
        let mut items = b.statements.clone();
        if let Some(tail) = &b.tail {
            items.push(Statement::new(
                StatementKind::Expression((**tail).clone()),
                tail.span,
            ));
        }
        items
    }

    /// The expression of an expression statement.
    #[track_caller]
    fn expr_of(item: &Statement) -> &Expr {
        match &item.kind {
            StatementKind::Expression(e) => e,
            other => panic!("Expected an expression statement, found {:?}", other),
        }
    }

    /// The items of the first function's block body, the tail last.
    fn block_stmts(src: &str) -> Vec<Statement> {
        let file = parse_source(src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Block(b)) = &f.body else {
            panic!("Expected block body");
        };
        items(b)
    }

    /// The first function's expression body.
    fn expr_body(src: &str) -> Expr {
        let file = parse_source(src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let Some(FunctionBody::Expr(e)) = &f.body else {
            panic!("Expected expression body");
        };
        e.clone()
    }

    /// The parameter types of the first function, as written back by `Display`.
    fn param_types(src: &str) -> Vec<Type> {
        let file = parse_source(src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        f.params.iter().map(|p| p.ty.clone()).collect()
    }

    #[track_caller]
    fn assert_parse_error(src: &str, message_part: &str) {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", err);
        assert!(
            err.message.contains(message_part),
            "expected '{}' in '{}' for {}",
            message_part,
            err.message,
            src
        );
    }

    /// A closure value with zero parameters, returning its body statement count.
    #[track_caller]
    fn parameterless_closure_len(e: &Expr) -> usize {
        match &e.kind {
            ExprKind::Closure { params, body, .. } if params.is_empty() => items(body).len(),
            other => panic!("Expected a parameterless closure, found {:?}", other),
        }
    }

    fn call_args(e: &Expr) -> &[CallArg] {
        match &e.kind {
            ExprKind::Call { args, .. } => args,
            other => panic!("Expected call, found {:?}", other),
        }
    }

    #[test]
    fn trailing_lambda_without_parameters_is_a_closure() {
        let stmts = block_stmts(
            "fun f() {\n    Column { Item() }\n    Button(onClick = handler) { Text(\"Click\") }\n}",
        );
        assert_eq!(
            parameterless_closure_len(&call_args(expr_of(&stmts[0]))[0].value),
            1
        );
        let args = call_args(expr_of(&stmts[1]));
        assert_eq!(args.len(), 2);
        assert_eq!(parameterless_closure_len(&args[1].value), 1);
    }

    #[test]
    fn brace_argument_and_initializer_are_closures() {
        let stmts = block_stmts(
            "fun f() {\n    g({ 1 })\n    Button(onClick = { count = 1 })\n    val f = { 1 }\n}",
        );
        assert_eq!(
            parameterless_closure_len(&call_args(expr_of(&stmts[0]))[0].value),
            1
        );
        assert_eq!(
            parameterless_closure_len(&call_args(expr_of(&stmts[1]))[0].value),
            1
        );
        let StatementKind::Local { initializer, .. } = &stmts[2].kind else {
            panic!("Expected declaration");
        };
        assert_eq!(parameterless_closure_len(initializer), 1);
    }

    #[test]
    fn single_expression_closure_body_is_a_block() {
        let e = expr_body("fun f() = run { x -> x + 1 }");
        let ExprKind::Closure { params, body, .. } = &call_args(&e)[0].value.kind else {
            panic!("Expected closure");
        };
        assert_eq!(names(params), vec!["x"]);
        // A parameter's span is its name.
        assert_eq!((params[0].span.line, params[0].span.column), (1, 17));
        assert_eq!(items(body).len(), 1);
        assert_eq!(items(body)[0].to_string(), "(x + 1)");
    }

    // --- implicit-it ---

    /// syntax: "it as a declared name".
    #[test]
    fn it_as_a_declared_name() {
        let file = parse_source(
            "fun it(it: Int): Int = it\nfun f() { val it = 1; g(it = it) }\nfun h() = x.it()",
        )
        .unwrap();
        let Declaration::Function(it) = &file.declarations[0] else {
            panic!("Expected function");
        };
        assert_eq!(it.name, "it");
        assert_eq!(it.params[0].name, "it");
        let stmts = block_stmts("fun f() { val it = 1; g(it = it) }");
        let StatementKind::Local { name, .. } = &stmts[0].kind else {
            panic!("Expected declaration");
        };
        assert_eq!(name, "it");
        let args = call_args(expr_of(&stmts[1]));
        assert_eq!(args[0].name.as_deref(), Some("it"));
        let e = expr_body("fun f() = run { it -> it + 1 }");
        let ExprKind::Closure { params, arrow, .. } = &call_args(&e)[0].value.kind else {
            panic!("Expected closure");
        };
        assert_eq!(names(params), vec!["it"]);
        assert!(arrow.is_some());
    }

    /// syntax: "A header-less closure has no parameters in the syntax tree".
    #[test]
    fn a_header_less_closure_has_no_parameters_in_the_syntax_tree() {
        let stmts = block_stmts("fun f() { xs.forEach { println(it) } }");
        let ExprKind::Closure {
            params,
            arrow,
            body,
        } = &call_args(expr_of(&stmts[0]))[0].value.kind
        else {
            panic!("Expected closure");
        };
        assert!(params.is_empty());
        assert!(arrow.is_none());
        let body_items = items(body);
        let ExprKind::Call { args, .. } = &expr_of(&body_items[0]).kind else {
            panic!("Expected call");
        };
        assert!(matches!(&args[0].value.kind, ExprKind::Ident(name) if name == "it"));
    }

    /// syntax: "An empty parameter header is recorded".
    #[test]
    fn an_empty_parameter_header_is_recorded() {
        let stmts = block_stmts("fun f() {\n    run { -> 1 }\n    run { 1 }\n    run { -> }\n}");
        let headers: Vec<(usize, Option<(u32, u32)>)> = stmts
            .iter()
            .map(
                |statement| match &call_args(expr_of(statement))[0].value.kind {
                    ExprKind::Closure { params, arrow, .. } => {
                        (params.len(), arrow.map(|a| (a.line, a.column)))
                    }
                    other => panic!("Expected closure, found {:?}", other),
                },
            )
            .collect();
        assert_eq!(
            headers,
            vec![(0, Some((2, 11))), (0, None), (0, Some((4, 11)))]
        );
        assert_eq!(
            expr_of(&stmts[0]).to_string(),
            "run({ -> ... (0 statements, tail) })"
        );
    }

    #[test]
    fn braces_in_block_position_stay_blocks() {
        let stmts = block_stmts(
            "fun f(c: Bool, x: Int) {\n    if (c) { a() } else { b() }\n    when (x) {\n        0 -> { a() }\n        else -> { }\n    }\n    { val t = 1 }\n}",
        );
        let ExprKind::If {
            then_branch,
            else_branch,
            ..
        } = &expr_of(&stmts[0]).kind
        else {
            panic!("Expected if");
        };
        assert!(matches!(then_branch.kind, ExprKind::Block(_)));
        assert!(matches!(
            else_branch.as_ref().unwrap().kind,
            ExprKind::Block(_)
        ));
        let ExprKind::When { branches, .. } = &expr_of(&stmts[1]).kind else {
            panic!("Expected when");
        };
        assert!(branches
            .iter()
            .all(|b| matches!(b.body.kind, ExprKind::Block(_))));
        assert!(matches!(expr_of(&stmts[2]).kind, ExprKind::Block(_)));
    }

    #[test]
    fn expression_bodied_function_returning_a_closure() {
        assert_eq!(parameterless_closure_len(&expr_body("fun f() = { 1 }")), 1);
    }

    #[test]
    fn parameterless_closure_body_span_differs_from_closure_and_statement() {
        let src = "fun f() = run { val y = 1 }";
        let e = expr_body(src);
        let closure = &call_args(&e)[0].value;
        let ExprKind::Closure { body, .. } = &closure.kind else {
            panic!("Expected closure");
        };
        let text = |span: Span| &src[span.start..span.end];
        assert_eq!(text(closure.span), "{ val y = 1 }");
        assert_eq!(text(body.span), "val y = 1 }");
        assert_eq!(text(items(body)[0].span), "val y = 1");
    }

    #[test]
    fn a_block_records_its_closing_brace() {
        let src = "fun f() = if (true) {\n    val y = 1\n  } else 2";
        let e = expr_body(src);
        let ExprKind::If { then_branch, .. } = &e.kind else {
            panic!("Expected if, found {:?}", e.kind);
        };
        let ExprKind::Block(block) = &then_branch.kind else {
            panic!("Expected block");
        };
        assert_eq!(&src[block.close_brace.start..block.close_brace.end], "}");
        assert_eq!((block.close_brace.line, block.close_brace.column), (3, 3));
        assert_eq!(block.span.end, block.close_brace.end);
    }

    #[test]
    fn function_type_syntax() {
        let types = param_types(
            "fun f(onClick: () -> Unit, content: compose () -> Unit, item: compose (Int) -> Unit, g: (Int) -> (Int) -> Int, a: (() -> Unit)?, b: () -> Unit?, u: ()) { }",
        );
        let shown: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        assert_eq!(
            shown,
            [
                "() -> Unit",
                "compose () -> Unit",
                "compose (Int) -> Unit",
                "(Int) -> (Int) -> Int",
                "(() -> Unit)?",
                "() -> Unit?",
                "()",
            ]
        );
        let compose_flags: Vec<bool> = types[..3]
            .iter()
            .map(|t| match &t.kind {
                TypeKind::Function {
                    is_compose, params, ..
                } => {
                    assert!(params.len() <= 1);
                    *is_compose
                }
                other => panic!("Expected function type, found {:?}", other),
            })
            .collect();
        assert_eq!(compose_flags, [false, true, true]);
        // `->` is right-associative.
        let TypeKind::Function { return_type, .. } = &types[3].kind else {
            panic!("Expected function type");
        };
        assert!(matches!(return_type.kind, TypeKind::Function { .. }));
        // `(() -> Unit)?` is a nullable function; `() -> Unit?` returns `Unit?`.
        assert!(
            matches!(&types[4].kind, TypeKind::Nullable(inner) if matches!(inner.kind, TypeKind::Function { .. }))
        );
        let TypeKind::Function { return_type, .. } = &types[5].kind else {
            panic!("Expected function type");
        };
        assert!(matches!(return_type.kind, TypeKind::Nullable(_)));
        assert!(matches!(types[6].kind, TypeKind::Unit));
    }

    #[test]
    fn escaping_parameters() {
        let file = parse_source(
            "fun b(onClick: escaping () -> Unit, content: escaping compose () -> Unit, f: escaping (() -> Unit)? = null, n: Int, r: (escaping () -> Unit, Int) -> Unit) {}",
        )
        .unwrap();
        let Declaration::Function(f) = &file.declarations[0] else {
            panic!("Expected function");
        };
        let flags: Vec<bool> = f.params.iter().map(|p| p.escaping).collect();
        assert_eq!(flags, [true, true, true, false, false]);
        let shown: Vec<String> = f.params.iter().map(|p| p.ty.to_string()).collect();
        assert_eq!(
            shown,
            [
                "() -> Unit",
                "compose () -> Unit",
                "(() -> Unit)?",
                "Int",
                "(escaping () -> Unit, Int) -> Unit",
            ]
        );
        let TypeKind::Function { escaping, .. } = &f.params[4].ty.kind else {
            panic!("Expected function type");
        };
        assert_eq!(escaping, &[true, false]);
    }

    #[test]
    fn escaping_on_another_type_rejected() {
        for src in [
            "fun f(n: escaping Int) {}",
            "fun f(n: escaping &Int) {}",
            "fun g(x: escaping (Int)) {}",
            "fun g(x: escaping ()) {}",
            "fun h() { val k: escaping () -> Unit = {} }",
            "fun h() { val k: (escaping () -> Unit)? = null }",
            "fun m(): escaping () -> Unit = {}",
            "struct S { val f: escaping () -> Unit }",
        ] {
            assert_parse_error(src, "'escaping' marks parameters of function type");
        }
    }

    #[test]
    fn escaping_as_an_identifier() {
        let types = param_types("fun f(x: escaping, y: escaping?, z: escaping<Int>) {}");
        let shown: Vec<String> = types.iter().map(|t| t.to_string()).collect();
        assert_eq!(shown, ["escaping", "escaping?", "escaping<Int>"]);
        parse_source("fun escaping(escaping: Int): Int = escaping").unwrap();
        parse_source("fun g() { val escaping = 1; h(escaping = escaping); x.escaping() }").unwrap();
    }

    #[test]
    fn multiple_types_in_parentheses_without_an_arrow_rejected() {
        assert_parse_error(
            "fun f(p: (Int, String)) { }",
            "Tuple types are not supported",
        );
        assert_parse_error("fun f(p: compose (Int)) { }", "Expected '->'");
    }

    #[test]
    fn declarations_in_statement_position_only() {
        for src in [
            "fun f() { g(val z = 1) }",
            "fun f() { x = val y = 3 }",
            "fun f() = val x = 1",
            "fun f(c: Bool): Int {\n    if (c) val x = 1\n    return x\n}",
            "fun f(x: Int) { when (x) { else -> var y = 1 } }",
        ] {
            assert_parse_error(src, "only allowed as a statement");
        }
        let stmts = block_stmts("fun f(c: Bool) { if (c) { val x = 1 } }");
        let ExprKind::If { then_branch, .. } = &expr_of(&stmts[0]).kind else {
            panic!("Expected if");
        };
        let ExprKind::Block(b) = &then_branch.kind else {
            panic!("Expected block");
        };
        assert!(matches!(items(b)[0].kind, StatementKind::Local { .. }));
    }

    #[test]
    fn chained_assignment_through_a_declaration_rejected() {
        for src in [
            "fun f() { val a = b = c }",
            "val a = b = c",
            "fun f() { a = b = c }",
        ] {
            assert_parse_error(src, "Chained assignments are not allowed");
        }
    }

    #[test]
    fn leading_minus_starts_a_new_statement() {
        for src in [
            "fun f() {\n    val a = b\n    -1\n}",
            "fun f(c: Bool) {\n    if (c) { foo() }\n    -1\n}",
        ] {
            let stmts = block_stmts(src);
            assert_eq!(stmts.len(), 2, "{}", src);
            assert_eq!(stmts[1].to_string(), "(-1)");
        }
    }

    #[test]
    fn leading_minus_starts_a_new_when_branch() {
        let e = expr_body("fun f(x: Int) = when (x) {\n    0 -> a\n    -1 -> b\n    else -> c\n}");
        let ExprKind::When { branches, .. } = &e.kind else {
            panic!("Expected when");
        };
        let patterns: Vec<String> = branches
            .iter()
            .map(|b| match &b.patterns[..] {
                [WhenPattern::Pattern(Pattern {
                    kind: PatternKind::Value(p),
                    ..
                })] => p.to_string(),
                [WhenPattern::Else] => "else".to_string(),
                other => panic!("Unexpected patterns {:?}", other),
            })
            .collect();
        assert_eq!(patterns, ["0", "(-1)", "else"]);
        assert_eq!(branches[0].body.to_string(), "a");
    }

    #[test]
    fn continuation_operators_join_lines() {
        let stmts = block_stmts(
            "fun f() {\n    val ok = a\n        && b\n    val n = user\n        ?.name\n        ?: \"anon\"\n}",
        );
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].to_string(), "val ok = (a && b)");
        assert_eq!(stmts[1].to_string(), "val n = (user?.name ?: \"anon\")");
    }

    #[test]
    fn newlines_inside_parentheses_and_brackets_are_ignored() {
        let stmts = block_stmts("fun f() {\n    g(a\n        - 1)\n    xs[i\n        + 1]\n}");
        assert_eq!(stmts.len(), 2);
        assert_eq!(stmts[0].to_string(), "g((a - 1))");
        assert_eq!(stmts[1].to_string(), "xs[(i + 1)]");
    }

    #[test]
    fn braces_inside_parentheses_restore_newline_sensitivity() {
        let stmts = block_stmts("fun f() {\n    g({ a\n        -1 })\n}");
        assert_eq!(
            parameterless_closure_len(&call_args(expr_of(&stmts[0]))[0].value),
            2
        );
    }

    #[test]
    fn leading_parenthesis_keeps_its_existing_rule() {
        let stmts = block_stmts("fun f() {\n    cleanup()\n    (1 + 2).toString()\n}");
        assert_eq!(stmts.len(), 2);
    }

    #[test]
    fn statements_and_members_on_one_line_need_a_separator() {
        assert_parse_error(
            "fun f() { val a = 1 val b = 2 }",
            "Expected a newline or ';' between statements",
        );
        assert_parse_error(
            "struct P { val x: Int val y: Int }",
            "between struct fields",
        );
        assert_parse_error("enum E { A B }", "between enum variants");
        assert_parse_error(
            "fun f(x: Int) = when (x) { 0 -> a 1 -> b }",
            "between 'when' branches",
        );

        assert_eq!(block_stmts("fun f() { val a = 1; val b = 2 }").len(), 2);
        assert_eq!(
            block_stmts("fun f() {\n    val a = 1\n    val b = 2\n}").len(),
            2
        );
        for src in [
            "struct P { val x: Int, val y: Int }",
            "struct P {\n    val x: Int\n    val y: Int\n}",
            "enum E { A, B }",
            "enum E {\n    A\n    B\n}",
            "fun f(x: Int) = when (x) { 0 -> a, 1 -> b, else -> c }",
            "fun f(x: Int) = when (x) {\n    0 -> a\n    else -> c\n}",
        ] {
            parse_source(src).unwrap_or_else(|e| panic!("{}\n{}", e, src));
        }
    }

    #[test]
    fn import_attempting_directory_traversal() {
        for src in [
            "import ../outside",
            "import ./sibling",
            "import /core/../fmt",
            "import core..fmt",
        ] {
            assert_parse_error(src, "import paths cannot contain '.' or '..'");
        }
        parse_source("import /core/fmt\nimport widgets").expect("identifier segments parse");
    }

    /// The patterns of each branch of the first function's `when` body, written back by
    /// `Display`, with each guard after `if`.
    fn when_patterns(src: &str) -> Vec<String> {
        let e = expr_body(src);
        let ExprKind::When { branches, .. } = &e.kind else {
            panic!("Expected when in {}", src);
        };
        branches
            .iter()
            .map(|b| {
                let patterns: Vec<String> = b
                    .patterns
                    .iter()
                    .map(|p| match p {
                        WhenPattern::Pattern(pattern) => match &pattern.kind {
                            PatternKind::Value(value) => format!("value {}", value),
                            _ => pattern.to_string(),
                        },
                        WhenPattern::Else => "else".to_string(),
                    })
                    .collect();
                match &b.guard {
                    Some(guard) => format!("{} if {}", patterns.join(", "), guard),
                    None => patterns.join(", "),
                }
            })
            .collect()
    }

    #[test]
    fn constructor_patterns_are_told_apart_from_values() {
        assert_eq!(
            when_patterns(
                "fun f(x: Int) = when (x) { g(1) + 1 -> a; g(3) -> b; f(x).y -> c; h(1) { 2 } -> d; Shape.Rect(val w, _) -> e; else -> z }"
            ),
            [
                "value (g(1) + 1)",
                "g(3)",
                "value f(x).y",
                "value h(1, { ... (0 statements, tail) })",
                "Shape.Rect(val w, _)",
                "else",
            ]
        );
        assert_eq!(
            when_patterns(
                "fun f(e: Event) = when (e) { Event.Click(Point(0, val y)), Event.Key(var k) if k > 1 -> a; Point(y = val y, x = 0) -> b; Box.Of(g(1) * 2) -> c }"
            ),
            [
                "Event.Click(Point(0, val y)), Event.Key(var k) if (k > 1)",
                "Point(y = val y, x = 0)",
                "Box.Of((g(1) * 2))",
            ]
        );
    }

    #[test]
    fn wildcards_bindings_and_ranges_as_patterns() {
        assert_eq!(
            when_patterns(
                "fun f(x: Int) = when (x) { 0 -> a; in 1..<3 -> b; val n if n > 9 -> c; _ -> d }"
            ),
            ["value 0", "in 1..<3", "val n if (n > 9)", "_"]
        );
        assert_eq!(
            when_patterns("fun f(x: Shape) = when (x) { Shape.Circle -> a; Red -> b }"),
            ["value Shape.Circle", "value Red"]
        );
    }

    #[test]
    fn patterns_and_guards_need_a_subject() {
        for src in [
            "fun f(c: Bool): Int = when { _ -> 1 }",
            "fun f(c: Bool): Int = when { val x -> 1; else -> 2 }",
            "fun f(c: Bool): Int = when { c if c -> 1; else -> 2 }",
        ] {
            assert_parse_error(src, "without a subject is a condition");
        }
        assert_parse_error(
            "fun f(x: Int): Int = when (x) { else if x > 0 -> 1 }",
            "'else' branch cannot have a guard",
        );
    }

    #[test]
    fn destructuring_declarations() {
        let stmts = block_stmts(
            "fun f(u: User) { val (name as n, email,) = u; var (age) = u; val as = 1 }",
        );
        let shown: Vec<String> = stmts.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            shown,
            ["val (name as n, email) = u", "var (age) = u", "val as = 1"]
        );
        for (src, message) in [
            ("fun f() { val (a, b): Pair = p }", "no type annotation"),
            ("fun f() { val () = p }", "at least one field name"),
            ("fun f() { val (a.b) = p }", "Expected ',' or ')'"),
            ("fun f() { val (_) = p }", "'_' is not needed"),
        ] {
            assert_parse_error(src, message);
        }
    }
}
