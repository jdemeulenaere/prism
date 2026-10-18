//! The callee's half of the elision rules on small files lowered from source: what a body may
//! return a borrow of, with codes, messages, labels, notes and helps (`borrowck-elision`).

use prism_borrowck::{check_file, BorrowError};

/// The MIR of `source`, a file `main` that must typecheck.
fn lower(source: &str) -> prism_mir::MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main");
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

/// The borrow errors of `source`, which must have some.
fn errors(source: &str) -> Vec<BorrowError> {
    match check_file(&lower(source)) {
        Ok(_) => panic!("expected borrow errors in:\n{}", source),
        Err(errors) => errors,
    }
}

/// An error as text: `code message @line:column label`, then each related label, note and help.
fn render(error: &BorrowError) -> String {
    let mut out = format!(
        "{} {} @{}:{} {}",
        error.code(),
        error.message(),
        error.primary.span.line,
        error.primary.span.column,
        error.primary.message
    );
    for label in &error.related {
        out.push_str(&format!(
            "\n  related @{}:{} {}",
            label.span.line, label.span.column, label.message
        ));
    }
    for note in &error.notes {
        out.push_str(&format!("\n  note: {}", note));
    }
    for help in &error.helps {
        out.push_str(&format!("\n  help: {}", help));
    }
    out
}

fn rendered(source: &str) -> String {
    errors(source)
        .iter()
        .map(render)
        .collect::<Vec<_>>()
        .join("\n")
}

fn passes(source: &str) {
    if let Err(errors) = check_file(&lower(source)) {
        panic!(
            "unexpected borrow errors:\n{}",
            errors.iter().map(render).collect::<Vec<_>>().join("\n")
        );
    }
}

#[test]
fn a_returned_local_is_dangling_under_rule_4() {
    let actual = rendered("fun f(): &Int { val n = 1; val r = &n; return r }\n");
    assert_eq!(actual, "\
E5008 cannot return a borrow of local 'n' @1:47 returned here
  related @1:36 'n' is borrowed here
  note: 'f' takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)
  help: return the value itself instead of a borrow of it", "\n{}", actual);
}

#[test]
fn a_returned_by_value_parameter_is_dangling() {
    let actual = rendered("fun g(s: String): &String = &s\n");
    assert_eq!(actual, "\
E5008 cannot return a borrow of parameter 's' @1:29 returned here
  note: 'g' takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)
  help: take 's' as a reference parameter ('&String') so that the result can borrow the caller's value", "\n{}", actual);
}

#[test]
fn a_returned_temporary_is_dangling() {
    let actual = rendered("struct Team { var lead: String }\nfun make(): String = \"x\"\nfun g(t: &Team): &String = &make()\n");
    assert_eq!(
        actual,
        "\
E5008 cannot return a borrow of a temporary value @3:28 returned here
  help: return the value itself instead of a borrow of it",
        "\n{}",
        actual
    );
}

#[test]
fn a_top_level_value_cannot_borrow_its_temporary() {
    let actual = rendered("val R: &Int = &1\n");
    assert_eq!(
        actual,
        "\
E5008 cannot return a borrow of a temporary value @1:15 assigned to a top-level value here
  help: declare the value as a top-level 'val' of its own and borrow that",
        "\n{}",
        actual
    );
}

