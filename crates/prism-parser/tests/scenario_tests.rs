//! Scenario coverage for `openspec/specs/syntax`: AST-shape assertions for the scenarios whose
//! THEN is "parses into ...". The accept/reject forms of the same scenarios are fixtures in
//! `prism-cli/tests/fixtures/syntax`.

use prism_ast::*;
use prism_parser::parse_source;

fn parse(src: &str) -> SourceFile {
    parse_source(src).unwrap_or_else(|e| panic!("source must parse: {}\n{}", e, src))
}

/// The body expression of the only function in `src`.
fn expr_body(src: &str) -> Expr {
    let file = parse(src);
    match &file.declarations[0] {
        Declaration::Function(f) => match f.body.as_ref().expect("function has a body") {
            FunctionBody::Expr(e) => e.clone(),
            FunctionBody::Block(_) => panic!("expected an expression body"),
        },
        other => panic!("expected a function, got {:?}", other),
    }
}

/// The block body of the only function in `src`.
fn block_body(src: &str) -> Block {
    let file = parse(src);
    match &file.declarations[0] {
        Declaration::Function(f) => match f.body.as_ref().expect("function has a body") {
            FunctionBody::Block(b) => b.clone(),
            FunctionBody::Expr(_) => panic!("expected a block body"),
        },
        other => panic!("expected a function, got {:?}", other),
    }
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

/// The items of the only function's block body in `src`, the tail last.
fn block_stmts(src: &str) -> Vec<Statement> {
    items(&block_body(src))
}

/// The expression of an expression statement.
#[track_caller]
fn expr_of(item: &Statement) -> &Expr {
    match &item.kind {
        StatementKind::Expression(e) => e,
        other => panic!("expected an expression statement, got {:?}", other),
    }
}

fn int(e: &Expr) -> i64 {
    match &e.kind {
        ExprKind::Literal(Literal::Int(n)) => *n,
        other => panic!("expected an Int literal, got {:?}", other),
    }
}

fn ident(e: &Expr) -> &str {
    match &e.kind {
        ExprKind::Ident(name) => name,
        other => panic!("expected an identifier, got {:?}", other),
    }
}

// --- Default Private Visibility and Export Modifiers

#[test]
fn declarations_without_a_modifier_are_private_for_every_kind() {
    let file = parse("fun f(): Int = 1\nstruct S { val a: Int }\nenum E { A }\nval v: Int = 1\n");
    let visibilities: Vec<_> = file.declarations.iter().map(|d| d.visibility()).collect();
    assert_eq!(visibilities, vec![Visibility::Private; 4]);
}

#[test]
fn package_and_public_modifiers_apply_to_every_kind() {
    let file = parse(
        "package fun f(): Int = 1\npackage struct S { val a: Int }\npackage enum E { A }\n\
         public fun g(): Int = 1\npublic struct T { val a: Int }\npublic enum F { A }\n",
    );
    let visibilities: Vec<_> = file.declarations.iter().map(|d| d.visibility()).collect();
    assert_eq!(
        visibilities,
        vec![
            Visibility::Package,
            Visibility::Package,
            Visibility::Package,
            Visibility::Public,
            Visibility::Public,
            Visibility::Public,
        ]
    );
}

// --- Value Structs and Algebraic Enums

#[test]
fn value_struct_has_by_value_fields_in_both_layouts() {
    for src in [
        "struct Point { val x: Double, val y: Double }\n",
        "struct Point {\n    val x: Double\n    val y: Double\n}\n",
    ] {
        let file = parse(src);
        let Declaration::Struct(s) = &file.declarations[0] else {
            panic!("expected a struct");
        };
        assert_eq!(s.name, "Point");
        let fields: Vec<_> = s
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.is_var, f.ty.to_string()))
            .collect();
        assert_eq!(
            fields,
            vec![
                ("x", false, "Double".to_string()),
                ("y", false, "Double".to_string())
            ],
            "{}",
            src
        );
    }
}

#[test]
fn recursive_enum_variants_carry_heap_payloads_in_both_layouts() {
    for src in [
        "enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }\n",
        "enum Expr {\n    Literal(Int)\n    Add(Heap<Expr>, Heap<Expr>)\n}\n",
    ] {
        let file = parse(src);
        let Declaration::Enum(e) = &file.declarations[0] else {
            panic!("expected an enum");
        };
        assert_eq!(e.name, "Expr");
        assert_eq!(e.variants.len(), 2);
        assert_eq!(e.variants[0].fields[0].to_string(), "Int");
        let add: Vec<_> = e.variants[1].fields.iter().map(|t| t.to_string()).collect();
        assert_eq!(add, vec!["Heap<Expr>", "Heap<Expr>"], "{}", src);
    }
}

// --- Expression-Oriented Control Flow

#[test]
fn if_else_assigned_to_a_variable_parses_condition_and_both_branches() {
    let stmts = block_stmts("fun f(condition: Bool): Int {\n    val result = if (condition) 10 else 20\n    return result\n}\n");
    let StatementKind::Local {
        name, initializer, ..
    } = &stmts[0].kind
    else {
        panic!("expected a local variable");
    };
    assert_eq!(name, "result");
    let ExprKind::If {
        cond,
        then_branch,
        else_branch,
    } = &initializer.kind
    else {
        panic!("expected an if expression, got {:?}", initializer.kind);
    };
    assert_eq!(ident(cond), "condition");
    assert_eq!(int(then_branch), 10);
    assert_eq!(int(else_branch.as_ref().expect("else branch")), 20);
}

#[test]
fn when_branches_parse_into_value_list_range_and_else_patterns_in_source_order() {
    let body = expr_body(
        "fun f(x: Int): String = when (x) { 0 -> \"zero\", 1, 2, 3 -> \"few\", in 4..10 -> \"small\", else -> \"large\" }\n",
    );
    let ExprKind::When { target, branches } = &body.kind else {
        panic!("expected a when expression");
    };
    assert_eq!(ident(target.as_ref().expect("subject")), "x");
    assert_eq!(branches.len(), 4);

    let [zero, few, small, large] = &branches[..] else {
        panic!("expected four branches");
    };
    assert!(
        matches!(&zero.patterns[..], [WhenPattern::Pattern(Pattern { kind: PatternKind::Value(e), .. })] if int(e) == 0)
    );
    let few_values: Vec<_> = few
        .patterns
        .iter()
        .map(|p| match p {
            WhenPattern::Pattern(Pattern {
                kind: PatternKind::Value(e),
                ..
            }) => int(e),
            other => panic!("expected a value pattern, got {:?}", other),
        })
        .collect();
    assert_eq!(few_values, vec![1, 2, 3]);
    match &small.patterns[..] {
        [WhenPattern::Pattern(Pattern {
            kind:
                PatternKind::InRange {
                    start,
                    end,
                    end_inclusive: true,
                },
            ..
        })] => assert_eq!((int(start), int(end)), (4, 10)),
        other => panic!("expected a range pattern, got {:?}", other),
    }
    assert!(matches!(&large.patterns[..], [WhenPattern::Else]));

    let bodies: Vec<_> = branches
        .iter()
        .map(|b| match &b.body.kind {
            ExprKind::Literal(Literal::String(s)) => s.clone(),
            other => panic!("expected a string body, got {:?}", other),
        })
        .collect();
    assert_eq!(bodies, vec!["zero", "few", "small", "large"]);
}

// --- Null Safety

#[test]
fn safe_navigation_and_elvis_parse_to_their_own_nodes() {
    let body = expr_body("fun f(name: Name?): Int = name?.length ?: 0\n");
    let ExprKind::Binary { op, left, right } = &body.kind else {
        panic!("expected a binary expression, got {:?}", body.kind);
    };
    assert_eq!(*op, BinaryOp::Elvis);
    match &left.kind {
        ExprKind::MemberAccess {
            target,
            member,
            is_safe_nav,
        } => {
            assert!(*is_safe_nav, "`?.` is safe navigation");
            assert_eq!((ident(target), member.as_str()), ("name", "length"));
        }
        other => panic!("expected a member access, got {:?}", other),
    }
    assert_eq!(int(right), 0);
}

#[test]
fn plain_member_access_is_not_safe_navigation() {
    let body = expr_body("fun f(name: Name): Int = name.length\n");
    assert!(matches!(
        &body.kind,
        ExprKind::MemberAccess {
            is_safe_nav: false,
            ..
        }
    ));
}

