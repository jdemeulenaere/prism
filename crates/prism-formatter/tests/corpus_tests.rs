//! "Formatting Preserves Meaning" and "Formatting Is Idempotent" (`openspec/specs/formatting`)
//! over every `.prism` file in the repository: the `prism check` fixtures, the examples and the
//! formatter's golden files.
//!
//! `format_source` itself checks that its output parses to the input's tree and keeps its
//! comments, so a file that formats without an internal error keeps its meaning; this test adds
//! that formatting the output again changes nothing.

use prism_formatter::{format_source, FormatError};
use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prism_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            prism_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "prism")
        {
            out.push(path);
        }
    }
}

fn corpus() -> Vec<PathBuf> {
    let root = repository_root();
    let mut files = Vec::new();
    for directory in [
        "crates/prism-cli/tests/fixtures",
        "examples",
        "crates/prism-formatter/tests/golden",
    ] {
        prism_files(&root.join(directory), &mut files);
    }
    files
}

/// `source` with a comment inserted before every token: a block comment on the token's line,
/// and also a line comment on a line of its own before each token that starts a line. Neither
/// changes the parse.
fn with_comments_everywhere(source: &str) -> Option<String> {
    let tokens = prism_lexer::Lexer::new(source).tokenize_all();
    if tokens
        .iter()
        .any(|t| matches!(t.kind, prism_lexer::TokenKind::Error(_)))
    {
        return None;
    }
    let mut out = String::new();
    let mut copied = 0;
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == prism_lexer::TokenKind::Eof {
            break;
        }
        out.push_str(&source[copied..token.span.start]);
        if !is_template_text_level(&token.kind) {
            if token.is_preceded_by_newline {
                out.push_str(&format!("// line {}\n", index));
            }
            out.push_str(&format!(" /* block {} */ ", index));
        }
        out.push_str(&source[token.span.start..token.span.end]);
        copied = token.span.end;
    }
    out.push_str(&source[copied..]);
    Some(out)
}

/// Whether a token sits at the text level of a string template, where anything inserted before
/// it would change the template's text (D14 of `string-templates`).
fn is_template_text_level(kind: &prism_lexer::TokenKind) -> bool {
    matches!(
        kind,
        prism_lexer::TokenKind::TemplateText(_)
            | prism_lexer::TokenKind::TemplateName(_)
            | prism_lexer::TokenKind::TemplateExpressionStart
            | prism_lexer::TokenKind::TemplateEnd
    )
}

/// `source` without comments and with its tokens joined by one space, or by one newline where
/// a newline preceded the token, which keeps the parse.
fn squeezed(source: &str) -> Option<String> {
    let tokens = prism_lexer::Lexer::new(source).tokenize_all();
    let mut out = String::new();
    for token in &tokens {
        match token.kind {
            prism_lexer::TokenKind::Eof => break,
            prism_lexer::TokenKind::Error(_) => return None,
            _ => {}
        }
        if !out.is_empty() && !is_template_text_level(&token.kind) {
            out.push(if token.is_preceded_by_newline {
                '\n'
            } else {
                ' '
            });
        }
        out.push_str(&source[token.span.start..token.span.end]);
    }
    Some(out)
}

/// `source` with a line comment at the end of every line that ends after a token, which keeps
/// the parse since the newline after it is kept.
fn with_trailing_comments(source: &str) -> Option<String> {
    let tokens = prism_lexer::Lexer::new(source).tokenize_all();
    let mut out = String::new();
    let mut copied = 0;
    for (index, pair) in tokens.windows(2).enumerate() {
        if matches!(pair[0].kind, prism_lexer::TokenKind::Error(_)) {
            return None;
        }
        if pair[1].is_preceded_by_newline || pair[1].kind == prism_lexer::TokenKind::Eof {
            out.push_str(&source[copied..pair[0].span.end]);
            out.push_str(&format!(" // trailing {}", index));
            copied = pair[0].span.end;
        }
    }
    out.push_str(&source[copied..]);
    Some(out)
}

