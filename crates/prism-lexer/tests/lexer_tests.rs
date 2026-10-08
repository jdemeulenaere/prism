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
}