// --- Variable Bindings

#[test]
fn val_and_var_declarations_and_reassignment_parse() {
    let stmts = block_stmts("fun f() {\n    val a = 10\n    var b = 10\n    b = 20\n}\n");
    assert!(
        matches!(&stmts[0].kind, StatementKind::Local { is_var: false, name, .. } if name == "a")
    );
    assert!(
        matches!(&stmts[1].kind, StatementKind::Local { is_var: true, name, .. } if name == "b")
    );
    match &stmts[2].kind {
        StatementKind::Assign { target, value } => {
            assert_eq!(ident(target), "b");
            assert_eq!(int(value), 20);
        }
        other => panic!("expected an assignment, got {:?}", other),
    }
}

// --- Mandatory Parentheses on Control Flow Conditions

#[test]
fn parenthesised_if_parses_condition_and_then_block() {
    let stmts = block_stmts("fun f(isValid: Bool) {\n    if (isValid) { doWork() }\n}\n");
    let ExprKind::If {
        cond,
        then_branch,
        else_branch,
    } = &expr_of(&stmts[0]).kind
    else {
        panic!("expected an if, got {:?}", stmts[0].kind);
    };
    assert_eq!(ident(cond), "isValid");
    assert!(else_branch.is_none());
    match &then_branch.kind {
        ExprKind::Block(b) => {
            assert_eq!(items(b).len(), 1);
            assert!(
                matches!(&expr_of(&items(b)[0]).kind, ExprKind::Call { callee, .. } if ident(callee) == "doWork")
            );
        }
        other => panic!("expected a block then-branch, got {:?}", other),
    }
}

#[test]
fn parenthesised_when_subject_and_branches_parse() {
    let body =
        expr_body("fun f(status: Int): String = when (status) { 0 -> \"ok\", else -> \"fail\" }\n");
    let ExprKind::When { target, branches } = &body.kind else {
        panic!("expected a when");
    };
    assert_eq!(ident(target.as_ref().expect("subject")), "status");
    assert_eq!(branches.len(), 2);
    assert!(
        matches!(&branches[0].patterns[..], [WhenPattern::Pattern(Pattern { kind: PatternKind::Value(e), .. })] if int(e) == 0)
    );
    assert!(matches!(&branches[1].patterns[..], [WhenPattern::Else]));
}

#[test]
fn subjectless_when_has_no_target_and_condition_patterns() {
    let body = expr_body("fun f(n: Int): Int = when { n < 0 -> 0 - 1, n == 0 -> 0, else -> 1 }\n");
    let ExprKind::When { target, branches } = &body.kind else {
        panic!("expected a when");
    };
    assert!(target.is_none());
    assert_eq!(branches.len(), 3);
    assert!(matches!(
        &branches[0].patterns[..],
        [WhenPattern::Pattern(Pattern { kind: PatternKind::Value(e), .. })] if matches!(e.kind, ExprKind::Binary { op: BinaryOp::Lt, .. })
    ));
}

// --- Precedence

#[test]
fn comparison_is_the_outermost_operator_over_arithmetic() {
    let body = expr_body("fun f(a: Int, b: Int, c: Int, d: Int): Bool = a + b * c > d\n");
    // ((a + (b * c)) > d)
    let ExprKind::Binary { op, left, right } = &body.kind else {
        panic!("expected a comparison");
    };
    assert_eq!(*op, BinaryOp::Gt);
    assert_eq!(ident(right), "d");
    let ExprKind::Binary {
        op,
        left: a,
        right: product,
    } = &left.kind
    else {
        panic!("expected a sum");
    };
    assert_eq!(*op, BinaryOp::Add);
    assert_eq!(ident(a), "a");
    let ExprKind::Binary {
        op,
        left: b,
        right: c,
    } = &product.kind
    else {
        panic!("expected a product");
    };
    assert_eq!(*op, BinaryOp::Mul);
    assert_eq!((ident(b), ident(c)), ("b", "c"));
}

#[test]
fn elvis_groups_around_its_operands_exactly_as_specified() {
    // (age ?: 0) >= 18
    let cmp = expr_body("fun f(age: Int?): Bool = age ?: 0 >= 18\n");
    let ExprKind::Binary { op, left, right } = &cmp.kind else {
        panic!("expected a comparison");
    };
    assert_eq!(*op, BinaryOp::GtEq);
    assert_eq!(int(right), 18);
    let ExprKind::Binary {
        op,
        left: age,
        right: zero,
    } = &left.kind
    else {
        panic!("expected an elvis");
    };
    assert_eq!(*op, BinaryOp::Elvis);
    assert_eq!((ident(age), int(zero)), ("age", 0));

    // base ?: (offset + 10)
    let sum = expr_body("fun f(base: Int?, offset: Int): Int = base ?: offset + 10\n");
    let ExprKind::Binary { op, left, right } = &sum.kind else {
        panic!("expected an elvis");
    };
    assert_eq!(*op, BinaryOp::Elvis);
    assert_eq!(ident(left), "base");
    let ExprKind::Binary {
        op,
        left: offset,
        right: ten,
    } = &right.kind
    else {
        panic!("expected a sum");
    };
    assert_eq!(*op, BinaryOp::Add);
    assert_eq!((ident(offset), int(ten)), ("offset", 10));
}

// --- Blocks Hold Statements and an Optional Tail

#[test]
fn block_ending_in_an_expression_has_that_tail() {
    let body = block_body("fun f(): Int { val a = 1; a + 1 }");
    assert_eq!(body.statements.len(), 1);
    assert!(matches!(&body.statements[0].kind, StatementKind::Local { name, .. } if name == "a"));
    assert_eq!(body.tail.expect("tail").to_string(), "(a + 1)");
}

#[test]
fn block_ending_in_an_assignment_has_no_tail() {
    let body = block_body("fun f() { var x = 0; x = 1 }");
    assert_eq!(body.statements.len(), 2);
    assert!(matches!(
        &body.statements[1].kind,
        StatementKind::Assign { .. }
    ));
    assert!(body.tail.is_none());
}

#[test]
fn trailing_semicolon_keeps_the_tail() {
    let body = block_body("fun f(): Int { 1; }");
    assert!(body.statements.is_empty());
    assert_eq!(int(&body.tail.expect("tail")), 1);
}

// --- Assignment Is a Statement

#[test]
fn assignments_as_unbraced_branch_bodies_are_blocks_without_tail() {
    let stmts = block_stmts(
        "fun f(c: Bool, n: Int) {\n    if (c) x = 1 else x = 2\n    when (n) { 0 -> x = 3, else -> x = 4 }\n}\n",
    );
    let assignment_block = |e: &Expr| match &e.kind {
        ExprKind::Block(b) => {
            b.tail.is_none()
                && b.statements.len() == 1
                && matches!(b.statements[0].kind, StatementKind::Assign { .. })
        }
        _ => false,
    };
    let ExprKind::If {
        then_branch,
        else_branch,
        ..
    } = &expr_of(&stmts[0]).kind
    else {
        panic!("expected an if, got {:?}", stmts[0].kind);
    };
    assert!(assignment_block(then_branch));
    assert!(assignment_block(else_branch.as_ref().expect("else branch")));
    let ExprKind::When { branches, .. } = &expr_of(&stmts[1]).kind else {
        panic!("expected a when, got {:?}", stmts[1].kind);
    };
    assert!(branches.iter().all(|b| assignment_block(&b.body)));
}

#[test]
fn parenthesized_assignment_as_a_value_rejected() {
    for src in [
        "fun f() { var x = 0; val y = (x = 1) }",
        "fun f() { var x = 0; g((x = 1)) }",
        "fun f() = x = 1",
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains("assignment is a statement"), "{}", err);
    }
    let file = parse("fun f() { g(x = 1) }");
    assert_eq!(file.declarations.len(), 1);
}

// --- Half-Open Range Operator

#[test]
fn half_open_range_precedence() {
    assert_eq!(
        expr_body("fun f(n: Int) = 0..<n - 1").to_string(),
        "(0 ..< (n - 1))"
    );
}

