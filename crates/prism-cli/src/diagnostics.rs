use prism_abi::AbiError;
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
        TypeError::CannotMoveOut { borrow, clone, .. } => {
            out.push_str(&format!("   = help: borrow it instead: '{}'\n", borrow));
            if let Some(clone) = clone {
                out.push_str(&format!("   = help: or clone it: '{}'\n", clone));
            }
        }
        TypeError::NotCloneable { ty, .. } => {
            let help = match ty {
                Type::Struct(id) | Type::Enum(id) => {
                    format!("add ': Clone' to the declaration of '{}'", id.name)
                }
                Type::Fn { .. } => {
                    "function values cannot be cloned, because a closure holds borrows of what it captures".to_string()
                }
                Type::TypeParam(_) => {
                    "type parameters cannot be cloned until 'Clone' bounds exist".to_string()
                }
                _ => "only 'Copy' and 'Clone' types, 'String', and nullables and 'Heap' of them can be cloned".to_string(),
            };
            out.push_str(&format!("   = help: {}\n", help));
        }
        _ => {}
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
        };
        let src = "fun main() {\n    val x: Int = \"hello\"\n}";
        let formatted = format_type_error(Path::new("main.prism"), Some(src), &err);
        assert!(formatted.contains("error[E4002]: type mismatch: expected 'Int', found 'String'"));
        assert!(formatted.contains("main.prism:2:11"));
        assert!(formatted.contains("val x: Int = \"hello\""));
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
}
