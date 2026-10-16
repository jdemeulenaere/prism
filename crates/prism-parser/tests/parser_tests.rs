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
                    assert_eq!(b.stmts.len(), 4);

                    // 1. Column(padding = 16) { Text("Hello") }
                    if let ExprKind::Call { callee, args } = &b.stmts[0].kind {
                        assert_eq!(callee.to_string(), "Column");
                        assert_eq!(args.len(), 2);
                        assert_eq!(args[0].name.as_deref(), Some("padding"));
                        assert!(matches!(args[1].value.kind, ExprKind::Block(_)));
                    } else {
                        panic!("Expected Call expr for Column");
                    }

                    // 2. val state = remember { mutableStateOf(0) }
                    if let ExprKind::LocalVar { initializer, .. } = &b.stmts[1].kind {
                        if let ExprKind::Call { callee, args } = &initializer.kind {
                            assert_eq!(callee.to_string(), "remember");
                            assert_eq!(args.len(), 1);
                        } else {
                            panic!("Expected Call for remember");
                        }
                    } else {
                        panic!("Expected LocalVar for state");
                    }

                    // 4. items.forEach { item -> val s = item * item; println(s) }
                    if let ExprKind::Call { callee, args } = &b.stmts[3].kind {
                        if let ExprKind::MemberAccess { member, .. } = &callee.kind {
                            assert_eq!(member, "forEach");
                        } else {
                            panic!("Expected MemberAccess for forEach");
                        }
                        assert_eq!(args.len(), 1);
                        if let ExprKind::Closure { params, body } = &args[0].value.kind {
                            assert_eq!(params, &vec!["item".to_string()]);
                            if let ExprKind::Block(cb) = &body.kind {
                                assert_eq!(cb.stmts.len(), 2);
                            } else {
                                panic!("Expected Block body in closure");
                            }
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
                if let ExprKind::Return(Some(val)) = &b.stmts[0].kind {
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
                assert_eq!(b.stmts.len(), 2);
                assert!(matches!(b.stmts[0].kind, ExprKind::Return(None)));
                assert!(matches!(b.stmts[1].kind, ExprKind::Call { .. }));
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
                assert_eq!(b.stmts.len(), 1);
                if let ExprKind::Return(Some(val)) = &b.stmts[0].kind {
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
                assert_eq!(b.stmts.len(), 3);
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
                assert_eq!(b.stmts.len(), 2);
                // First stmt is cleanup()
                if let ExprKind::Call { callee, args } = &b.stmts[0].kind {
                    assert_eq!(callee.to_string(), "cleanup");
                    assert_eq!(args.len(), 0);
                } else {
                    panic!("Expected cleanup() call");
                }
                // Second stmt is (1 + 2).toString()
                if let ExprKind::Call { callee, .. } = &b.stmts[1].kind {
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
                if let ExprKind::Return(Some(val)) = &b.stmts[0].kind {
                    if let ExprKind::Call { callee, args } = &val.kind {
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
        assert!(err1.message.contains("zero-cost memory invariant"));

        let src2 = r#"
            struct MutableRefHolder(val ptr: &mut Int)
        "#;
        let res2 = parse_source(src2);
        assert!(res2.is_err());
        let err2 = res2.unwrap_err();
        assert!(err2.message.contains("borrowed reference types"));
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
        let ExprKind::If {
            then_branch,
            else_branch,
            ..
        } = &body.stmts[0].kind
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
        assert_eq!(params, &vec!["compose".to_string()]);
    }
}
