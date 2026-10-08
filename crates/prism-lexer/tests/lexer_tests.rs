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
        let input = "42 3.5 0xFF 0b1010 4..10";
        let tokens = Lexer::new(input).tokenize_all();
        let kinds: Vec<TokenKind> = tokens.into_iter().map(|t| t.kind).collect();

        assert_eq!(
            kinds,
            vec![
                TokenKind::Int(42),
                TokenKind::Float(3.5),
                TokenKind::Int(255),
                TokenKind::Int(10),
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
        let input = r#"val msg = "café 🚀" val c = 'é' val rocket = '\u{1F680}'"#;
        let tokens = Lexer::new(input).tokenize_all();

        assert_eq!(tokens[3].kind, TokenKind::String("café 🚀".to_string()));
        assert_eq!(tokens[7].kind, TokenKind::Char('é'));
        assert_eq!(tokens[11].kind, TokenKind::Char('🚀'));
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
        assert!(matches!(tokens[0].kind, TokenKind::Error(_)));
    }

    #[test]
    fn test_scientific_notation_floats() {
        let input = "1e10 3.14e-2 2E+5";
        let tokens = Lexer::new(input).tokenize_all();

        assert_eq!(tokens[0].kind, TokenKind::Float(1e10));
        assert_eq!(tokens[1].kind, TokenKind::Float(3.14e-2));
        assert_eq!(tokens[2].kind, TokenKind::Float(2e5));
    }
}