#[test]
fn half_open_range_in_a_when_pattern() {
    let ExprKind::When { branches, .. } =
        expr_body("fun f(x: Int) = when (x) { in 0..<10 -> \"small\", else -> \"large\" }").kind
    else {
        panic!("expected a when");
    };
    match branches[0].patterns.as_slice() {
        [WhenPattern::Pattern(Pattern {
            kind:
                PatternKind::InRange {
                    start,
                    end,
                    end_inclusive: false,
                },
            ..
        })] => assert_eq!((int(start), int(end)), (0, 10)),
        other => panic!("expected a half-open range pattern, got {:?}", other),
    }
}

#[test]
fn newline_before_half_open_range_ends_the_expression() {
    let err = parse_source("fun f() {\n    val r = 0\n    ..<10\n}")
        .expect_err("..< starts no statement");
    assert_eq!(err.code, "E2001");
}

// --- Loops (`loops` spec): AST shapes

#[test]
fn while_do_and_for_loops_are_statements() {
    let body = block_body(
        "fun f(n: Int) {\n    while (n > 0) { g() }\n    do { val x = 1 } while (x > 0)\n    for (i in 0..<n) g(i)\n}\n",
    );
    assert!(body.tail.is_none(), "a block ending in a loop has no tail");
    assert_eq!(body.statements.len(), 3);
    match &body.statements[0].kind {
        StatementKind::While { condition, body } => {
            assert_eq!(condition.to_string(), "(n > 0)");
            assert!(body.tail.is_some());
        }
        other => panic!("expected a while loop, got {:?}", other),
    }
    match &body.statements[1].kind {
        StatementKind::DoWhile { body, condition } => {
            assert!(matches!(
                body.statements[0].kind,
                StatementKind::Local { .. }
            ));
            assert_eq!(condition.to_string(), "(x > 0)");
        }
        other => panic!("expected a do-while loop, got {:?}", other),
    }
    match &body.statements[2].kind {
        StatementKind::For {
            variable,
            iterable,
            body,
            ..
        } => {
            assert_eq!(variable, "i");
            assert_eq!(iterable.to_string(), "(0 ..< n)");
            // An unbraced expression body is the block's tail.
            assert!(body.statements.is_empty());
            assert_eq!(body.tail.as_ref().expect("tail").to_string(), "g(i)");
        }
        other => panic!("expected a for loop, got {:?}", other),
    }
}

#[test]
fn unbraced_assignment_loop_body_is_a_statement() {
    let body = block_body("fun f(n: Int) { var i = 0; while (i < n) i = i + 1 }");
    let StatementKind::While { body, .. } = &body.statements[1].kind else {
        panic!("expected a while loop");
    };
    assert!(body.tail.is_none());
    assert!(matches!(
        body.statements[0].kind,
        StatementKind::Assign { .. }
    ));
}

#[test]
fn do_loop_body_must_be_braced() {
    let err = parse_source("fun f(c: Bool) { do c = false while (c) }").expect_err("unbraced do");
    assert_eq!(err.code, "E2001");
    assert!(err.message.contains("must be braced"), "{}", err);
}

#[test]
fn break_and_continue_are_expressions() {
    let body = block_body(
        "fun f(xs: Int?) {\n    while (true) {\n        val x = xs ?: break\n        if (x > 0) continue\n    }\n}\n",
    );
    let StatementKind::While { body, .. } = &body.statements[0].kind else {
        panic!("expected a while loop");
    };
    let StatementKind::Local { initializer, .. } = &body.statements[0].kind else {
        panic!("expected a declaration");
    };
    assert_eq!(initializer.to_string(), "(xs ?: break)");
    let ExprKind::If { then_branch, .. } = &body.tail.as_ref().expect("tail").kind else {
        panic!("expected an if");
    };
    assert!(matches!(then_branch.kind, ExprKind::Continue));
}

fn markers(src: &str) -> Vec<Option<TypeMarker>> {
    parse(src)
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::Struct(s) => s.marker,
            Declaration::Enum(e) => e.marker,
            _ => panic!("expected a struct or enum"),
        })
        .collect()
}

#[test]
fn markers_on_a_block_struct_and_an_enum() {
    assert_eq!(
        markers(
            "struct Point : Copy { val x: Int, val y: Int }\nstruct User : Clone { val name: String }\nenum Color : Copy { Red, Green }\n"
        ),
        vec![
            Some(TypeMarker::Copy),
            Some(TypeMarker::Clone),
            Some(TypeMarker::Copy)
        ]
    );
}

#[test]
fn marker_after_type_parameters_and_a_shorthand_parameter_list() {
    let file =
        parse("struct Tagged<T> : Copy { val id: Int }\nstruct Name(val v: String) : Clone\n");
    let Declaration::Struct(tagged) = &file.declarations[0] else {
        panic!("expected a struct");
    };
    let names: Vec<&str> = tagged
        .type_parameters
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, vec!["T"]);
    assert_eq!(tagged.marker, Some(TypeMarker::Copy));
    let Declaration::Struct(name) = &file.declarations[1] else {
        panic!("expected a struct");
    };
    assert_eq!(name.fields.len(), 1);
    assert_eq!(name.marker, Some(TypeMarker::Clone));
}

#[test]
fn declaration_without_a_marker() {
    assert_eq!(markers("struct Point { val x: Int }"), vec![None]);
}

#[test]
fn other_names_after_the_colon_rejected() {
    for src in [
        "struct P : Eq { val x: Int }",
        "struct P : Copy, Clone { val x: Int }",
        "struct P : Copy, Copy { val x: Int }",
        "struct P : { val x: Int }",
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(
            err.message.contains("Exactly one of 'Copy' or 'Clone'"),
            "{}",
            err
        );
    }
}

#[test]
fn marker_in_the_wrong_place_rejected() {
    for src in [
        "struct P : Copy <T> { val x: T }",
        "struct Id : Copy (val v: Int)",
    ] {
        assert_eq!(parse_source(src).expect_err(src).code, "E2001", "{}", src);
    }
}

#[test]
fn marker_names_remain_identifiers() {
    let file =
        parse("fun f(Copy: Int, Clone: Int): Int = Copy + Clone\nstruct Clone { val x: Int }\n");
    assert_eq!(file.declarations.len(), 2);
}

// --- Writing Through a Reference

/// The number of `*` around an assignment target, and the operand under them.
fn deref_target(target: &Expr) -> (usize, &Expr) {
    let mut depth = 0;
    let mut expr = target;
    while let ExprKind::Unary {
        op: UnaryOp::Deref,
        expr: inner,
    } = &expr.kind
    {
        depth += 1;
        expr = inner;
    }
    (depth, expr)
}

#[track_caller]
fn assign_parts(item: &Statement) -> (&Expr, &Expr) {
    match &item.kind {
        StatementKind::Assign { target, value } => (target, value),
        other => panic!("expected an assignment, got {:?}", other),
    }
}

#[test]
fn write_through_statement() {
    let stmts =
        block_stmts("fun f() {\n    *r = 5\n    **rr = 0\n    *pick(&mut a, &mut b) = 1\n}\n");
    let (target, value) = assign_parts(&stmts[0]);
    let (depth, operand) = deref_target(target);
    assert_eq!((depth, ident(operand), int(value)), (1, "r", 5));
    let (depth, operand) = deref_target(assign_parts(&stmts[1]).0);
    assert_eq!((depth, ident(operand)), (2, "rr"));
    let (depth, operand) = deref_target(assign_parts(&stmts[2]).0);
    assert_eq!(depth, 1);
    assert!(
        matches!(operand.kind, ExprKind::Call { .. }),
        "{:?}",
        operand
    );
}

#[test]
fn postfix_binds_tighter_than_prefix_star() {
    let stmts = block_stmts("fun f() {\n    *r.x = 1\n}\n");
    let (depth, operand) = deref_target(assign_parts(&stmts[0]).0);
    assert_eq!(depth, 1);
    let ExprKind::MemberAccess { target, member, .. } = &operand.kind else {
        panic!("expected '*' around 'r.x', got {:?}", operand);
    };
    assert_eq!((ident(target), member.as_str()), ("r", "x"));
}

#[test]
fn prefix_star_is_not_a_binary_operator_after_a_newline() {
    let stmts = block_stmts("fun f() {\n    val a = b\n    *r = 5\n}\n");
    assert_eq!(stmts.len(), 2);
    let StatementKind::Local { initializer, .. } = &stmts[0].kind else {
        panic!("expected a local, got {:?}", stmts[0].kind);
    };
    assert_eq!(ident(initializer), "b");
    assert_eq!(deref_target(assign_parts(&stmts[1]).0).0, 1);
}

