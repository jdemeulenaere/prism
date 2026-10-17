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
    assert!(matches!(&zero.patterns[..], [WhenPattern::Expr(e)] if int(e) == 0));
    let few_values: Vec<_> = few
        .patterns
        .iter()
        .map(|p| match p {
            WhenPattern::Expr(e) => int(e),
            other => panic!("expected a value pattern, got {:?}", other),
        })
        .collect();
    assert_eq!(few_values, vec![1, 2, 3]);
    match &small.patterns[..] {
        [WhenPattern::InRange {
            start,
            end,
            end_inclusive: true,
        }] => assert_eq!((int(start), int(end)), (4, 10)),
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
    assert!(matches!(&branches[0].patterns[..], [WhenPattern::Expr(e)] if int(e) == 0));
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
        [WhenPattern::Expr(e)] if matches!(e.kind, ExprKind::Binary { op: BinaryOp::Lt, .. })
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
        [WhenPattern::InRange {
            start,
            end,
            end_inclusive: false,
        }] => assert_eq!((int(start), int(end)), (0, 10)),
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
    assert_eq!(tagged.type_params, vec!["T"]);
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
        ("fun f() { g(*r) }", 13),
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
