use prism_abi::AbiError;
use prism_ast::Span;
use prism_borrowck::{BorrowError, Label};
use prism_parser::ParseError;
use prism_resolver::ResolveError;
use prism_typechecker::{Type, TypeError};
use std::path::Path;

/// Formats a snippet with a line pointer when source text is provided.
fn render_code_snippet(source: &str, line: u32, col: u32) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let line_usize = line as usize;
    if line == 0 || line_usize > lines.len() {
        return None;
    }
    let src_line = lines[line_usize - 1];
    let col_idx = if col > 0 { (col - 1) as usize } else { 0 };
    let pointer = format!("{}^", " ".repeat(col_idx));
    Some(format!(" {:4} | {}\n      | {}", line, src_line, pointer))
}

/// Formats the snippet of `span`'s first line, underlined from its column to the end of the span
/// or of the line (at least one `^`), followed by `label`.
fn render_labelled_snippet(source: &str, span: Span, label: &str) -> Option<String> {
    let line = source.lines().nth((span.line as usize).checked_sub(1)?)?;
    let column = (span.column.max(1) - 1) as usize;
    let spanned = source
        .get(span.start..span.end.max(span.start))
        .unwrap_or_default();
    let width = spanned
        .chars()
        .take_while(|c| *c != '\n')
        .count()
        .min(line.chars().count().saturating_sub(column))
        .max(1);
    Some(format!(
        " {:4} | {}\n      | {}{} {}",
        span.line,
        line,
        " ".repeat(column),
        "^".repeat(width),
        label
    ))
}

/// Formats a [`ResolveError`] as a human-readable compiler diagnostic.
pub fn format_resolve_error(err: &ResolveError) -> String {
    let mut out = String::new();
    let code = err.code();

    match err {
        ResolveError::PackagePathMismatch {
            file,
            declared_package,
            expected_suffix,
            actual_dir,
        } => {
            out.push_str(&format!("error[{}]: package path mismatch\n", code));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: directory path '{}' does not match declared package '{}' (expected suffix: '{}')\n",
                actual_dir.display(),
                declared_package,
                expected_suffix
            ));
        }
        ResolveError::FileNotFound {
            import_path,
            target_path,
        } => {
            out.push_str(&format!("error[{}]: imported file not found\n", code));
            out.push_str(&format!(
                "   = note: cannot resolve import '{}': target file '{}' does not exist\n",
                import_path,
                target_path.display()
            ));
        }
        ResolveError::PackageHeterogeneity {
            file,
            package_dir,
            declared_package,
            expected_package,
        } => {
            out.push_str(&format!(
                "error[{}]: conflicting package in directory\n",
                code
            ));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: file in directory '{}' declares package '{}' but expected package '{}'\n",
                package_dir.display(),
                declared_package,
                expected_package
            ));
        }
        ResolveError::InconsistentWorkspaceRoot {
            file,
            file_root,
            workspace_root,
        } => {
            out.push_str(&format!("error[{}]: inconsistent workspace root\n", code));
            out.push_str(&format!("  --> {}:1:1\n", file.display()));
            out.push_str(&format!(
                "   = note: dependency root '{}' conflicts with workspace root '{}'\n",
                file_root.display(),
                workspace_root.display()
            ));
        }
        ResolveError::WorkspaceBoundaryViolation {
            import_path,
            target_path,
            workspace_root,
        } => {
            out.push_str(&format!(
                "error[{}]: import escapes workspace boundary\n",
                code
            ));
            out.push_str(&format!(
                "   = note: import '{}' targets '{}' which escapes workspace root '{}'\n",
                import_path,
                target_path.display(),
                workspace_root.display()
            ));
        }
        ResolveError::InvalidImportPath {
            import_path,
            reason,
        } => {
            out.push_str(&format!("error[{}]: invalid import path\n", code));
            out.push_str(&format!(
                "   = note: import path '{}' is invalid: {}\n",
                import_path, reason
            ));
        }
        ResolveError::Io { path, message } => {
            out.push_str(&format!("error[{}]: IO error\n", code));
            out.push_str(&format!("  --> {}: {}\n", path.display(), message));
        }
        ResolveError::Parse { path, error } => {
            let source = std::fs::read_to_string(path).ok();
            out.push_str(&format_parse_error(path, source.as_deref(), error));
        }
    }

    if let Some(suggestion) = err.suggestion() {
        out.push_str(&format!("   = help: {}\n", suggestion));
    }

    out
}