#[test]
fn write_through_in_an_unbraced_branch_or_loop_body() {
    let stmts = block_stmts(
        "fun f(c: Bool, n: Int) {\n    if (c) *r = 1 else *r = 2\n    when (n) { 0 -> *r = 1, else -> {} }\n    while (c) *r = 0\n}\n",
    );
    let write = |e: &Expr| match &e.kind {
        ExprKind::Block(b) => {
            b.statements.len() == 1 && deref_target(assign_parts(&b.statements[0]).0).0 == 1
        }
        _ => false,
    };
    let ExprKind::If {
        then_branch,
        else_branch,
        ..
    } = &expr_of(&stmts[0]).kind
    else {
        panic!("expected an if, got {:?}", stmts[0].kind);
    };
    assert!(write(then_branch));
    assert!(write(else_branch.as_ref().expect("else branch")));
    let ExprKind::When { branches, .. } = &expr_of(&stmts[1]).kind else {
        panic!("expected a when, got {:?}", stmts[1].kind);
    };
    assert!(write(&branches[0].body));
    let StatementKind::While { body, .. } = &stmts[2].kind else {
        panic!("expected a while, got {:?}", stmts[2].kind);
    };
    assert_eq!(deref_target(assign_parts(&body.statements[0]).0).0, 1);
}

#[test]
fn prefix_star_outside_an_assignment_target_rejected() {
    for (src, column) in [
        ("fun f() { val x = *r }", 19),
        ("fun f() { g((*r)) }", 14),
        ("fun f() { return *r }", 18),
        ("fun f() { (*r).x = 1 }", 12),
        ("fun f() { *r }", 11),
        ("fun f() { *r == 1 }", 11),
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(
            err.message.contains("only allowed on the left of '='")
                && err
                    .message
                    .contains("reads through references are implicit"),
            "{}: {}",
            src,
            err
        );
        assert_eq!(err.span.column, column, "{}", src);
    }
}

#[test]
fn multiplication_unchanged() {
    for src in [
        "fun f(a: Int, b: Int) = a * b",
        "fun f(a: Int, b: Int) = a *b",
    ] {
        let ExprKind::Binary { op, .. } = expr_body(src).kind else {
            panic!("expected a multiplication: {}", src);
        };
        assert_eq!(op, BinaryOp::Mul, "{}", src);
    }
}

#[test]
fn deeply_nested_write_through_rejected() {
    let src = format!("fun f() {{ {}r = 1 }}", "*".repeat(100_000));
    assert_eq!(parse_source(&src).expect_err("too deep").code, "E2002");
}

// --- Extension Function Declarations, Receiver Mode Modifiers and the this Expression

/// The functions declared in `src`, in order.
fn functions(src: &str) -> Vec<FunctionDecl> {
    parse(src)
        .declarations
        .into_iter()
        .filter_map(|d| match d {
            Declaration::Function(f) => Some(f),
            _ => None,
        })
        .collect()
}

/// The receiver type name and mode of extension `f`.
#[track_caller]
fn receiver_of(f: &FunctionDecl) -> (String, ReceiverMode) {
    let receiver = f.receiver.as_ref().expect("an extension has a receiver");
    (receiver.ty.to_string(), receiver.mode)
}

#[test]
fn extension_declarations() {
    let fns = functions(
        "fun User.greet(): String = \"Hi \" + name\npublic fun Int.squared(): Int = this * this\npublic compose fun User.Avatar(): Unit { }\n",
    );
    let shapes: Vec<(&str, (String, ReceiverMode))> = fns
        .iter()
        .map(|f| (f.name.as_str(), receiver_of(f)))
        .collect();
    assert_eq!(
        shapes,
        vec![
            ("greet", ("User".to_string(), ReceiverMode::Shared)),
            ("squared", ("Int".to_string(), ReceiverMode::Shared)),
            ("Avatar", ("User".to_string(), ReceiverMode::Shared)),
        ]
    );
    assert!(fns[2].is_compose && fns[2].visibility == Visibility::Public);
    assert!(functions("fun greet(): String = \"x\"")[0]
        .receiver
        .is_none());
}

#[test]
fn unsupported_receivers_rejected() {
    for (src, expected) in [
        (
            "fun Heap<User>.size(): Int = 0",
            "calls look through 'Heap'",
        ),
        ("fun <T> T.also(): T = this", "bare type parameter"),
        ("fun User?.greet(): String = \"x\"", "call it with '?.'"),
        ("fun &User.greet(): String = \"x\"", "receiver mode"),
        ("fun &mut User.rename() { }", "receiver mode"),
        ("fun (User).greet() { }", "single type name"),
        ("fun models.User.greet(): String = \"x\"", "not qualified"),
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains(expected), "{}: {}", src, err);
    }
}

#[test]
fn mutating_and_consuming_extensions() {
    let fns = functions(
        "mut fun User.rename(n: String) { name = n }\npublic consuming fun User.toAccount(): Account = Account(this)\npackage compose mut fun Counter.Increment(): Unit { }\n",
    );
    let modes: Vec<ReceiverMode> = fns.iter().map(|f| receiver_of(f).1).collect();
    assert_eq!(
        modes,
        vec![
            ReceiverMode::Mutable,
            ReceiverMode::Consuming,
            ReceiverMode::Mutable
        ]
    );
    assert_eq!(fns[1].visibility, Visibility::Public);
    assert!(fns[2].is_compose && fns[2].visibility == Visibility::Package);
    // `package consuming fun` at the top of a file is a visibility, not a package declaration.
    let file = parse("package consuming fun User.take(): User = this\n");
    assert!(file.package.is_none());
    assert_eq!(
        receiver_of(&functions("package consuming fun User.take(): User = this")[0]).1,
        ReceiverMode::Consuming
    );
}

#[test]
fn receiver_modifier_without_a_receiver_rejected() {
    for src in ["mut fun f() { }", "consuming fun g(): Int = 1"] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(
            err.message
                .contains("modifies the receiver of an extension function"),
            "{}: {}",
            src,
            err
        );
    }
}

#[test]
fn modifiers_in_the_wrong_order_rejected() {
    for src in [
        "mut public fun User.a() { }",
        "consuming compose fun User.b() { }",
        "mut consuming fun User.c() { }",
        "mut mut fun User.d() { }",
        "mut struct S { val x: Int }",
    ] {
        assert_eq!(parse_source(src).expect_err(src).code, "E2001", "{}", src);
    }
}

#[test]
fn this_in_expressions() {
    assert!(matches!(
        expr_body("fun Int.squared(): Int = this * this").kind,
        ExprKind::Binary { ref left, ref right, .. }
            if left.kind == ExprKind::This && right.kind == ExprKind::This
    ));
    assert!(matches!(
        expr_body("fun User.n(): String = this.name").kind,
        ExprKind::MemberAccess { ref target, .. } if target.kind == ExprKind::This
    ));
    let ExprKind::Call { callee, .. } = expr_body("fun User.g(): String = this.greet()").kind
    else {
        panic!("expected a call");
    };
    assert!(matches!(
        callee.kind,
        ExprKind::MemberAccess { ref target, .. } if target.kind == ExprKind::This
    ));
    let ExprKind::Call { args, .. } = expr_body("fun User.u() = use(&this)").kind else {
        panic!("expected a call");
    };
    assert!(matches!(
        args[0].value.kind,
        ExprKind::Unary { op: UnaryOp::Borrow, ref expr } if expr.kind == ExprKind::This
    ));
}

#[test]
fn this_as_a_name_rejected() {
    for src in [
        "fun f() { val this = 1 }",
        "fun g(this: Int) { }",
        "fun this() { }",
        "fun f() { g(this = 1) }",
        "struct S { val this: Int }",
    ] {
        assert_eq!(parse_source(src).expect_err(src).code, "E2001", "{}", src);
    }
}

#[test]
fn consuming_as_an_identifier() {
    let fns = functions(
        "fun consuming(consuming: Int): Int = consuming\nfun f() { val consuming = 1; g(consuming = consuming) }\nfun h(x: Int) = x.consuming()\n",
    );
    assert_eq!(fns[0].name, "consuming");
    assert!(fns[0].receiver.is_none());
    assert_eq!(fns[0].params[0].name, "consuming");
    let FunctionBody::Expr(call) = fns[2].body.as_ref().expect("body") else {
        panic!("expected an expression body");
    };
    assert!(matches!(
        &call.kind,
        ExprKind::Call { callee, .. }
            if matches!(&callee.kind, ExprKind::MemberAccess { member, .. } if member == "consuming")
    ));
}

