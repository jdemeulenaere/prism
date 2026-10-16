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

/// The statements of the only function's block body in `src`.
fn block_stmts(src: &str) -> Vec<Expr> {
    let file = parse(src);
    match &file.declarations[0] {
        Declaration::Function(f) => match f.body.as_ref().expect("function has a body") {
            FunctionBody::Block(b) => b.stmts.clone(),
            FunctionBody::Expr(_) => panic!("expected a block body"),
        },
        other => panic!("expected a function, got {:?}", other),
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
    let ExprKind::LocalVar {
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
        [WhenPattern::InRange { start, end }] => assert_eq!((int(start), int(end)), (4, 10)),
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
        matches!(&stmts[0].kind, ExprKind::LocalVar { is_var: false, name, .. } if name == "a")
    );
    assert!(matches!(&stmts[1].kind, ExprKind::LocalVar { is_var: true, name, .. } if name == "b"));
    match &stmts[2].kind {
        ExprKind::Assign { target, value } => {
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
    } = &stmts[0].kind
    else {
        panic!("expected an if, got {:?}", stmts[0].kind);
    };
    assert_eq!(ident(cond), "isValid");
    assert!(else_branch.is_none());
    match &then_branch.kind {
        ExprKind::Block(b) => {
            assert_eq!(b.stmts.len(), 1);
            assert!(
                matches!(&b.stmts[0].kind, ExprKind::Call { callee, .. } if ident(callee) == "doWork")
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