/// Formats a parser error.
pub fn format_parse_error(path: &Path, source: Option<&str>, err: &ParseError) -> String {
    let mut out = String::new();
    out.push_str(&format!("error[{}]: syntax error\n", err.code));
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path.display(),
        err.span.line,
        err.span.column
    ));
    if let Some(src) = source {
        if let Some(snippet) = render_code_snippet(src, err.span.line, err.span.column) {
            out.push_str(&format!("{}\n", snippet));
        }
    }
    out.push_str(&format!("   = note: {}\n", err.message));
    out
}

/// Formats an ABI extraction error.
pub fn format_abi_error(path: &Path, source: Option<&str>, err: &AbiError) -> String {
    let mut out = String::new();
    out.push_str("error[E3001]: ABI specification violation\n");
    let span = match err {
        AbiError::MissingExplicitReturnType { span, .. } => span,
        AbiError::MissingExplicitVarType { span, .. } => span,
    };
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path.display(),
        span.line,
        span.column
    ));
    if let Some(src) = source {
        if let Some(snippet) = render_code_snippet(src, span.line, span.column) {
            out.push_str(&format!("{}\n", snippet));
        }
    }
    out.push_str(&format!("   = note: {}\n", err));
    out.push_str("   = help: all public and package declarations must declare explicit types for ABI extraction\n");
    out
}

/// Formats a list of type checking errors.
pub fn format_type_errors(path: &Path, source: Option<&str>, errors: &[TypeError]) -> String {
    let mut out = String::new();
    for err in errors {
        out.push_str(&format_type_error(path, source, err));
    }
    out
}

