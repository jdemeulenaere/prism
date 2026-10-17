//! Scenario coverage for `openspec/specs/syntax` (lexical requirements): token-level assertions.
//! The accept/reject forms of the same scenarios are fixtures in `prism-cli/tests/fixtures/syntax`.

use prism_lexer::{Lexer, Token, TokenKind};

fn tokens(src: &str) -> Vec<Token> {
    Lexer::new(src).tokenize_all()
}

fn kinds(src: &str) -> Vec<TokenKind> {
    tokens(src).into_iter().map(|t| t.kind).collect()
}

fn is_error(kind: &TokenKind) -> bool {
    matches!(kind, TokenKind::Error(_))
}

#[test]
fn escape_and_braced_unicode_sequence_decode_inside_one_string() {
    assert_eq!(
        kinds(r#""Hello\n\u{1F680}""#),
        vec![TokenKind::String("Hello\n🚀".to_string()), TokenKind::Eof]
    );
}

#[test]
fn every_simple_escape_decodes() {
    assert_eq!(
        kinds(r#""a\tb\rc\\d\"e\'f\0g""#),
        vec![
            TokenKind::String("a\tb\rc\\d\"e'f\0g".to_string()),
            TokenKind::Eof
        ]
    );
}

#[test]
fn braced_and_fixed_width_unicode_escapes_agree() {
    assert_eq!(kinds(r#""\u{41}""#)[0], kinds(r#""A""#)[0]);
    assert_eq!(kinds(r#""\u{41}""#)[0], TokenKind::String("A".to_string()));
}

#[test]
fn malformed_unicode_escapes_are_error_tokens() {
    for src in [
        r#""\u12""#,
        r#""\u{1F680""#,
        r#""\u{}""#,
        r#""\u{D800}""#,
        r#""\u{110000}""#,
        r#""\uZZZZ""#,
    ] {
        assert!(
            kinds(src).iter().any(is_error),
            "{} must produce an error token, got {:?}",
            src,
            kinds(src)
        );
    }
}

#[test]
fn raw_newline_error_covers_only_the_first_line() {
    let src = "val s = \"hello\nval next = 42";
    let toks = tokens(src);
    let error = toks
        .iter()
        .find(|t| is_error(&t.kind))
        .expect("error token");
    assert_eq!(error.span.line, 1);
    assert!(
        error.span.end <= src.find('\n').unwrap() + 1,
        "the error must not consume the second line: {:?}",
        error.span
    );
    // The next line still lexes normally.
    assert!(toks
        .iter()
        .any(|t| t.kind == TokenKind::Ident("next".to_string())));
}

#[test]
fn unterminated_string_at_end_of_file_is_an_error_token_without_panic() {
    for src in ["\"never closed", "\"", "val s = \"ab\\"] {
        let toks = tokens(src);
        assert!(
            toks.iter().any(|t| is_error(&t.kind)),
            "{:?} must produce an error token",
            src
        );
        assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
    }
}

#[test]
fn unknown_escapes_and_empty_char_literal_are_error_tokens() {
    for src in [r#""\q""#, r#"'\é'"#, r#"'\q'"#, "''"] {
        assert!(
            kinds(src).iter().any(is_error),
            "{} must produce an error token",
            src
        );
    }
}

#[test]
fn numeric_literal_values() {
    assert_eq!(
        kinds("0xFF_FF 0b1010_0001 1_000 3.14e-2 1e10 4..10"),
        vec![
            TokenKind::Int(65535),
            TokenKind::Int(161),
            TokenKind::Int(1000),
            TokenKind::Float(0.0314),
            TokenKind::Float(1e10),
            TokenKind::Int(4),
            TokenKind::DotDot,
            TokenKind::Int(10),
            TokenKind::Eof,
        ]
    );
}

#[test]
fn out_of_range_numeric_literals_are_error_tokens() {
    for src in ["1e999", "18446744073709551616", "0xFFFFFFFFFFFFFFFFF"] {
        assert!(
            kinds(src).iter().any(is_error),
            "{} must produce an error token",
            src
        );
    }
    assert_eq!(
        kinds("9223372036854775807")[0],
        TokenKind::Int(i64::MAX as u64)
    );
}

// --- Half-Open Range Operator (`statements-and-loops`)

#[test]
fn half_open_range_tokens() {
    assert_eq!(
        kinds("4..<10 4..10 4.10 a..<b"),
        vec![
            TokenKind::Int(4),
            TokenKind::DotDotLess,
            TokenKind::Int(10),
            TokenKind::Int(4),
            TokenKind::DotDot,
            TokenKind::Int(10),
            TokenKind::Float(4.10),
            TokenKind::Ident("a".to_string()),
            TokenKind::DotDotLess,
            TokenKind::Ident("b".to_string()),
            TokenKind::Eof,
        ]
    );
}

// --- Reserved Loop Keywords (`statements-and-loops`)

#[test]
fn loop_keywords_are_reserved() {
    assert_eq!(
        kinds("while do for break continue"),
        vec![
            TokenKind::While,
            TokenKind::Do,
            TokenKind::For,
            TokenKind::Break,
            TokenKind::Continue,
            TokenKind::Eof,
        ]
    );
}
