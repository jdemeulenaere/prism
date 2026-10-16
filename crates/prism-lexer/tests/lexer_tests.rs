#[cfg(test)]
mod tests {
    use prism_lexer::{Lexer, TokenKind};

    #[test]
    fn test_keywords_and_identifiers() {
        let input = "package import fun val var struct enum compose public private return if else when in this mut";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Package,
                TokenKind::Import,
                TokenKind::Fun,
                TokenKind::Val,
                TokenKind::Var,
                TokenKind::Struct,
                TokenKind::Enum,
                TokenKind::Compose,
                TokenKind::Public,
                TokenKind::Private,
                TokenKind::Return,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::When,
                TokenKind::In,
                TokenKind::This,
                TokenKind::Mut,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_numbers_and_ranges() {
        let input = "42 3.5 0xFF 0xFF_FF 0b1010 0b1010_0001 1_000_000 1.23_45 4..10";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Int(42),
                TokenKind::Float(3.5),
                TokenKind::Int(255),
                TokenKind::Int(65535),
                TokenKind::Int(10),
                TokenKind::Int(161),
                TokenKind::Int(1000000),
                TokenKind::Float(1.2345),
                TokenKind::Int(4),
                TokenKind::DotDot,
                TokenKind::Int(10),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_strings_and_escapes() {
        let input = r#""hello\nworld" 'x' '\n'"#;
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::String("hello\nworld".to_string()),
                TokenKind::Char('x'),
                TokenKind::Char('\n'),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_operators_and_punctuation() {
        let input = "+ - * / % == != < <= > >= && || ! & . ?. ?: ? -> = : ; , ( ) { } [ ]";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::EqEq,
                TokenKind::BangEq,
                TokenKind::Lt,
                TokenKind::LtEq,
                TokenKind::Gt,
                TokenKind::GtEq,
                TokenKind::AmpAmp,
                TokenKind::PipePipe,
                TokenKind::Bang,
                TokenKind::Amp,
                TokenKind::Dot,
                TokenKind::QuestionDot,
                TokenKind::QuestionColon,
                TokenKind::Question,
                TokenKind::Arrow,
                TokenKind::Eq,
                TokenKind::Colon,
                TokenKind::Semicolon,
                TokenKind::Comma,
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_comments_ignored() {
        let input = r#"
            // Line comment
            val x = 10 /* block comment */ + 20
        "#;
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Val,
                TokenKind::Ident("x".to_string()),
                TokenKind::Eq,
                TokenKind::Int(10),
                TokenKind::Plus,
                TokenKind::Int(20),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_root_import_tokens() {
        let input = "import /core/fmt";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Import,
                TokenKind::Slash,
                TokenKind::Ident("core".to_string()),
                TokenKind::Slash,
                TokenKind::Ident("fmt".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_utf8_strings_and_chars() {
        let input = r#"val msg = "café 🚀" val c = 'é' val rocket = '\u{1F680}' val a = "\u0041""#;
        let tokens = Lexer::new(input).tokenize_all();

        assert_eq!(tokens[3].kind, TokenKind::String("café 🚀".to_string()));
        assert_eq!(tokens[7].kind, TokenKind::Char('é'));
        assert_eq!(tokens[11].kind, TokenKind::Char('🚀'));
        assert_eq!(tokens[15].kind, TokenKind::String("A".to_string()));

        // Raw newline in single-line string rejected without consuming next line
        let raw_nl_input = "val s = \"hello\nval next = 42";
        let raw_nl_tokens = Lexer::new(raw_nl_input).tokenize_all();
        assert!(matches!(raw_nl_tokens[3].kind, TokenKind::Error(_)));
        assert_eq!(raw_nl_tokens[4].kind, TokenKind::Val);
        assert!(raw_nl_tokens[4].is_preceded_by_newline);

        // Malformed unicode escape doesn't consume closing quote
        let malformed_u = r#"val bad = "\u12""#;
        let malformed_tokens = Lexer::new(malformed_u).tokenize_all();
        assert!(matches!(malformed_tokens[3].kind, TokenKind::Error(_)));
        // Closing quote was not consumed, so it remains at span 15
        assert!(matches!(malformed_tokens[4].kind, TokenKind::Error(_)));
        assert_eq!(malformed_tokens[4].span.start, 15);

        // Unterminated string before EOF
        let unclosed = r#"val s = "unclosed"#;
        let unclosed_tokens = Lexer::new(unclosed).tokenize_all();
        assert!(matches!(unclosed_tokens[3].kind, TokenKind::Error(_)));
    }

    #[test]
    fn test_nested_block_comments() {
        let input = "/* outer /* inner */ still outer */ val x = 42";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Val,
                TokenKind::Ident("x".to_string()),
                TokenKind::Eq,
                TokenKind::Int(42),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_unclosed_block_comment() {
        let input = "/* unclosed comment";
        let tokens = Lexer::new(input).tokenize_all();
        assert_eq!(
            tokens[0].kind,
            TokenKind::Error("Unterminated block comment".to_string())
        );
        assert_eq!(tokens[0].span.start, 0);
        assert_eq!(tokens[0].span.end, 19);
        assert_eq!(tokens[0].span.line, 1);
        assert_eq!(tokens[0].span.column, 1);

        // Nested unclosed comment
        let nested_input = "val a = 1 /* outer /* inner */ still open";
        let tokens2 = Lexer::new(nested_input).tokenize_all();
        assert_eq!(tokens2[3].kind, TokenKind::Int(1));
        assert_eq!(
            tokens2[4].kind,
            TokenKind::Error("Unterminated block comment".to_string())
        );
        assert_eq!(tokens2[4].span.start, 10);
        assert_eq!(tokens2[4].span.end, 41);
    }

    #[test]
    fn test_scientific_notation_floats() {
        let input = "1e10 3.14e-2 2E+5";
        let tokens = Lexer::new(input).tokenize_all();

        assert_eq!(tokens[0].kind, TokenKind::Float(1e10));
        assert_eq!(tokens[1].kind, TokenKind::Float(3.14e-2));
        assert_eq!(tokens[2].kind, TokenKind::Float(2e5));
    }

    #[test]
    fn test_newline_tracking() {
        let input = "val a = 1\nval b = 2\r\n(3 + 4)\n// line comment\nval c = 5\n/* multiline\ncomment */val d = 6 /* inline */ + 7";
        let tokens = Lexer::new(input).tokenize_all();

        // val a = 1 (on first line)
        assert_eq!(tokens[0].kind, TokenKind::Val);
        assert!(!tokens[0].is_preceded_by_newline);
        assert_eq!(tokens[1].kind, TokenKind::Ident("a".to_string()));
        assert!(!tokens[1].is_preceded_by_newline);
        assert_eq!(tokens[2].kind, TokenKind::Eq);
        assert!(!tokens[2].is_preceded_by_newline);
        assert_eq!(tokens[3].kind, TokenKind::Int(1));
        assert!(!tokens[3].is_preceded_by_newline);

        // val b = 2 (preceded by \n)
        assert_eq!(tokens[4].kind, TokenKind::Val);
        assert!(tokens[4].is_preceded_by_newline);

        // (3 + 4) (preceded by \r\n)
        assert_eq!(tokens[8].kind, TokenKind::LParen);
        assert!(tokens[8].is_preceded_by_newline);

        // val c = 5 (preceded by line comment + newline)
        let val_c = tokens
            .iter()
            .find(|t| matches!(&t.kind, TokenKind::Ident(s) if s == "c"))
            .unwrap();
        let val_before_c_idx = tokens
            .iter()
            .position(|t| matches!(&t.kind, TokenKind::Ident(s) if s == "c"))
            .unwrap()
            - 1;
        assert_eq!(tokens[val_before_c_idx].kind, TokenKind::Val);
        assert!(tokens[val_before_c_idx].is_preceded_by_newline);
        assert!(!val_c.is_preceded_by_newline);

        // val d = 6 (preceded by multiline comment with newline)
        let val_before_d_idx = tokens
            .iter()
            .position(|t| matches!(&t.kind, TokenKind::Ident(s) if s == "d"))
            .unwrap()
            - 1;
        assert_eq!(tokens[val_before_d_idx].kind, TokenKind::Val);
        assert!(tokens[val_before_d_idx].is_preceded_by_newline);

        // + 7 (preceded by inline block comment on same line)
        let plus_tok = tokens.iter().find(|t| t.kind == TokenKind::Plus).unwrap();
        assert!(!plus_tok.is_preceded_by_newline);
    }

    #[test]
    fn test_escape_followed_by_multibyte_char_does_not_panic() {
        // A backslash before a non-ASCII char used to read one byte of the
        // UTF-8 sequence and panic on the next char boundary check.
        for src in [
            r#"fun f() = "\é""#,
            r#"fun f() = '\é'"#,
            r#"fun f() = "a\🚀""#,
        ] {
            let tokens = Lexer::new(src).tokenize_all();
            assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof, "{}", src);
        }
    }

    #[test]
    fn test_invalid_literals_rejected() {
        let error = |src: &str| match &Lexer::new(src).tokenize_all()[5].kind {
            TokenKind::Error(msg) => msg.clone(),
            other => panic!("{}: expected error, found {:?}", src, other),
        };
        assert_eq!(error(r#"fun f() = "\q""#), "Unknown escape sequence '\\q'");
        assert_eq!(error(r#"fun f() = '\q'"#), "Unknown escape sequence '\\q'");
        assert!(error(r#"fun f() = "\é""#).starts_with("Unknown escape sequence"));
        assert_eq!(error("fun f() = ''"), "Empty char literal");
        assert_eq!(
            error("fun f() = 1e999"),
            "Float literal out of range: 1e999"
        );
        assert_eq!(
            error("fun f() = 9223372036854775809"),
            "Integer literal out of range: 9223372036854775809"
        );

        // The magnitude of i64::MIN is kept for the parser to negate.
        let tokens = Lexer::new("fun f() = 9223372036854775808").tokenize_all();
        assert_eq!(tokens[5].kind, TokenKind::Int(9223372036854775808));
    }

    #[test]
    fn test_columns_count_characters() {
        // S4: non-ASCII text in comments, strings and chars advances the
        // column by one per character, not per byte.
        for (src, column) in [
            ("/* café */ fun", 12),
            ("// é\n🚀🚀 fun", 4),
            (r#""café" fun"#, 8),
            ("'é' fun", 5),
        ] {
            let tokens = Lexer::new(src).tokenize_all();
            let fun = tokens.iter().find(|t| t.kind == TokenKind::Fun).unwrap();
            assert_eq!(fun.span.column, column, "{}", src);
        }
    }
}
