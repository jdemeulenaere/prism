//! The program's entry point: `fun main()` in the entry file (D13 of `codegen-core`).
//!
//! The entry file must declare a top-level function `main` without type parameters, receiver
//! or parameters, returning `Unit` and not `compose`. None is `E6002`; one breaking a rule, or a
//! top-level `val main`, is `E6003`. A `main` of any other file is an ordinary function.

use crate::error::{CodegenError, CodegenErrorKind};
use prism_ast::Span;
use prism_mir::{Body, BodyOwner, MirFile};
use prism_typed_tree::{DeclarationPath, Type};

/// The name of the entry point.
pub const MAIN: &str = "main";

/// The declaration path of the entry file's `main`, checked against the rules of the entry
/// point.
pub fn find_main(file: &MirFile) -> Result<DeclarationPath, CodegenError> {
    let is_main = |path: &DeclarationPath| path.module == file.module && path.name == MAIN;
    let mut extension = None;
    for (_, body) in file.bodies.iter() {
        match &body.owner {
            BodyOwner::Function(path) if is_main(path) && path.receiver.is_none() => {
                return check_main(body).map(|()| path.clone());
            }
            BodyOwner::Function(path) if is_main(path) => extension = extension.or(Some(body)),
            BodyOwner::Value(path) if is_main(path) => {
                return Err(invalid(body, "it is a top-level value, not a function"));
            }
            _ => {}
        }
    }
    if let Some(body) = extension {
        return Err(invalid(
            body,
            "it is an extension; it must not have a receiver",
        ));
    }
    Err(CodegenError {
        kind: CodegenErrorKind::MissingMain,
        message: format!(
            "the entry file of 'prism run' must declare a function '{}'",
            MAIN
        ),
        span: Span::dummy(),
        notes: Vec::new(),
        help: Some("add 'fun main() {}' to the entry file".to_string()),
    })
}

/// Checks the rules a `main` function must follow.
fn check_main(body: &Body) -> Result<(), CodegenError> {
    if !body.type_parameters.is_empty() {
        return Err(invalid(body, "it has type parameters"));
    }
    if body.is_compose {
        return Err(invalid(body, "it is 'compose'"));
    }
    if body.argument_count > 0 {
        return Err(invalid(body, "it has parameters"));
    }
    if *body.return_type() != Type::Unit {
        return Err(invalid(
            body,
            &format!("it returns '{}'; it must return 'Unit'", body.return_type()),
        ));
    }
    Ok(())
}

fn invalid(body: &Body, rule: &str) -> CodegenError {
    CodegenError {
        kind: CodegenErrorKind::InvalidMain,
        message: format!("'{}' cannot be the entry point: {}", MAIN, rule),
        span: body.span,
        notes: Vec::new(),
        help: Some("declare it as 'fun main() { ... }'".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{lower_files, lower_source};

    fn main_error(source: &str) -> CodegenError {
        find_main(&lower_source(source)).expect_err("main is invalid")
    }

    #[test]
    fn a_valid_main() {
        let path = find_main(&lower_source("fun helper() {}\nfun main() { helper() }\n")).unwrap();
        assert_eq!(path, DeclarationPath::new("main", "main"));
        assert!(find_main(&lower_source("fun main(): Unit {}\n")).is_ok());
    }

    #[test]
    fn missing_main() {
        let error = main_error("fun helper() {}\n");
        assert_eq!(error.code(), "E6002");
        assert_eq!((error.span.line, error.span.column), (1, 1));
        assert!(error.help.is_some());
    }

    #[test]
    fn invalid_main_in_every_form() {
        let cases = [
            ("fun main(n: Int) {}\n", "it has parameters"),
            ("fun main(): Int = 0\n", "it returns 'Int'"),
            ("fun <T> main() {}\n", "it has type parameters"),
            ("compose fun main() {}\n", "it is 'compose'"),
            ("val main: Int = 1\n", "it is a top-level value"),
            ("fun Int.main() {}\n", "it is an extension"),
        ];
        for (source, rule) in cases {
            let error = main_error(source);
            assert_eq!(error.code(), "E6003", "{}", source);
            assert!(
                error.message.contains(rule),
                "{}: {}",
                source,
                error.message
            );
            assert_eq!(error.span.line, 1, "{}", source);
        }
    }

    #[test]
    fn a_main_of_an_imported_file_is_ordinary() {
        let files = lower_files(&[
            (
                "lib/tool",
                "package lib\npublic fun main(n: Int): Int = n\n",
            ),
            (
                "app/main",
                "package app\nimport /lib/tool\nfun helper() {}\n",
            ),
        ]);
        assert_eq!(find_main(&files[1]).unwrap_err().code(), "E6002");
    }
}