/// Formats an individual type checking error with source snippets and carets.
pub fn format_type_error(path: &Path, source: Option<&str>, err: &TypeError) -> String {
    let mut out = String::new();
    let span = err.span();
    out.push_str(&format!("error[{}]: {}\n", err.code(), err.message()));
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path.display(),
        span.line,
        span.column
    ));
    if let Some(src) = source {
        if let Some(snippet) = render_code_snippet(src, span.line, span.column) {
            out.push_str(&format!("{}\n", snippet));
        }
    }

    match err {
        TypeError::ImmutabilityViolation {
            help: Some(help), ..
        }
        | TypeError::TypeMismatch {
            help: Some(help), ..
        }
        | TypeError::OperatorMismatch {
            help: Some(help), ..
        }
        | TypeError::ArgumentTypeMismatch {
            help: Some(help), ..
        }
        | TypeError::ReturnTypeMismatch {
            help: Some(help), ..
        }
        | TypeError::UnknownField {
            help: Some(help), ..
        }
        | TypeError::CannotInferType {
            help: Some(help), ..
        } => {
            out.push_str(&format!("   = help: {}\n", help));
        }
        TypeError::ImmutabilityViolation { .. } => {
            out.push_str("   = help: use 'var' to declare a mutable binding or field\n");
        }
        TypeError::CallSignatureMismatch { .. } | TypeError::ArgumentTypeMismatch { .. } => {
            out.push_str(
                "   = help: verify parameter counts, argument types, or named parameters\n",
            );
        }
        TypeError::VisibilityViolation { .. } => {
            out.push_str("   = help: change declaration visibility to 'public' or access from the defining package\n");
        }
        TypeError::InvalidComposeContext { .. } => {
            out.push_str("   = help: add 'compose' modifier to the caller or invoke within a composable function/closure\n");
        }
        TypeError::ReturnTypeMismatch { .. } => {
            out.push_str("   = help: return a value of the declared return type, or change the declared return type\n");
        }
        TypeError::MissingReturnType { .. } => {
            out.push_str("   = help: add an explicit return type such as ': Unit'; public and package functions must declare return types for ABI extraction\n");
        }
        TypeError::AmbiguousImport { .. } => {
            out.push_str("   = help: import only one of the modules that export this name\n");
        }
        TypeError::MemberOfUnimportedType { module, .. } => {
            out.push_str(&format!(
                "   = help: add 'import /{}' to this file\n",
                module
            ));
        }
        TypeError::CannotInferType { .. } => {
            out.push_str("   = help: add an explicit type annotation, or pass the closure where a function type is expected\n");
        }
        TypeError::NonExhaustiveWhen { .. } => {
            out.push_str(
                "   = help: add an 'else -> ...' branch to make the 'when' expression exhaustive\n",
            );
        }
        TypeError::BreakOutsideLoop { .. } => {
            out.push_str("   = help: move it into a 'while', 'do' or 'for' loop body; inside a closure, return from the closure instead\n");
        }
        TypeError::InvalidMarkerDeclaration {
            marker,
            type_name,
            member_is_clone,
            ..
        } => {
            let marker = marker.as_str();
            out.push_str(&format!(
                "   = help: a '{marker}' type can only contain '{marker}' types; change the member's type or remove ': {marker}'\n"
            ));
            if marker == "Copy" && *member_is_clone {
                out.push_str(&format!(
                    "   = help: declare '{type_name} : Clone' instead to allow explicit '.clone()'\n"
                ));
            }
        }
        TypeError::CannotMoveOut {
            borrow, clone, ty, ..
        } => match borrow {
            Some(borrow) => {
                out.push_str(&format!("   = help: borrow it instead: '{}'\n", borrow));
                if let Some(clone) = clone {
                    out.push_str(&format!("   = help: or clone it: '{}'\n", clone));
                }
            }
            // The source is already a borrow: clone it, or keep borrowing where it is used.
            None => {
                if let Some(clone) = clone {
                    out.push_str(&format!("   = help: clone it: '{}'\n", clone));
                }
                out.push_str(&format!(
                    "   = help: {}take a '&{}' where the value is used, instead of a '{}'\n",
                    if clone.is_some() { "or " } else { "" },
                    ty,
                    ty
                ));
            }
        },
        TypeError::WrongTypeArgumentCount { expected: 0, .. } => {
            out.push_str(
                "   = help: remove the type arguments; only generic declarations take them\n",
            );
        }
        TypeError::WrongTypeArgumentCount { expected, .. } => {
            out.push_str(&format!(
                "   = help: write exactly {} type argument{}, one per type parameter, in order\n",
                expected,
                if *expected == 1 { "" } else { "s" }
            ));
        }
        TypeError::UnsatisfiedBound {
            parameter, bound, ..
        } => {
            out.push_str(&format!(
                "   = help: add the bound to the caller's type parameter, or pass a '{}' value ('{}' is declared '{} : {}')\n",
                bound.as_str(),
                parameter,
                parameter,
                bound.as_str()
            ));
        }
        TypeError::BorrowInTypeArgument { .. } => {
            out.push_str("   = help: type parameters stand for owned values; take '&T' in the signature instead, as in 'fun <T> first(a: &T): &T'\n");
        }
        TypeError::NotAReference { .. } => {
            out.push_str("   = help: '*r = v' writes through a reference 'r'; assign a variable or field directly with 'x = v'\n");
        }
        TypeError::NotCloneable { ty, .. } => {
            let help = match ty {
                Type::Struct { id, .. } | Type::Enum { id, .. } => {
                    format!("add ': Clone' to the declaration of '{}'", id.name)
                }
                Type::Fn { .. } => {
                    "function values cannot be cloned, because a closure holds borrows of what it captures".to_string()
                }
                Type::Parameter(parameter) => {
                    format!("add the bound '<{} : Clone>'", parameter.name)
                }
                _ => "only 'Copy' and 'Clone' types, 'String', and nullables and 'Heap' of them can be cloned".to_string(),
            };
            out.push_str(&format!("   = help: {}\n", help));
        }
        _ => {}
    }

    out
}