// --- Type Parameter Lists

/// The names and bounds of `parameters`, in order.
fn type_parameter_shapes(parameters: &[TypeParameter]) -> Vec<(&str, Option<TypeMarker>)> {
    parameters
        .iter()
        .map(|p| (p.name.as_str(), p.bound))
        .collect()
}

#[test]
fn type_parameter_lists() {
    let file = parse(
        "fun <T> id(x: T): T = x\npublic compose fun <T : Copy, U : Clone,> show(a: T, b: U) {}\nmut fun <T> Box<T>.put(v: T) {}\nstruct Pair<A, B>(val a: A, val b: B)\nenum Option<T> : Copy { Some(T), None }\n",
    );
    let shapes: Vec<Vec<(&str, Option<TypeMarker>)>> = file
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::Function(f) => type_parameter_shapes(&f.type_parameters),
            Declaration::Struct(s) => type_parameter_shapes(&s.type_parameters),
            Declaration::Enum(e) => type_parameter_shapes(&e.type_parameters),
            Declaration::Var(_) | Declaration::ExtensionProperty(_) => {
                panic!("no top-level val in the source")
            }
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            vec![("T", None)],
            vec![
                ("T", Some(TypeMarker::Copy)),
                ("U", Some(TypeMarker::Clone))
            ],
            vec![("T", None)],
            vec![("A", None), ("B", None)],
            vec![("T", None)],
        ]
    );
    let Declaration::Function(put) = &file.declarations[2] else {
        panic!("expected a function");
    };
    assert_eq!(put.name, "put");
    assert_eq!(
        receiver_of(put),
        ("Box<T>".to_string(), ReceiverMode::Mutable)
    );
}

#[test]
fn type_parameters_after_the_function_name_rejected() {
    for src in ["fun id<T>(x: T): T = x", "fun User.f<T>(x: T) {}"] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains("fun <T> id(...)"), "{}: {}", src, err);
    }
}

#[test]
fn malformed_type_parameter_lists_rejected() {
    for (src, expected) in [
        ("fun <> f() {}", "at least one type parameter"),
        ("fun <T : Eq> f(x: T) {}", "Only 'Copy' or 'Clone'"),
        ("fun <T : Copy, Clone> f(x: T) {}", "exactly one bound"),
        ("struct Box<T : Copy>(val v: T)", "cannot be bounded yet"),
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains(expected), "{}: {}", src, err);
    }
}

#[test]
fn receivers_with_type_arguments() {
    let fns = functions("fun <T> Box<T>.get(): &T = &v\nfun Box<Int>.sum(): Int = v\n");
    let receivers: Vec<String> = fns.iter().map(|f| receiver_of(f).0).collect();
    assert_eq!(receivers, vec!["Box<T>", "Box<Int>"]);
    assert_eq!(fns[0].name, "get");
    assert_eq!(fns[1].name, "sum");
}

// --- Explicit Type Arguments at Call Sites

/// The callee and written type arguments of call `e`.
#[track_caller]
fn call_shape(e: &Expr) -> (String, Vec<String>) {
    match &e.kind {
        ExprKind::Call {
            callee,
            type_arguments,
            ..
        } => (
            callee.to_string(),
            type_arguments.iter().map(|t| t.to_string()).collect(),
        ),
        other => panic!("expected a call, got {:?}", other),
    }
}

#[test]
fn type_arguments_on_calls() {
    let stmts = block_stmts(
        "fun f() {\n    id<Int>(1)\n    Box<String?>(null)\n    remember<Int> { 0 }\n    s.map<Box<Int>> { n -> Box(n) }\n    u?.pick<Int>(1)\n}\n",
    );
    let shapes: Vec<(String, Vec<String>)> = stmts.iter().map(|s| call_shape(expr_of(s))).collect();
    assert_eq!(
        shapes,
        vec![
            ("id".to_string(), vec!["Int".to_string()]),
            ("Box".to_string(), vec!["String?".to_string()]),
            ("remember".to_string(), vec!["Int".to_string()]),
            ("s.map".to_string(), vec!["Box<Int>".to_string()]),
            ("u?.pick".to_string(), vec!["Int".to_string()]),
        ]
    );
}

#[test]
fn type_arguments_inside_an_operand() {
    // The list binds like the call, tighter than the operators around it.
    let e = expr_body("fun f(): Int = a + id<Int>(1) * 2");
    assert_eq!(e.to_string(), "(a + (id<Int>(1) * 2))");
}

#[test]
fn comparisons_stay_comparisons() {
    assert_eq!(expr_body("fun f(): Bool = a < b").to_string(), "(a < b)");
    let stmts = block_stmts("fun f() {\n    if (i<n) f()\n    val m = a < b + 1\n}\n");
    let ExprKind::If { cond, .. } = &expr_of(&stmts[0]).kind else {
        panic!("expected an if");
    };
    assert_eq!(cond.to_string(), "(i < n)");
    let StatementKind::Local { initializer, .. } = &stmts[1].kind else {
        panic!("expected a local");
    };
    assert_eq!(initializer.to_string(), "(a < (b + 1))");
    let src = "fun f() { val k = a < b >= c }";
    let err = parse_source(src).expect_err(src);
    assert_eq!(err.code, "E2001");
    assert!(err.message.contains("cannot be chained"), "{}", err);
}

#[test]
fn comma_separated_comparisons_read_as_type_arguments() {
    let ExprKind::Call { args, .. } = expr_body("fun f(): Int = g(a < b, c > (d))").kind else {
        panic!("expected a call");
    };
    assert_eq!(args.len(), 1);
    assert_eq!(
        call_shape(&args[0].value),
        ("a".to_string(), vec!["b".to_string(), "c".to_string()])
    );
    let ExprKind::Call { args, .. } = expr_body("fun f(): Int = g((a < b), c > (d))").kind else {
        panic!("expected a call");
    };
    let written: Vec<String> = args.iter().map(|a| a.value.to_string()).collect();
    assert_eq!(written, vec!["(a < b)", "(c > d)"]);
}

#[test]
fn a_failed_speculation_leaves_the_token_stream_unchanged() {
    // `a < b >= c` tries `<b>` as a type-argument list closed by the `>` of `>=`; the rewind
    // keeps `>=` as one token, so the chained comparison is reported at `>=`.
    let src = "fun f() { val k = a < b >= c }";
    let err = parse_source(src).expect_err(src);
    assert_eq!(err.span.start, src.find(">=").unwrap());
    assert_eq!(err.span.end, src.find(">=").unwrap() + 2);
    // A split `&&` is restored too: `x < &&y` is a comparison with a double borrow.
    assert_eq!(
        expr_body("fun f(): Bool = x < &&y").to_string(),
        "(x < (&(&y)))"
    );
}

#[test]
fn type_arguments_then_a_line_break_are_not_a_call() {
    // `(x)` on its own line is not the call's argument list, so `f<Int>` is not a call.
    let src = "fun f() {\n    f<Int>\n    (x)\n}\n";
    let err = parse_source(src).expect_err(src);
    assert_eq!(err.code, "E2001");
}

#[test]
fn non_associative_comparison() {
    for src in ["fun f(): Bool = a < b < c", "fun f(): Bool = a < b > c"] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains("'&&'"), "{}: {}", src, err);
        let second = src.rfind(['<', '>']).unwrap();
        assert_eq!(err.span.start, second, "{}", src);
    }
    assert_eq!(
        expr_body("fun f(): Bool = (a < b) == c").to_string(),
        "((a < b) == c)"
    );
    assert_eq!(
        expr_body("fun f(): Bool = a < b == c > d").to_string(),
        "((a < b) == (c > d))"
    );
}

// --- Compound Assignment (`compound-assignment`)

/// The operator, target and value of a compound assignment statement.
#[track_caller]
fn compound_parts(item: &Statement) -> (BinaryOp, &Expr, &Expr) {
    match &item.kind {
        StatementKind::CompoundAssign {
            operator,
            target,
            value,
        } => (*operator, target, value),
        other => panic!("expected a compound assignment, got {:?}", other),
    }
}