#[test]
fn a_method_returning_another_parameter_is_untied() {
    let actual =
        rendered("struct Team { var lead: String }\nfun Team.other(o: &String): &String = o\n");
    assert_eq!(actual, "\
E5009 method 'other' cannot return a borrow derived from parameter 'o' @2:39 returned here
  related @2:16 'o' is a parameter here
  note: a method's returned borrow is tied to its receiver only (elision rule 2), so callers keep only the receiver borrowed
  help: return a borrow derived from the receiver, or declare a function taking the receiver as a reference parameter, whose result is tied to every reference parameter (elision rule 3)", "\n{}", actual);
}

#[test]
fn a_function_typed_parameter_result_is_untied() {
    let actual = rendered("fun call(h: () -> &Int): &Int = h()\n");
    assert_eq!(actual, "\
E5009 cannot return a borrow obtained from function-typed parameter 'h' @1:33 returned here
  related @1:10 'h' is a parameter here
  note: a function-typed parameter is not a borrowed input, so callers do not keep borrowed what the function passed for it borrows", "\n{}", actual);
}

#[test]
fn a_closure_with_a_borrowed_parameter_cannot_return_a_capture() {
    let actual = rendered(
        "fun app(g: (&Int) -> &Int): Int = 1\nfun f(): Int { val n = 1; return app { k -> &n } }\n",
    );
    assert_eq!(actual, "\
E5009 the closure cannot return a borrow derived from captured 'n' @2:45 returned here
  related @2:46 'n' is captured here
  note: the closure's type takes a borrowed input, so its returned borrow is tied to its reference parameters only (elision rules 1 and 3) and callers do not keep what it captures borrowed
  help: pass 'n' to the closure as a reference parameter instead", "\n{}", actual);
}

#[test]
fn a_default_borrowing_a_by_value_parameter_is_untied() {
    let actual = rendered("fun m(a: String, b: &String = a): Int = 1\n");
    assert_eq!(actual, "\
E5009 default value of 'b' cannot borrow parameter 'a' @1:31 returned as a default value here
  related @1:7 'a' is a parameter here
  note: the call passes 'a' itself, so a default can only borrow what a shared reference parameter refers to
  help: declare 'a' as a reference parameter ('&String')", "\n{}", actual);
}

#[test]
fn a_default_borrowing_a_mutable_receiver_is_untied() {
    let actual =
        rendered("struct User { var name: String }\nmut fun User.rename(n: &String = name) {}\n");
    assert_eq!(actual, "\
E5009 default value of 'n' cannot borrow from 'this', which the call borrows mutably @2:34 returned as a default value here
  related @2:1 'this' is the receiver here
  note: the call passes 'this' as a mutable borrow beside the default, which would alias it
  help: pass 'n' explicitly at each call, or give it a default that does not borrow 'this'", "\n{}", actual);
}

#[test]
fn a_write_holding_a_dangling_loan_and_an_untied_input_reports_once() {
    let actual = rendered("struct Team { var lead: String }\nfun first(a: &String, b: &String): &String = a\nfun Team.pick(o: &String): &String { val s = \"x\"; return first(o, &s) }\n");
    assert_eq!(
        actual,
        "\
E5008 cannot return a borrow of local 's' @3:58 returned here
  related @3:67 's' is borrowed here
  help: return the value itself instead of a borrow of it",
        "\n{}",
        actual
    );
}

#[test]
fn borrows_through_arguments_and_of_top_level_values_are_accepted() {
    passes(
        "struct User { val name: String }\nval DEFAULT = User(\"guest\")\nval LIMIT = 10\nval REF: &Int = &LIMIT\nstruct Team { var lead: String }\nfun lead(t: &Team): &String = &t.lead\nfun Team.captain(o: &String): &String = &lead\nfun guest(): &User = &DEFAULT\nfun orGuest(u: &User, c: Bool): &User = if (c) u else &DEFAULT\nfun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)\nfun app(g: () -> &Int): Int = 1\nfun f(): Int { val n = 1; return app { &n } }\nfun hi(u: &User, n: &String = u.name): &String = n\n",
    );
}

#[test]
fn a_closure_with_a_borrowed_parameter_cannot_return_a_captured_reference() {
    let errors =
        errors("fun app(g: (&Int) -> &Int): Int = 1\nfun f(r: &Int): Int = app { k -> r }\n");
    let codes: Vec<&str> = errors.iter().map(|error| error.code()).collect();
    assert_eq!(codes, vec!["E5009"], "{}", render(&errors[0]));
    assert_eq!(
        errors[0].message(),
        "the closure cannot return a borrow derived from captured 'r'"
    );
}