/// Formats the borrow checker's errors in one file.
pub fn format_borrow_errors(path: &Path, source: Option<&str>, errors: &[BorrowError]) -> String {
    errors
        .iter()
        .map(|error| format_borrow_error(path, source, error))
        .collect()
}

/// Formats one borrow checker error: its header, its primary location with its label, each
/// related location with its own, then its notes and help lines (the `diagnostics` spec,
/// "Diagnostics Show Related Locations").
pub fn format_borrow_error(path: &Path, source: Option<&str>, error: &BorrowError) -> String {
    let mut out = format!("error[{}]: {}\n", error.code(), error.message());
    let mut location = |label: &Label| {
        out.push_str(&format!(
            "  --> {}:{}:{}\n",
            path.display(),
            label.span.line,
            label.span.column
        ));
        if let Some(snippet) =
            source.and_then(|src| render_labelled_snippet(src, label.span, &label.message))
        {
            out.push_str(&format!("{}\n", snippet));
        }
    };
    location(&error.primary);
    for label in &error.related {
        location(label);
    }
    for note in &error.notes {
        out.push_str(&format!("   = note: {}\n", note));
    }
    for help in &error.helps {
        out.push_str(&format!("   = help: {}\n", help));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_format_resolve_error_package_mismatch() {
        let err = ResolveError::PackagePathMismatch {
            file: PathBuf::from("src/views.prism"),
            declared_package: "app.ui".to_string(),
            expected_suffix: "app/ui".to_string(),
            actual_dir: PathBuf::from("src"),
        };
        let formatted = format_resolve_error(&err);
        assert!(formatted.contains("error[E1001]: package path mismatch"));
        assert!(formatted.contains("--> src/views.prism:1:1"));
        assert!(formatted
            .contains("help: Rename directory to match package or update package statement."));
    }

    #[test]
    fn test_format_resolve_error_file_not_found() {
        let err = ResolveError::FileNotFound {
            import_path: "/core/math".to_string(),
            target_path: PathBuf::from("/workspace/core/math.prism"),
        };
        let formatted = format_resolve_error(&err);
        assert!(formatted.contains("error[E1002]: imported file not found"));
        assert!(formatted.contains("cannot resolve import '/core/math'"));
    }

    #[test]
    fn test_format_parse_error() {
        let err = ParseError::new("expected identifier", prism_ast::Span::new(10, 15, 2, 5));
        let src = "fun main() {\n    val = 1\n}";
        let formatted = format_parse_error(Path::new("test.prism"), Some(src), &err);
        assert!(formatted.contains("error[E2001]: syntax error"));
        assert!(formatted.contains("test.prism:2:5"));
        assert!(formatted.contains("val = 1"));
    }

    #[test]
    fn test_format_type_error() {
        let err = TypeError::TypeMismatch {
            expected: prism_typechecker::Type::Int,
            found: prism_typechecker::Type::String,
            span: prism_ast::Span::new(15, 21, 2, 11),
            help: None,
        };
        let src = "fun main() {\n    val x: Int = \"hello\"\n}";
        let formatted = format_type_error(Path::new("main.prism"), Some(src), &err);
        assert!(formatted.contains("error[E4002]: type mismatch: expected 'Int', found 'String'"));
        assert!(formatted.contains("main.prism:2:11"));
        assert!(formatted.contains("val x: Int = \"hello\""));
    }

    #[test]
    fn compound_operator_mismatch_names_the_operator_and_its_operand_types() {
        let rendered = render_type_errors("fun f() {\n    var s = \"a\"\n    s -= \"b\"\n}\n");
        assert!(
            rendered
                .contains("error[E4002]: operator '-=' cannot be applied to 'String' and 'String'"),
            "{}",
            rendered
        );
        assert!(
            rendered.contains(
                "= help: '-=' needs a target and a value that are both 'Int' or both 'Float'"
            ),
            "{}",
            rendered
        );
    }

    #[test]
    fn compound_assignment_to_a_reference_binding_suggests_writing_through_it() {
        let rendered = render_type_errors("fun f(r: &mut Int) {\n    r += 1\n}\n");
        assert!(rendered.contains("error[E4003]"), "{}", rendered);
        assert!(
            rendered.contains("= help: to write through the reference, use '*r += 1'"),
            "{}",
            rendered
        );
    }

    /// Renders the type errors of `src`, checked as a single file.
    fn render_type_errors(src: &str) -> String {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = prism_typechecker::TypeChecker::new(None, "main");
        checker.check_module(&file);
        checker
            .errors
            .iter()
            .map(|err| format_type_error(Path::new("main.prism"), Some(src), err))
            .collect()
    }

    #[test]
    fn test_generic_errors_render_their_helps() {
        let out = render_type_errors(
            "fun <T> id(x: T): T = x\nfun <T : Copy> dup(x: T): T = x\nfun f(a: Int, s: String) {\n    val r = id<&Int>(&a)\n    val d = dup(s)\n    val n = id<Int, Int>(1)\n}\nfun <T> none(): T? = null\nfun g() {\n    val o = none()\n}\n",
        );
        assert!(
            out.contains("error[E4022]: type argument '&Int' of 'id'"),
            "{out}"
        );
        assert!(out.contains("take '&T' in the signature instead"), "{out}");
        assert!(
            out.contains("error[E4021]: type argument 'String' of 'dup' does not satisfy the bound 'T : Copy'"),
            "{out}"
        );
        assert!(
            out.contains("add the bound to the caller's type parameter"),
            "{out}"
        );
        assert!(
            out.contains("error[E4020]: 'id' takes 1 type argument, but 2 were given"),
            "{out}"
        );
        assert!(
            out.contains("error[E4013]: cannot infer the type argument 'T' of 'none'"),
            "{out}"
        );
        assert!(
            out.contains("help: write the type arguments, as in 'name<...>(...)'"),
            "{out}"
        );
    }

    #[test]
    fn test_clone_of_an_unbounded_type_parameter_suggests_a_bound() {
        let out = render_type_errors("fun <T> c(x: &T): T = x.clone()\n");
        assert!(out.contains("error[E4018]"), "{out}");
        assert!(out.contains("help: add the bound '<T : Clone>'"), "{out}");
    }

    #[test]
    fn test_cannot_move_out_help_suggests_borrow_and_clone() {
        let out = render_type_errors(
            "struct User { val name: String, val nick: String? }\nfun f(u: User) {\n    val n = u.name\n    val k = u.nick\n}\n",
        );
        assert!(
            out.contains("error[E4017]: cannot move out of field 'name' of 'u'"),
            "{out}"
        );
        assert!(out.contains("help: borrow it instead: '&u.name'"), "{out}");
        assert!(out.contains("help: or clone it: 'u.name.clone()'"), "{out}");
        assert!(
            out.contains("help: or clone it: 'u.nick?.clone()'"),
            "{out}"
        );
    }

    #[test]
    fn test_cannot_move_out_help_for_a_non_clone_field() {
        let out = render_type_errors(
            "struct Token { val id: Int }\nstruct Session { val token: Token }\nfun f(s: Session): Token = s.token\n",
        );
        assert!(out.contains("help: borrow it instead: '&s.token'"), "{out}");
        assert!(!out.contains("clone"), "{out}");
    }

    #[test]
    fn test_cannot_move_out_of_a_borrow_help() {
        let out = render_type_errors(
            "struct User : Clone { val name: String }\nfun take(u: User) {}\nfun f(u: &User) {\n    take(u)\n}\n",
        );
        assert!(
            out.contains("error[E4017]: cannot move out of borrow 'u'"),
            "{out}"
        );
        assert!(out.contains("help: clone it: 'u.clone()'"), "{out}");
        assert!(
            out.contains("help: or take a '&User' where the value is used"),
            "{out}"
        );
        assert!(!out.contains("borrow it instead"), "{out}");
    }

    #[test]
    fn test_cannot_move_out_of_a_borrow_help_without_clone() {
        let out = render_type_errors("struct Token { val id: Int }\nfun f(t: &Token): Token = t\n");
        assert!(
            out.contains("error[E4017]: cannot move out of borrow 't'"),
            "{out}"
        );
        assert!(
            out.contains("help: take a '&Token' where the value is used"),
            "{out}"
        );
        assert!(!out.contains("clone"), "{out}");
    }

    /// ownership: "Moving an earlier parameter in a default value".
    #[test]
    fn test_moving_an_earlier_parameter_in_a_default_value_help() {
        let out = render_type_errors(
            "struct User : Clone { val name: String }\nfun f(a: String, b: String = a): Int = 1\nconsuming fun User.g(u: User = this): Int = 1\n",
        );
        assert!(
            out.contains(
                "error[E4017]: cannot move out of parameter 'a', which a default value borrows"
            ),
            "{out}"
        );
        assert!(out.contains("help: clone it: 'a.clone()'"), "{out}");
        assert!(
            out.contains("help: or take a '&String' where the value is used"),
            "{out}"
        );
        assert!(
            out.contains("error[E4017]: cannot move out of 'this', which a default value borrows"),
            "{out}"
        );
        assert!(out.contains("help: clone it: 'this.clone()'"), "{out}");
        assert!(!out.contains("borrow it instead"), "{out}");
    }

    #[test]
    fn test_moving_a_captured_variable_inside_a_closure_help() {
        let out = render_type_errors(
            "fun take(s: String) {}\nfun run(f: () -> Int): Int = f()\nfun f(s: String): Int = run { take(s); 1 }\n",
        );
        assert!(
            out.contains(
                "error[E4017]: cannot move out of captured variable 's', which the closure borrows"
            ),
            "{out}"
        );
        assert!(out.contains("help: clone it: 's.clone()'"), "{out}");
        assert!(!out.contains("borrow it instead"), "{out}");
    }

    #[test]
    fn test_borrow_help_lines() {
        let out = render_type_errors(
            "struct User { val name: String }\nfun bump(r: &mut Int) {}\nfun f(name: String, u: &User): &String {\n    var x = 1\n    bump(x)\n    val r: &String = name\n    return u.name\n}\n",
        );
        assert!(
            out.contains("help: pass a mutable borrow: '&mut x'"),
            "{out}"
        );
        assert!(out.contains("help: borrow it: '&name'"), "{out}");
        assert!(out.contains("help: borrow it: '&u.name'"), "{out}");
        assert!(!out.contains("E4017"), "{out}");
    }

    #[test]
    fn test_mutable_borrow_of_a_val_reference_binding_help() {
        let out = render_type_errors("fun f(r: &mut Int) {\n    val m = &mut r\n}\n");
        assert!(out.contains("error[E4003]"), "{out}");
        assert!(
            out.contains("help: 'r' is already a mutable borrow: pass 'r' itself"),
            "{out}"
        );
        assert!(!out.contains("use 'var'"), "{out}");
    }

    #[test]
    fn test_write_through_help_lines() {
        let out = render_type_errors(
            "fun f(r: &mut Int, s: &Int) {\n    r = 5\n    var x = 1\n    var m: &mut Int = &mut x\n    m = 6\n    *x = 2\n    *s = 3\n}\n",
        );
        assert!(
            out.contains(
                "help: to write through the reference instead of re-pointing it, use '*r = 5'"
            ),
            "{out}"
        );
        assert!(out.contains("use '*m = 6'"), "{out}");
        assert!(
            out.contains("error[E4019]: cannot write through a value of type 'Int'"),
            "{out}"
        );
        assert!(
            out.contains("error[E4003]: cannot write through 's'"),
            "{out}"
        );
        // Each statement reports one error.
        assert_eq!(out.matches("error[").count(), 4, "{out}");
    }

    /// Renders the borrow errors of `src`, checked as a single file `main.prism`.
    fn render_borrow_errors(src: &str) -> String {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = prism_typechecker::TypeChecker::new(None, "main");
        let tree = checker.check_module(&file);
        assert!(checker.errors.is_empty(), "{:?}", checker.errors);
        let mir = prism_mir::lower_file(&tree);
        let errors = prism_borrowck::check_file(&mir).expect_err("expected borrow errors");
        format_borrow_errors(Path::new("main.prism"), Some(src), &errors)
    }

    /// ownership: "Use after move points at the move"; diagnostics: "Rendered related location".
    #[test]
    fn test_use_after_move_points_at_the_move() {
        let out =
            render_borrow_errors("fun take(s: String) {}\nfun f(s: String) { take(s); take(s) }\n");
        let expected = "\
error[E5001]: use of moved value 's'
  --> main.prism:2:34
    2 | fun f(s: String) { take(s); take(s) }
      |                                  ^ value used here after move
  --> main.prism:2:25
    2 | fun f(s: String) { take(s); take(s) }
      |                         ^ value moved here
   = note: 's' has type 'String', which is not 'Copy'
   = help: clone it where it moves: 's.clone()'
";
        assert_eq!(out, expected);
    }

    /// references: "Conflict points at the borrow and its later use".
    #[test]
    fn test_conflict_points_at_the_borrow_and_its_later_use() {
        let out = render_borrow_errors(
            "fun print(n: &Int) {}\nfun f() { var x = 1; val r = &x; x = 2; print(r) }\n",
        );
        let expected = "\
error[E5004]: cannot assign to 'x' because it is borrowed
  --> main.prism:2:34
    2 | fun f() { var x = 1; val r = &x; x = 2; print(r) }
      |                                  ^ 'x' is assigned here while borrowed
  --> main.prism:2:30
    2 | fun f() { var x = 1; val r = &x; x = 2; print(r) }
      |                              ^^ 'x' is borrowed here
  --> main.prism:2:47
    2 | fun f() { var x = 1; val r = &x; x = 2; print(r) }
      |                                               ^ borrow later used here
";
        assert_eq!(out, expected);
    }

    /// references: "Closure borrow labels".
    #[test]
    fn test_closure_borrow_labels() {
        let out = render_borrow_errors(
            "struct User { var name: String }\nfun greet(s: &String): Int = 1\nfun run(f: () -> Int): Int = f()\nfun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }\n",
        );
        let expected = "\
error[E5003]: cannot borrow 'u.name' as shared because 'u' is borrowed as mutable
  --> main.prism:4:65
    4 | fun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }
      |                                                                 ^^^^^^ shared borrow occurs here
  --> main.prism:4:38
    4 | fun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }
      |                                      ^^^^^^^^^^^^^^^^^^^ closure borrows 'u' mutably here
  --> main.prism:4:40
    4 | fun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }
      |                                        ^^^^^^ borrow occurs due to this use of 'u' in the closure
  --> main.prism:4:78
    4 | fun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }
      |                                                                              ^ mutable borrow later used here