#[test]
fn compound_assignment_statements() {
    let stmts = block_stmts(
        "fun f() {\n    x += 1\n    x -= 1\n    x *= 2\n    x /= 2\n    x %= 2\n    u.count += 1\n    xs[i] += 1\n    *r += 1\n    count += 1\n}\n",
    );
    let operators: Vec<BinaryOp> = stmts.iter().map(|s| compound_parts(s).0).collect();
    assert_eq!(
        operators,
        vec![
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Mod,
            BinaryOp::Add,
            BinaryOp::Add,
            BinaryOp::Add,
            BinaryOp::Add,
        ]
    );
    let (_, target, value) = compound_parts(&stmts[0]);
    assert_eq!((ident(target), int(value)), ("x", 1));
    assert!(matches!(
        compound_parts(&stmts[5]).1.kind,
        ExprKind::MemberAccess { .. }
    ));
    assert!(matches!(
        compound_parts(&stmts[6]).1.kind,
        ExprKind::Index { .. }
    ));
    let (depth, operand) = deref_target(compound_parts(&stmts[7]).1);
    assert_eq!((depth, ident(operand)), (1, "r"));
    assert_eq!(ident(compound_parts(&stmts[8]).1), "count");
}

#[test]
fn compound_value_binds_looser_than_every_operator() {
    let stmts = block_stmts("fun f() {\n    x += a ?: b * c\n    *r.n += 1\n}\n");
    let (_, _, value) = compound_parts(&stmts[0]);
    assert_eq!(value.to_string(), "(a ?: (b * c))");
    let (depth, operand) = deref_target(compound_parts(&stmts[1]).1);
    assert_eq!(depth, 1);
    let ExprKind::MemberAccess { target, member, .. } = &operand.kind else {
        panic!("expected '*' around 'r.n', got {:?}", operand);
    };
    assert_eq!((ident(target), member.as_str()), ("r", "n"));
}

#[test]
fn compound_assignment_in_an_unbraced_body() {
    let stmts = block_stmts(
        "fun f(c: Bool, n: Int) {\n    if (c) x += 1 else x -= 1\n    when (n) { 0 -> x += 1, else -> {} }\n    while (c) x *= 2\n}\n",
    );
    let compound = |e: &Expr, operator: BinaryOp| match &e.kind {
        ExprKind::Block(b) => {
            b.tail.is_none()
                && b.statements.len() == 1
                && compound_parts(&b.statements[0]).0 == operator
        }
        _ => false,
    };
    let ExprKind::If {
        then_branch,
        else_branch,
        ..
    } = &expr_of(&stmts[0]).kind
    else {
        panic!("expected an if, got {:?}", stmts[0].kind);
    };
    assert!(compound(then_branch, BinaryOp::Add));
    assert!(compound(
        else_branch.as_ref().expect("else branch"),
        BinaryOp::Sub
    ));
    let ExprKind::When { branches, .. } = &expr_of(&stmts[1]).kind else {
        panic!("expected a when, got {:?}", stmts[1].kind);
    };
    assert!(compound(&branches[0].body, BinaryOp::Add));
    let StatementKind::While { body, .. } = &stmts[2].kind else {
        panic!("expected a while, got {:?}", stmts[2].kind);
    };
    assert!(body.tail.is_none());
    assert_eq!(compound_parts(&body.statements[0]).0, BinaryOp::Mul);
}

#[test]
fn invalid_compound_assignment_target_rejected() {
    for (src, column) in [
        ("fun f() { a + b += 1 }", 11),
        ("fun f() { 1 += 2 }", 11),
        ("fun f() { g() += 1 }", 11),
        ("fun f() { a?.b += 1 }", 11),
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains("Invalid assignment target"), "{}", err);
        assert_eq!(err.span.column, column, "{}", src);
    }
}

#[test]
fn compound_assignment_as_a_value_rejected() {
    for (src, message) in [
        (
            "fun f() { var x = 0; val y = (x += 1) }",
            "assignment is a statement",
        ),
        (
            "fun f() { var x = 0; g(x += 1) }",
            "assignment is a statement",
        ),
        (
            "fun f() { var x = 0; val y = x += 1 }",
            "assignment is a statement",
        ),
        (
            "fun f() { var a = 0; var b = 0; a += b += 1 }",
            "Chained assignments",
        ),
    ] {
        let err = parse_source(src).expect_err(src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert!(err.message.contains(message), "{}: {}", src, err);
        let at = &src[err.span.start..err.span.end];
        assert_eq!(at, "+=", "{}", src);
    }
}

#[test]
fn a_newline_before_a_compound_operator_ends_the_expression() {
    let src = "fun f() {\nvar x = 0\nx\n+= 1\n}\n";
    let err = parse_source(src).expect_err(src);
    assert_eq!(err.code, "E2001");
    assert_eq!((err.span.line, err.span.column), (4, 1));
}

// --- No Increment or Decrement Operators (`compound-assignment`)

#[test]
fn increment_and_decrement_rejected() {
    for (statement, operator, help) in [
        ("x++", "++", "x += 1"),
        ("x--", "--", "x -= 1"),
        ("++x", "++", "x += 1"),
        ("--x", "--", "x -= 1"),
    ] {
        let src = format!("fun f() {{\n    {}\n}}\n", statement);
        let err = parse_source(&src).expect_err(&src);
        assert_eq!(err.code, "E2001", "{}", src);
        assert_eq!(&src[err.span.start..err.span.end], operator, "{}", src);
        assert!(err.message.contains(help), "{}: {}", src, err);
    }
}

#[test]
fn double_minus_needs_a_space() {
    let src = "fun f(a: Int, b: Int): Int = a--b";
    let err = parse_source(src).expect_err(src);
    assert_eq!(err.code, "E2001");
    assert_eq!(&src[err.span.start..err.span.end], "--");
    assert_eq!(
        expr_body("fun g(a: Int, b: Int): Int = a - -b").to_string(),
        "(a - (-b))"
    );
    assert_eq!(
        expr_body("fun h(x: Int): Int = -(-x)").to_string(),
        "(-(-x))"
    );
}

// --- String Templates (`string-templates`)

/// The initializers of the top-level `val`s of `src`, in order.
fn initializers(src: &str) -> Vec<Expr> {
    parse(src)
        .declarations
        .into_iter()
        .map(|declaration| match declaration {
            Declaration::Var(var) => var.initializer,
            other => panic!("expected a top-level val, got {:?}", other),
        })
        .collect()
}

/// The parts of a template expression.
#[track_caller]
fn template_parts(e: &Expr) -> &[TemplatePart] {
    match &e.kind {
        ExprKind::Template(parts) => parts,
        other => panic!("expected a template, got {:?}", other),
    }
}

#[track_caller]
fn text_part(part: &TemplatePart) -> &str {
    match part {
        TemplatePart::Text(text) => text,
        other => panic!("expected a text part, got {:?}", other),
    }
}

#[track_caller]
fn expression_part(part: &TemplatePart) -> &Expr {
    match part {
        TemplatePart::Expression(e) => e,
        other => panic!("expected an expression part, got {:?}", other),
    }
}

#[test]
fn template_expression_tree() {
    let src = r#"val s = "Hi $name, ${a + b}!".length"#;
    let initializer = &initializers(src)[0];
    let ExprKind::MemberAccess { target, member, .. } = &initializer.kind else {
        panic!("expected a member access, got {:?}", initializer.kind);
    };
    assert_eq!(member, "length");
    let parts = template_parts(target);
    assert_eq!(parts.len(), 5);
    assert_eq!(text_part(&parts[0]), "Hi ");
    let name = expression_part(&parts[1]);
    assert_eq!(ident(name), "name");
    // A name part spans the name without its `$`.
    assert_eq!(&src[name.span.start..name.span.end], "name");
    assert_eq!(text_part(&parts[2]), ", ");
    let ExprKind::Binary { op, left, right } = &expression_part(&parts[3]).kind else {
        panic!("expected a binary expression");
    };
    assert_eq!((*op, ident(left), ident(right)), (BinaryOp::Add, "a", "b"));
    assert_eq!(text_part(&parts[4]), "!");
    // The template spans from its opening to its closing quote.
    assert_eq!(
        &src[target.span.start..target.span.end],
        r#""Hi $name, ${a + b}!""#
    );
}

#[test]
fn name_and_braced_parts_are_the_same_expression() {
    let found = initializers("val a = \"$x\"\nval b = \"${x}\"\nval c = \"$this\"");
    for initializer in &found[..2] {
        let parts = template_parts(initializer);
        assert_eq!(parts.len(), 1);
        assert_eq!(ident(expression_part(&parts[0])), "x");
    }
    assert_eq!(
        expression_part(&template_parts(&found[2])[0]).kind,
        ExprKind::This
    );
    // A literal without parts stays a string literal.
    assert_eq!(
        initializers("val s = \"$5\"")[0].kind,
        ExprKind::Literal(Literal::String("$5".to_string()))
    );
}

#[test]
fn newlines_inside_an_expression_part() {
    let found = initializers("val s = \"${a +\n    b}\"");
    let parts = template_parts(&found[0]);
    assert_eq!(parts.len(), 1);
    let ExprKind::Binary { op, left, right } = &expression_part(&parts[0]).kind else {
        panic!("expected a binary expression");
    };
    assert_eq!((*op, ident(left), ident(right)), (BinaryOp::Add, "a", "b"));
}

#[test]
fn nested_templates_and_braces_parse() {
    let src = r#"val s = "${when (n) { 1 -> "}"; else -> "$n {" }} done""#;
    let found = initializers(src);
    let parts = template_parts(&found[0]);
    assert_eq!(parts.len(), 2);
    assert!(matches!(
        expression_part(&parts[0]).kind,
        ExprKind::When { .. }
    ));
    assert_eq!(text_part(&parts[1]), " done");
}

#[test]
fn an_empty_or_statement_part_rejected() {
    for src in [
        "val s = \"${}\"",
        "val t = \"${val x = 1}\"",
        "val u = \"${a b}\"",
    ] {
        let error = parse_source(src).expect_err(src);
        assert_eq!(error.code, "E2001", "{}", src);
    }
    let error = parse_source("val s = \"${}\"").unwrap_err();
    assert!(error.message.contains("Expected an expression in '${}'"));
    let error = parse_source("val s = \"${a b}\"").unwrap_err();
    assert!(error.message.contains("to close '${'"), "{}", error.message);
}

// --- Library declarations (`standard-library`, D6 and D12)

/// syntax: "Library declarations parse".
#[test]
fn library_declarations_parse() {
    let file = parse(
        "private external fun prism_rt_exit(status: Int): Nothing\n\
         public intrinsic fun <T> print(value: &T)\n\
         public intrinsic mut fun <T> List<T>.add(element: T)\n\
         public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T\n",
    );
    let shapes: Vec<_> = file
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::Function(f) => (
                f.library.map(|m| m.kind),
                f.visibility,
                f.receiver.as_ref().map(|r| r.mode),
                f.is_compose,
                f.body.is_none(),
            ),
            _ => panic!("expected functions"),
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            (
                Some(LibraryKind::External),
                Visibility::Private,
                None,
                false,
                true
            ),
            (
                Some(LibraryKind::Intrinsic),
                Visibility::Public,
                None,
                false,
                true
            ),
            (
                Some(LibraryKind::Intrinsic),
                Visibility::Public,
                Some(ReceiverMode::Mutable),
                false,
                true
            ),
            (
                Some(LibraryKind::Intrinsic),
                Visibility::Public,
                None,
                true,
                true
            ),
        ]
    );
}

