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

// --- Compound Assignment (`compound-assignment`)

fn ident(name: &str) -> TokenKind {
    TokenKind::Ident(name.to_string())
}

#[test]
fn compound_operators_are_single_tokens() {
    let joined = [
        ("a+=b", TokenKind::PlusEqual),
        ("a-=b", TokenKind::MinusEqual),
        ("a*=b", TokenKind::StarEqual),
        ("a/=b", TokenKind::SlashEqual),
        ("a%=b", TokenKind::PercentEqual),
    ];
    for (source, operator) in joined {
        assert_eq!(
            kinds(source),
            vec![ident("a"), operator, ident("b"), TokenKind::Eof],
            "{}",
            source
        );
    }
    assert_eq!(
        kinds("a = -b"),
        vec![
            ident("a"),
            TokenKind::Eq,
            TokenKind::Minus,
            ident("b"),
            TokenKind::Eof
        ]
    );
    assert_eq!(
        kinds("a->b"),
        vec![ident("a"), TokenKind::Arrow, ident("b"), TokenKind::Eof]
    );
    assert_eq!(
        kinds("a == b"),
        vec![ident("a"), TokenKind::EqEq, ident("b"), TokenKind::Eof]
    );
    assert_eq!(
        kinds("a /* c */ = b"),
        vec![ident("a"), TokenKind::Eq, ident("b"), TokenKind::Eof]
    );
}

// --- No Increment or Decrement Operators (`compound-assignment`)

#[test]
fn increment_and_decrement_are_single_tokens() {
    assert_eq!(
        kinds("x++ --y"),
        vec![
            ident("x"),
            TokenKind::PlusPlus,
            TokenKind::MinusMinus,
            ident("y"),
            TokenKind::Eof
        ]
    );
    assert_eq!(
        kinds("a - -b"),
        vec![
            ident("a"),
            TokenKind::Minus,
            TokenKind::Minus,
            ident("b"),
            TokenKind::Eof
        ]
    );
}

// --- Comments Are Available to Tools (`prism-fmt`)

#[test]
fn comment_collecting_lexer_yields_the_same_tokens() {
    use prism_lexer::CommentKind;
    let source = "a /* b /* c */ d */ + // e\nf";
    let (with_comments, comments) = Lexer::new(source).tokenize_with_comments();
    assert_eq!(with_comments, tokens(source));
    assert_eq!(
        with_comments.iter().map(|t| &t.kind).collect::<Vec<_>>(),
        vec![&ident("a"), &TokenKind::Plus, &ident("f"), &TokenKind::Eof]
    );
    assert!(with_comments[2].is_preceded_by_newline);
    let found: Vec<(CommentKind, &str, u32, u32)> = comments
        .iter()
        .map(|c| {
            let text = &source[c.span.start..c.span.end];
            (c.kind, text, c.span.line, c.span.column)
        })
        .collect();
    assert_eq!(
        found,
        vec![
            (CommentKind::Block, "/* b /* c */ d */", 1, 3),
            (CommentKind::Line, "// e", 1, 23),
        ]
    );
}

#[test]
fn comment_markers_inside_literals_are_not_comments() {
    let source = r#"val s = "// not a comment /* either */""#;
    let (with_comments, comments) = Lexer::new(source).tokenize_with_comments();
    assert!(comments.is_empty());
    assert_eq!(with_comments, tokens(source));
    assert_eq!(
        with_comments[3].kind,
        TokenKind::String("// not a comment /* either */".to_string())
    );
}

#[test]
fn leading_byte_order_mark_ignored() {
    let plain = tokens("package app\nfun main() {}");
    let marked = tokens("\u{FEFF}package app\nfun main() {}");
    assert_eq!(marked[0].kind, TokenKind::Package);
    assert_eq!(
        (marked[0].span.line, marked[0].span.column),
        (1, 1),
        "the mark takes no column"
    );
    assert_eq!(marked.len(), plain.len());
    for (marked, plain) in marked.iter().zip(&plain) {
        assert_eq!(marked.kind, plain.kind);
        assert_eq!(
            (marked.span.line, marked.span.column),
            (plain.span.line, plain.span.column)
        );
        // Spans index the source as given, so the mark's three bytes shift the offsets.
        assert_eq!(marked.span.start, plain.span.start + 3);
        assert_eq!(marked.span.end, plain.span.end + 3);
    }

    // The comment-collecting mode skips the mark too.
    let (collected, comments) = Lexer::new("\u{FEFF}// note\nval x = 1").tokenize_with_comments();
    assert!(collected.iter().all(|token| !is_error(&token.kind)));
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].span.column, 1);

    // A file of only a mark is empty.
    assert_eq!(kinds("\u{FEFF}"), vec![TokenKind::Eof]);
}

#[test]
fn byte_order_mark_elsewhere_rejected() {
    for source in [
        "val x = 1\n\u{FEFF}val y = 2",
        "\u{FEFF}\u{FEFF}val x = 1",
        " \u{FEFF}val x = 1",
        "val x = \u{FEFF}1",
    ] {
        assert_eq!(
            kinds(source).iter().filter(|kind| is_error(kind)).count(),
            1,
            "{:?}",
            source
        );
    }
}