";
        assert_eq!(out, expected);
    }

    /// references: "Dropped value labels".
    #[test]
    fn test_dropped_value_labels() {
        let out = render_borrow_errors(
            "fun print(n: &Int) {}\nfun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }\n",
        );
        let expected = "\
error[E5006]: 'n' does not live long enough
  --> main.prism:2:71
    2 | fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }
      |                                                                       ^^ borrowed value does not live long enough
  --> main.prism:2:74
    2 | fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }
      |                                                                          ^ 'n' dropped here while still borrowed
  --> main.prism:2:83
    2 | fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }
      |                                                                                   ^ borrow later used here
";
        assert_eq!(out, expected);
    }

    /// references: "Dropped parameter labels".
    #[test]
    fn test_dropped_parameter_labels() {
        let out = render_borrow_errors("fun keep(slot: &mut &String, v: String) { *slot = &v }\n");
        let expected = "\
error[E5006]: 'v' does not live long enough
  --> main.prism:1:51
    1 | fun keep(slot: &mut &String, v: String) { *slot = &v }
      |                                                   ^^ borrowed value does not live long enough
  --> main.prism:1:10
    1 | fun keep(slot: &mut &String, v: String) { *slot = &v }
      |          ^^^^^^^^^^^^^^^^^^ borrow later used by the caller through 'slot'
  --> main.prism:1:30
    1 | fun keep(slot: &mut &String, v: String) { *slot = &v }
      |                              ^^^^^^^^^ 'v' is dropped when the function returns, while still borrowed
";
        assert_eq!(out, expected);
    }

    /// references: "Dangling returned borrow labels".
    #[test]
    fn test_dangling_returned_borrow_labels() {
        let out = render_borrow_errors("fun f(): &Int { val n = 1; val r = &n; return r }\n");
        let expected = "\
error[E5008]: cannot return a borrow of local 'n'
  --> main.prism:1:47
    1 | fun f(): &Int { val n = 1; val r = &n; return r }
      |                                               ^ returned here
  --> main.prism:1:36
    1 | fun f(): &Int { val n = 1; val r = &n; return r }
      |                                    ^^ 'n' is borrowed here
   = note: 'f' takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)
   = help: return the value itself instead of a borrow of it