/// syntax: "Intrinsic properties parse".
#[test]
fn intrinsic_properties_parse() {
    let file = parse(
        "public intrinsic val String.length: Int\npublic intrinsic val <T> List<T>.size: Int\n",
    );
    let shapes: Vec<(String, String, String, usize)> = file
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::ExtensionProperty(p) => (
                p.receiver.to_string(),
                p.name.clone(),
                p.ty.to_string(),
                p.type_parameters.len(),
            ),
            _ => panic!("expected properties"),
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            ("String".into(), "length".into(), "Int".into(), 0),
            ("List<T>".into(), "size".into(), "Int".into(), 1),
        ]
    );
}

/// syntax: "A writable intrinsic property parses".
#[test]
fn a_writable_intrinsic_property_parses() {
    let file = parse("public intrinsic var <T> MutableState<T>.value: T\npublic intrinsic val String.length: Int\n");
    let shapes: Vec<(bool, String, String, String, usize)> = file
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::ExtensionProperty(p) => (
                p.writable,
                p.receiver.to_string(),
                p.name.clone(),
                p.ty.to_string(),
                p.type_parameters.len(),
            ),
            _ => panic!("expected properties"),
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            (
                true,
                "MutableState<T>".into(),
                "value".into(),
                "T".into(),
                1
            ),
            (false, "String".into(), "length".into(), "Int".into(), 0),
        ]
    );
}

/// syntax: "Writable extension properties need intrinsic".
#[test]
fn writable_extension_properties_need_intrinsic() {
    for source in ["var Int.twice: Int\n", "intrinsic var Int.twice: Int = 2\n"] {
        let error = prism_parser::parse_source(source).expect_err(source);
        assert_eq!(error.code, "E2001", "{}", source);
    }
}

/// syntax: "external and intrinsic as identifiers".
#[test]
fn external_and_intrinsic_as_identifiers() {
    let file = parse(
        "fun external(intrinsic: Int): Int = intrinsic\nfun f() { val external = 1; g(intrinsic = external) }\nfun h(x: X) = x.intrinsic()\n",
    );
    assert_eq!(file.declarations.len(), 3);
    for declaration in &file.declarations {
        let Declaration::Function(f) = declaration else {
            panic!("expected functions");
        };
        assert!(f.library.is_none() && f.body.is_some());
    }
}

/// syntax: "A function without a body rejected", "A library declaration with a body rejected"
/// and "Library modifiers in the wrong order rejected".
#[test]
fn misplaced_library_forms_are_rejected() {
    for source in [
        "fun a(): Int\nfun b() {}\n",
        "external fun f(): Int = 1\n",
        "intrinsic fun g() { }\n",
        "compose intrinsic fun A() { }\n",
        "intrinsic public fun b(): Int\n",
        "external intrinsic fun c(): Int\n",
        "mut external fun Int.d()\n",
        "val String.twice: Int = 2\n",
        "intrinsic val String.a: Int = 1\n",
    ] {
        let error = prism_parser::parse_source(source).expect_err(source);
        assert_eq!(error.code, "E2001", "{}", source);
    }
    let error = prism_parser::parse_source("fun a(): Int\nfun b() {}\n").unwrap_err();
    assert_eq!(
        (error.span.line, error.span.column),
        (1, 10),
        "at the end of the signature"
    );
}

// --- operator-conventions ---

/// The only function declared by `src`.
fn function(src: &str) -> FunctionDecl {
    match parse(src).declarations.into_iter().next() {
        Some(Declaration::Function(f)) => f,
        other => panic!("expected a function, got {:?}", other),
    }
}

/// Scenario "Operator declarations parse".
#[test]
fn operator_declarations_parse() {
    let plus = function("operator fun Vector2.plus(other: Vector2): Vector2 = other");
    assert!(plus.is_operator);
    assert_eq!(plus.visibility, Visibility::Private);
    assert_eq!(plus.receiver.as_ref().unwrap().mode, ReceiverMode::Shared);
    let get =
        function("public operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]");
    assert!(get.is_operator);
    assert_eq!(get.visibility, Visibility::Public);
    assert_eq!(get.receiver.as_ref().unwrap().mode, ReceiverMode::Mutable);
    let text = function("operator consuming fun Text.plus(other: &Text): Text = this");
    assert!(text.is_operator);
    assert_eq!(
        text.receiver.as_ref().unwrap().mode,
        ReceiverMode::Consuming
    );
    let contains =
        function("public intrinsic operator fun <T> List<T>.contains(element: &T): Bool");
    assert!(contains.is_operator && contains.is_intrinsic());
    assert_eq!(contains.visibility, Visibility::Public);
    assert!(!function("fun Vector2.plus(other: Vector2): Vector2 = other").is_operator);
}

/// Scenario "Operator modifiers in the wrong order rejected".
#[test]
fn operator_modifiers_in_the_wrong_order_rejected() {
    for src in [
        "operator public fun P.plus(o: P): P = o",
        "mut operator fun P.minus(o: P): P = o",
        "operator operator fun P.times(o: P): P = o",
        "operator intrinsic fun P.div(o: P): P",
    ] {
        let error = parse_source(src).expect_err(src);
        assert_eq!(error.code, "E2001", "{}", src);
    }
}