/// `source` with every identifier made 24 characters longer, which forces the width-aware
/// breaks everywhere.
fn with_long_names(source: &str) -> Option<String> {
    let tokens = prism_lexer::Lexer::new(source).tokenize_all();
    let mut out = String::new();
    let mut copied = 0;
    for token in &tokens {
        match &token.kind {
            prism_lexer::TokenKind::Error(_) => return None,
            prism_lexer::TokenKind::Ident(_) => {
                out.push_str(&source[copied..token.span.end]);
                out.push_str("_abcdefghijklmnopqrstuvw");
                copied = token.span.end;
            }
            prism_lexer::TokenKind::TemplateName(name) if name != "this" => {
                out.push_str(&source[copied..token.span.end]);
                out.push_str("_abcdefghijklmnopqrstuvw");
                copied = token.span.end;
            }
            _ => {}
        }
    }
    out.push_str(&source[copied..]);
    prism_parser::parse_source(&out).ok()?;
    Some(out)
}

#[test]
fn trailing_comments_on_every_line_keep_their_order_and_the_meaning() {
    check_variants(with_trailing_comments);
}

#[test]
fn long_names_break_without_changing_the_meaning() {
    check_variants(with_long_names);
}

#[test]
fn comments_anywhere_keep_their_order_and_the_meaning() {
    check_variants(with_comments_everywhere);
}

#[test]
fn squeezed_layouts_format_like_their_sources() {
    check_variants(squeezed);
}

/// Formats `variant` of every corpus file that parses, expecting no error and idempotency.
fn check_variants(variant: fn(&str) -> Option<String>) {
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in corpus() {
        let source = fs::read_to_string(&path).unwrap();
        if prism_parser::parse_source(&source).is_err() {
            continue;
        }
        let Some(commented) = variant(&source) else {
            continue;
        };
        checked += 1;
        match format_source(&commented) {
            Ok(formatted) => match format_source(&formatted) {
                Ok(again) if again == formatted => {}
                Ok(again) => failures.push(format!(
                    "{}: not idempotent\n--- once:\n{}\n--- twice:\n{}",
                    path.display(),
                    formatted,
                    again
                )),
                Err(err) => failures.push(format!("{}: second pass: {}", path.display(), err)),
            },
            Err(err) => failures.push(format!("{}: {}", path.display(), err)),
        }
    }
    assert!(checked > 500, "too few files checked");
    assert!(
        failures.is_empty(),
        "{} of {} files failed:\n\n{}",
        failures.len(),
        checked,
        failures.join("\n\n")
    );
}

#[test]
fn every_fixture_and_example_keeps_its_meaning_and_formats_idempotently() {
    let files = corpus();
    assert!(files.len() > 1000, "the corpus is missing files");
    let mut failures = Vec::new();
    let mut formatted_count = 0;
    for path in &files {
        let source = fs::read_to_string(path).unwrap();
        match format_source(&source) {
            Ok(formatted) => {
                formatted_count += 1;
                match format_source(&formatted) {
                    Ok(again) if again == formatted => {}
                    Ok(again) => failures.push(format!(
                        "{}: not idempotent\n--- once:\n{}\n--- twice:\n{}",
                        path.display(),
                        formatted,
                        again
                    )),
                    Err(err) => failures.push(format!(
                        "{}: formatting the output failed: {}\n--- output:\n{}",
                        path.display(),
                        err,
                        formatted
                    )),
                }
            }
            Err(FormatError::Syntax(_)) => {
                if prism_parser::parse_source(&source).is_ok() {
                    failures.push(format!(
                        "{}: reported as a syntax error but parses",
                        path.display()
                    ));
                }
            }
            Err(FormatError::Internal(reason)) => {
                failures.push(format!("{}: {}", path.display(), reason))
            }
        }
    }
    assert!(formatted_count > 500, "too few files formatted");
    assert!(
        failures.is_empty(),
        "{} of {} files failed:\n\n{}",
        failures.len(),
        files.len(),
        failures.join("\n\n")
    );
}