";
        assert_eq!(out, expected);
    }

    /// references: "Untied returned borrow labels".
    #[test]
    fn test_untied_returned_borrow_labels() {
        let out = render_borrow_errors(
            "struct Team { var lead: String }\nfun Team.other(o: &String): &String = o\n",
        );
        let expected = "\
error[E5009]: method 'other' cannot return a borrow derived from parameter 'o'
  --> main.prism:2:39
    2 | fun Team.other(o: &String): &String = o
      |                                       ^ returned here
  --> main.prism:2:16
    2 | fun Team.other(o: &String): &String = o
      |                ^^^^^^^^^^ 'o' is a parameter here
   = note: a method's returned borrow is tied to its receiver only (elision rule 2), so callers keep only the receiver borrowed
   = help: return a borrow derived from the receiver, or declare a function taking the receiver as a reference parameter, whose result is tied to every reference parameter (elision rule 3)
";
        assert_eq!(out, expected);
    }

    /// references: "Escaping closure labels".
    #[test]
    fn test_escaping_closure_labels() {
        let out = render_borrow_errors(
            "fun greet(s: &String): Int = 1\nfun f(s: String): () -> Int = { greet(s) }\n",
        );
        let expected = "\
error[E5007]: a closure that captures 's' cannot escape
  --> main.prism:2:31
    2 | fun f(s: String): () -> Int = { greet(s) }
      |                               ^^^^^^^^^^^^ returned here
  --> main.prism:2:39
    2 | fun f(s: String): () -> Int = { greet(s) }
      |                                       ^ 's' is captured here
   = note: a closure borrows what it captures, so it can only be called or passed to functions while that is in scope; escaping closures that capture by move are not designed yet
   = help: pass 's' to the closure as a parameter instead of capturing it
";
        assert_eq!(out, expected);
    }

    /// ownership: "Possibly moved value".
    #[test]
    fn test_possibly_moved_value() {
        let out = render_borrow_errors(
            "fun take(s: String) {}\nfun f(c: Bool, s: String) { if (c) take(s); take(s) }\n",
        );
        let expected = "\
error[E5001]: use of possibly moved value 's'
  --> main.prism:2:50
    2 | fun f(c: Bool, s: String) { if (c) take(s); take(s) }
      |                                                  ^ value used here after move
  --> main.prism:2:41
    2 | fun f(c: Bool, s: String) { if (c) take(s); take(s) }
      |                                         ^ value moved here
   = note: 's' has type 'String', which is not 'Copy'
   = help: clone it where it moves: 's.clone()'
";
        assert_eq!(out, expected);
    }

    /// ownership: "Continue skips a declaration that the condition reads".
    #[test]
    fn test_uninitialized_read_points_at_the_skipping_jump() {
        let out = render_borrow_errors(
            "fun next(): Int = 0\nfun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }\n",
        );
        let expected = "\
error[E5002]: use of possibly uninitialized value 'x'
  --> main.prism:2:77
    2 | fun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }
      |                                                                             ^ 'x' used here, but it is not initialized on every path
  --> main.prism:2:43
    2 | fun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }
      |                                           ^^^^^^^^ the declaration of 'x' is skipped here
   = help: declare 'x' before the jump that skips it
";
        assert_eq!(out, expected);
    }
}