/// Scenario "operator as an identifier".
#[test]
fn operator_as_an_identifier() {
    let f = function("fun operator(operator: Int): Int = operator");
    assert_eq!(f.name, "operator");
    assert!(!f.is_operator);
    assert_eq!(f.params[0].name, "operator");
    let items = block_stmts("fun f() { val operator = 1; g(operator = operator) }");
    let ExprKind::Call { args, .. } = &expr_of(&items[1]).kind else {
        panic!("expected a call");
    };
    assert_eq!(args[0].name.as_deref(), Some("operator"));
    assert_eq!(ident(&args[0].value), "operator");
    let call = expr_body("fun f(): Int = x.operator()");
    let ExprKind::Call { callee, .. } = &call.kind else {
        panic!("expected a call");
    };
    assert!(matches!(&callee.kind, ExprKind::MemberAccess { member, .. } if member == "operator"));
}

/// Scenario "Membership precedence".
#[test]
fn membership_precedence() {
    let shape = |body: &str| expr_body(&format!("fun f(): Bool = {}", body)).to_string();
    assert_eq!(shape("x in xs == true"), "((x in xs) == true)");
    assert_eq!(shape("a ?: b in xs"), "((a ?: b) in xs)");
    assert_eq!(shape("n in 0..<k + 1"), "(n in (0 ..< (k + 1)))");
    assert_eq!(shape("x !in xs || y"), "((x !in xs) || y)");
    let negated = expr_body("fun f(): Bool = !(x in xs)");
    let ExprKind::Unary {
        op: UnaryOp::Not,
        expr,
    } = &negated.kind
    else {
        panic!("expected a negation, got {}", negated);
    };
    assert!(matches!(
        expr.kind,
        ExprKind::Binary {
            op: BinaryOp::In,
            ..
        }
    ));
}

/// Scenario "Chained membership rejected".
#[test]
fn chained_membership_rejected() {
    for (src, second) in [
        (
            "fun f(): Bool = a in b in c",
            "fun f(): Bool = a in b ".len(),
        ),
        (
            "fun f(): Bool = a !in b in c",
            "fun f(): Bool = a !in b ".len(),
        ),
    ] {
        let error = parse_source(src).expect_err(src);
        assert_eq!(error.code, "E2001");
        assert_eq!(error.span.start, second, "{}", src);
    }
    assert_eq!(
        expr_body("fun f(): Bool = (a in b) in c").to_string(),
        "((a in b) in c)"
    );
}

/// Scenario "Loops and patterns keep their in".
#[test]
fn loops_and_patterns_keep_their_in() {
    let items = block_stmts("fun f() { for (x in xs) {} }");
    let StatementKind::For {
        variable, iterable, ..
    } = &items[0].kind
    else {
        panic!("expected a loop");
    };
    assert_eq!(variable, "x");
    assert_eq!(ident(iterable), "xs");
    let when = expr_body("fun f(n: Int): Int = when (n) { in 0..<10 -> 1; else -> 2 }");
    let ExprKind::When { branches, .. } = &when.kind else {
        panic!("expected a when");
    };
    assert!(matches!(
        &branches[0].patterns[0],
        WhenPattern::Pattern(Pattern {
            kind: PatternKind::InRange { .. },
            ..
        })
    ));
}

/// Scenario "Several indices".
#[test]
fn several_indices() {
    let indices = |e: &Expr| match &e.kind {
        ExprKind::Index { target, indices } => (target.to_string(), indices.len()),
        _ => panic!("expected an index, got {}", e),
    };
    assert_eq!(
        indices(&expr_body("fun f(): Int = m[1, 2]")),
        ("m".into(), 2)
    );
    assert_eq!(
        indices(&expr_body("fun f(): Int = m[1, 2,]")),
        ("m".into(), 2)
    );
    let items = block_stmts("fun f() { m[r, c] = 0.0; m[i, i + 1] += 1.0 }");
    match &items[0].kind {
        StatementKind::Assign { target, .. } => assert_eq!(indices(target), ("m".into(), 2)),
        other => panic!("expected an assignment, got {:?}", other),
    }
    match &items[1].kind {
        StatementKind::CompoundAssign { target, .. } => {
            assert_eq!(indices(target), ("m".into(), 2))
        }
        other => panic!("expected a compound assignment, got {:?}", other),
    }
    let nested = expr_body("fun f(): Int = grid[0][1]");
    let ExprKind::Index { target, indices } = &nested.kind else {
        panic!("expected an index");
    };
    assert_eq!(indices.len(), 1);
    assert!(matches!(target.kind, ExprKind::Index { .. }));
}

/// Scenario "An empty index list rejected".
#[test]
fn an_empty_index_list_rejected() {
    let src = "fun f(m: M): Int = m[]";
    let error = parse_source(src).expect_err(src);
    assert_eq!(error.code, "E2001");
    assert_eq!(error.span.start, src.len() - 1);
}

/// syntax: "Extension property getters parse".
#[test]
fn extension_property_getters_parse() {
    let file = parse(concat!(
        "public val Int.dp: Dp get() = Dp(this)\n",
        "val <T> List<T>.second: &T\n",
        "    get() = &this[1]\n",
        "private val Box.area: Int get() { return w * h }\n",
    ));
    let shapes: Vec<(Visibility, usize, String, String, String, bool, bool)> = file
        .declarations
        .iter()
        .map(|d| match d {
            Declaration::ExtensionProperty(p) => (
                p.visibility,
                p.type_parameters.len(),
                p.receiver.to_string(),
                p.name.clone(),
                p.ty.to_string(),
                p.is_intrinsic(),
                matches!(
                    p.getter.as_ref().expect("a getter").body,
                    FunctionBody::Expr(_)
                ),
            ),
            _ => panic!("expected properties"),
        })
        .collect();
    assert_eq!(
        shapes,
        vec![
            (
                Visibility::Public,
                0,
                "Int".into(),
                "dp".into(),
                "Dp".into(),
                false,
                true
            ),
            (
                Visibility::Private,
                1,
                "List<T>".into(),
                "second".into(),
                "&T".into(),
                false,
                true
            ),
            (
                Visibility::Private,
                0,
                "Box".into(),
                "area".into(),
                "Int".into(),
                false,
                false
            ),
        ]
    );
}

/// syntax: "Extension property getters parse": the getter on the line after the type, after
/// several line breaks, and a declaration following the getter on its own line.
#[test]
fn extension_property_getters_on_later_lines() {
    let file = parse("val Int.twice: Int\n\n\n    get() = this * 2\nfun f(): Int = 1.twice\n");
    assert_eq!(file.declarations.len(), 2);
    let Declaration::ExtensionProperty(property) = &file.declarations[0] else {
        panic!("expected a property");
    };
    let Some(Getter {
        body: FunctionBody::Expr(body),
        ..
    }) = &property.getter
    else {
        panic!("expected an expression getter");
    };
    assert_eq!(body.to_string(), "(this * 2)");
    let file = parse("val Int.twice: Int get() {\n    return this * 2\n}\nval x = 1\n");
    assert_eq!(file.declarations.len(), 2);
}

/// syntax: "get as an identifier".
#[test]
fn get_as_an_identifier() {
    let file = parse(concat!(
        "fun get(get: Int): Int = get\n",
        "val get = 1\n",
        "fun f(b: Bag) { b.get(0); val x = get }\n",
        "struct S { val get: Int }\n",
    ));
    assert_eq!(file.declarations.len(), 4);
}

/// syntax: "Extension properties without a getter rejected".
#[test]
fn extension_properties_without_a_getter_rejected() {
    for (source, needle) in [
        ("val String.twice: Int\nfun f() {}\n", "needs a getter"),
        ("val Int.half: Int = 2\n", "get() = 2"),
        ("var Int.twice: Int get() = 2\n", "read-only"),
        ("compose val Int.x: Int get() = 1\n", "compose"),
        (
            "val <T, R> List<T>.other: Int get() = 0\n",
            "'R' does not occur in the receiver type",
        ),
    ] {
        let error = prism_parser::parse_source(source).expect_err(source);
        assert_eq!(error.code, "E2001", "{}", source);
        assert!(
            error.message.contains(needle),
            "{}: {}",
            source,
            error.message
        );
    }
}
