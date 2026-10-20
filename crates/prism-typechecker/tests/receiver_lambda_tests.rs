//! The messages and helps of the diagnostics of `receiver-lambdas`, driven by real Prism source
//! through the parser and the checker. The fixtures of `prism-cli` check the codes; these check
//! what the scenarios say the diagnostics name.

use prism_typechecker::{TypeChecker, TypeError};

/// Parses and checks a single source file, returning its diagnostics.
fn check(src: &str) -> Vec<TypeError> {
    let file = prism_parser::parse_source(src).expect("source must parse");
    let mut checker = TypeChecker::new(None, "main").with_library_imports();
    checker.check_module(&file);
    checker.errors
}

/// The only diagnostic of `src`, with its code.
#[track_caller]
fn only_error(src: &str, code: &str) -> TypeError {
    let errors = check(src);
    assert_eq!(errors.len(), 1, "{:?}", errors);
    assert_eq!(errors[0].code(), code, "{:?}", errors[0]);
    errors[0].clone()
}

#[test]
fn receiver_function_types_in_diagnostics() {
    let error = only_error(
        "struct ColumnScope : Copy {}\nfun Column(content: ColumnScope.(Int) -> Unit) {}\nfun f(): String = Column",
        "E4005",
    );
    assert!(
        error
            .message()
            .contains("(ColumnScope.(Int) -> Unit) -> Unit"),
        "{}",
        error.message()
    );
}

#[test]
fn this_outside_an_extension_names_lambdas_with_a_receiver() {
    let error = only_error("fun f() { val t = this }", "E4001");
    assert!(
        error.message().contains(
            "only available in an extension function ('fun T.name()') or a lambda with a receiver"
        ),
        "{}",
        error.message()
    );
}

#[test]
fn parameter_counts_exclude_the_receiver() {
    let error = only_error(
        "struct Scope : Copy {}\nfun pair(f: Scope.(Int, Int) -> Int): Int = f(Scope(), 1, 2)\nfun g() { pair { it } }",
        "E4004",
    );
    assert!(
        error
            .message()
            .contains("expected 2 parameters ('Int', 'Int')"),
        "{}",
        error.message()
    );
}

#[test]
fn a_receiver_function_value_called_without_a_receiver() {
    for src in [
        "struct Scope : Copy {}\nfun a(f: Scope.() -> Int): Int = f()",
        "struct Scope : Copy {}\nstruct Other : Copy {}\nfun Other.b(f: Scope.() -> Int): Int = f()",
    ] {
        let error = only_error(src, "E4004");
        assert!(
            error
                .message()
                .contains("'f' takes a receiver of type 'Scope'"),
            "{}",
            error.message()
        );
        let help = error
            .missing_function_value_receiver_help()
            .expect("a help saying how to pass the receiver");
        assert!(help.contains("'f(scope)' or 'scope.f()'"), "{}", help);
    }
}

#[test]
fn an_outer_receivers_field_is_hidden() {
    let error = only_error(
        "struct User { val name: String }\nstruct Scope : Copy {}\nfun column(f: Scope.() -> Unit) { f(Scope()) }\nfun show(s: &String) {}\nfun User.card() { column { show(name) } }",
        "E4001",
    );
    let help = error
        .outer_receiver_help()
        .expect("a help naming the outer receiver");
    assert!(
        help.contains("'name' is a field of the outer receiver 'User'")
            && help.contains("'val user = this'"),
        "{}",
        help
    );
}

#[test]
fn nested_scopes_name_the_outer_receiver() {
    let error = only_error(
        "struct RowScope : Copy {}\nstruct BoxScope : Copy {}\nfun row(f: RowScope.() -> Unit) { f(RowScope()) }\nfun box(f: BoxScope.() -> Unit) { f(BoxScope()) }\nfun RowScope.spacer() {}\nfun g() { row { box { spacer() } } }",
        "E4001",
    );
    let help = error
        .outer_receiver_help()
        .expect("a help naming the outer receiver");
    assert!(
        help.contains("'spacer' is an extension of the outer receiver 'RowScope'"),
        "{}",
        help
    );
}

#[test]
fn an_inner_scope_hides_the_outer_context() {
    let prelude = "struct Modifier : Copy { val fraction: Double }\nstruct RowScope : Copy {}\nstruct BoxScope : Copy {}\nfun row(f: RowScope.() -> Unit) { f(RowScope()) }\nfun box(f: BoxScope.() -> Unit) { f(BoxScope()) }\ncontext(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)\nfun use(m: Modifier) {}\n";
    for body in [
        "fun f() { row { box { use(Modifier(0.0).weight(1.0)) } } }",
        "context(scope: RowScope) fun g() { box { use(Modifier(0.0).weight(1.0)) } }",
    ] {
        let error = only_error(&format!("{prelude}{body}"), "E4034");
        let message = error.message();
        assert!(
            message.contains("'weight' needs a context of type 'RowScope'")
                && message.contains("the innermost receiver here is 'BoxScope'"),
            "{}",
            message
        );
        assert!(error
            .missing_context_help()
            .is_some_and(|help| help.contains("'context(scope: RowScope)'")));
    }
}

#[test]
fn missing_context_names_the_declared_contexts() {
    let error = only_error(
        "struct Modifier : Copy { val fraction: Double }\nstruct RowScope : Copy {}\nstruct ColumnScope : Copy {}\ncontext(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)\ncontext(scope: ColumnScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)\nfun f(): Modifier = Modifier(0.0).weight(1.0)",
        "E4034",
    );
    let message = error.message();
    assert!(
        message.contains("'weight' is declared for the contexts 'RowScope' and 'ColumnScope'")
            && message.contains("this position has no receiver"),
        "{}",
        message
    );
}
