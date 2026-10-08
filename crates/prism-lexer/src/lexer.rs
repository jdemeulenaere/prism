//! Fast hand-written tokenizer for the Prism language.

use crate::token::{Token, TokenKind};
use prism_ast::Span;

/// Tokenizer for Prism source code.
pub struct Lexer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    cursor: usize,
    line: u32,
    column: u32,
    had_unclosed_comment: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            cursor: 0,
            line: 1,
            column: 1,
            had_unclosed_comment: false,
        }
    }

    /// Tokenizes the entire source input into a `Vec<Token>`.
    pub fn tokenize_all(mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    /// Fetches the next token from the input stream.
    pub fn next_token(&mut self) -> Token {
        let saw_newline = self.skip_whitespace_and_comments();

        let start_pos = self.cursor;
        let start_line = self.line;
        let start_col = self.column;

        if self.had_unclosed_comment {
            self.had_unclosed_comment = false;
            return Token::with_newline(
                TokenKind::Error("Unterminated block comment".to_string()),
                Span::new(start_pos, start_pos, start_line, start_col),
                saw_newline,
            );
        }

        if self.is_eof() {
            return Token::with_newline(
                TokenKind::Eof,
                Span::new(start_pos, start_pos, start_line, start_col),
                saw_newline,
            );
        }

        let ch = self.peek_byte();

        let kind = match ch {
            b'+' => {
                self.bump();
                TokenKind::Plus
            }
            b'-' => {
                self.bump();
                if self.peek_byte() == b'>' {
                    self.bump();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                self.bump();
                TokenKind::Star
            }
            b'/' => {
                self.bump();
                TokenKind::Slash
            }
            b'%' => {
                self.bump();
                TokenKind::Percent
            }
            b'=' => {
                self.bump();
                if self.peek_byte() == b'=' {
                    self.bump();
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            b'!' => {
                self.bump();
                if self.peek_byte() == b'=' {
                    self.bump();
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                self.bump();
                if self.peek_byte() == b'=' {
                    self.bump();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                self.bump();
                if self.peek_byte() == b'=' {
                    self.bump();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'&' => {
                self.bump();
                if self.peek_byte() == b'&' {
                    self.bump();
                    TokenKind::AmpAmp
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                self.bump();
                if self.peek_byte() == b'|' {
                    self.bump();
                    TokenKind::PipePipe
                } else {
                    TokenKind::Error("Unexpected single '|'".to_string())
                }
            }
            b'.' => {
                self.bump();
                if self.peek_byte() == b'.' {
                    self.bump();
                    TokenKind::DotDot
                } else {
                    TokenKind::Dot
                }
            }
            b'?' => {
                self.bump();
                if self.peek_byte() == b'.' {
                    self.bump();
                    TokenKind::QuestionDot
                } else if self.peek_byte() == b':' {
                    self.bump();
                    TokenKind::QuestionColon
                } else {
                    TokenKind::Question
                }
            }
            b':' => {
                self.bump();
                TokenKind::Colon
            }
            b';' => {
                self.bump();
                TokenKind::Semicolon
            }
            b',' => {
                self.bump();
                TokenKind::Comma
            }
            b'(' => {
                self.bump();
                TokenKind::LParen
            }
            b')' => {
                self.bump();
                TokenKind::RParen
            }
            b'{' => {
                self.bump();
                TokenKind::LBrace
            }
            b'}' => {
                self.bump();
                TokenKind::RBrace
            }
            b'[' => {
                self.bump();
                TokenKind::LBracket
            }
            b']' => {
                self.bump();
                TokenKind::RBracket
            }
            b'"' => self.lex_string(),
            b'\'' => self.lex_char(),
            b'0'..=b'9' => self.lex_number(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident_or_keyword(),
            _ => {
                let bad = self.bump_char().unwrap_or(' ');
                TokenKind::Error(format!("Unexpected character: {:?}", bad))
            }
        };

        let span = Span::new(start_pos, self.cursor, start_line, start_col);
        Token::with_newline(kind, span, saw_newline)
    }

    fn lex_ident_or_keyword(&mut self) -> TokenKind {
        let start = self.cursor;
        while !self.is_eof() {
            let b = self.peek_byte();
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.bump();
            } else {
                break;
            }
        }
        let text = &self.input[start..self.cursor];
        if let Some(kw) = TokenKind::keyword_from_str(text) {
            kw
        } else {
            TokenKind::Ident(text.to_string())
        }
    }

    fn lex_number(&mut self) -> TokenKind {
        let start = self.cursor;
        let mut is_float = false;

        // Check for 0x / 0b prefix
        if self.peek_byte() == b'0' && self.cursor + 1 < self.bytes.len() {
            let next = self.bytes[self.cursor + 1];
            if next == b'x' || next == b'X' {
                self.bump(); // 0
                self.bump(); // x
                while !self.is_eof()
                    && (self.peek_byte().is_ascii_hexdigit() || self.peek_byte() == b'_')
                {
                    self.bump();
                }
                let raw = self.input[start + 2..self.cursor].replace('_', "");
                return match i64::from_str_radix(&raw, 16) {
                    Ok(val) => TokenKind::Int(val),
                    Err(_) => TokenKind::Error(format!("Invalid hex literal: 0x{}", raw)),
                };
            } else if next == b'b' || next == b'B' {
                self.bump(); // 0
                self.bump(); // b
                while !self.is_eof()
                    && (self.peek_byte() == b'0'
                        || self.peek_byte() == b'1'
                        || self.peek_byte() == b'_')
                {
                    self.bump();
                }
                let raw = self.input[start + 2..self.cursor].replace('_', "");
                return match i64::from_str_radix(&raw, 2) {
                    Ok(val) => TokenKind::Int(val),
                    Err(_) => TokenKind::Error(format!("Invalid binary literal: 0b{}", raw)),
                };
            }
        }

        while !self.is_eof() {
            let b = self.peek_byte();
            if b.is_ascii_digit() || b == b'_' {
                self.bump();
            } else if b == b'.' {
                // Lookahead: is this a decimal point or a range `..` or method call `.foo`?
                if self.cursor + 1 < self.bytes.len()
                    && self.bytes[self.cursor + 1].is_ascii_digit()
                {
                    is_float = true;
                    self.bump(); // '.'
                    while !self.is_eof()
                        && (self.peek_byte().is_ascii_digit() || self.peek_byte() == b'_')
                    {
                        self.bump();
                    }
                } else {
                    // It is followed by `.` or non-digit, stop number here
                    break;
                }
            } else {
                break;
            }
        }

        // Check for scientific notation: e.g. 1e10, 3.14e-2
        if !self.is_eof() && (self.peek_byte() == b'e' || self.peek_byte() == b'E') {
            is_float = true;
            self.bump(); // 'e' or 'E'
            if !self.is_eof() && (self.peek_byte() == b'+' || self.peek_byte() == b'-') {
                self.bump();
            }
            while !self.is_eof() && (self.peek_byte().is_ascii_digit() || self.peek_byte() == b'_')
            {
                self.bump();
            }
        }

        let raw = self.input[start..self.cursor].replace('_', "");
        if is_float {
            match raw.parse::<f64>() {
                Ok(val) => TokenKind::Float(val),
                Err(_) => TokenKind::Error(format!("Invalid float literal: {}", raw)),
            }
        } else {
            match raw.parse::<i64>() {
                Ok(val) => TokenKind::Int(val),
                Err(_) => TokenKind::Error(format!("Invalid integer literal: {}", raw)),
            }
        }
    }

    fn lex_string(&mut self) -> TokenKind {
        self.bump(); // consume opening '"'
        let mut result = String::new();

        while !self.is_eof() {
            let b = self.peek_byte();
            if b == b'"' {
                self.bump();
                return TokenKind::String(result);
            } else if b == b'\\' {
                self.bump(); // consume '\\'
                if self.is_eof() {
                    return TokenKind::Error("Unterminated string escape".to_string());
                }
                match self.bump() {
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'\\' => result.push('\\'),
                    b'"' => result.push('"'),
                    b'\'' => result.push('\''),
                    b'0' => result.push('\0'),
                    b'u' => {
                        // Unicode escape: \u{1F680} or \u0041
                        if self.peek_byte() == b'{' {
                            self.bump();
                            let mut hex = String::new();
                            while !self.is_eof() && self.peek_byte() != b'}' {
                                hex.push(self.bump() as char);
                            }
                            if self.peek_byte() == b'}' {
                                self.bump();
                                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                    Some(c) => result.push(c),
                                    None => {
                                        return TokenKind::Error(format!(
                                            "Invalid unicode escape \\u{{{}}}",
                                            hex
                                        ))
                                    }
                                }
                            } else {
                                return TokenKind::Error("Unterminated unicode escape".to_string());
                            }
                        } else {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                if !self.is_eof() {
                                    hex.push(self.bump() as char);
                                }
                            }
                            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                Some(c) => result.push(c),
                                None => {
                                    return TokenKind::Error(format!(
                                        "Invalid unicode escape \\u{}",
                                        hex
                                    ))
                                }
                            }
                        }
                    }
                    other => {
                        result.push(other as char);
                    }
                }
            } else {
                match self.bump_char() {
                    Some(ch) => result.push(ch),
                    None => break,
                }
            }
        }

        TokenKind::Error("Unterminated string literal".to_string())
    }

    fn lex_char(&mut self) -> TokenKind {
        self.bump(); // consume opening '\''
        if self.is_eof() {
            return TokenKind::Error("Unterminated char literal".to_string());
        }

        let ch = if self.peek_byte() == b'\\' {
            self.bump();
            if self.is_eof() {
                return TokenKind::Error("Unterminated char escape".to_string());
            }
            match self.bump() {
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'\\' => '\\',
                b'\'' => '\'',
                b'"' => '"',
                b'0' => '\0',
                b'u' => {
                    if self.peek_byte() == b'{' {
                        self.bump();
                        let mut hex = String::new();
                        while !self.is_eof() && self.peek_byte() != b'}' {
                            hex.push(self.bump() as char);
                        }
                        if self.peek_byte() == b'}' {
                            self.bump();
                            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                Some(c) => c,
                                None => {
                                    return TokenKind::Error(format!(
                                        "Invalid unicode escape \\u{{{}}}",
                                        hex
                                    ))
                                }
                            }
                        } else {
                            return TokenKind::Error("Unterminated unicode escape".to_string());
                        }
                    } else {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            if !self.is_eof() {
                                hex.push(self.bump() as char);
                            }
                        }
                        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            Some(c) => c,
                            None => {
                                return TokenKind::Error(format!(
                                    "Invalid unicode escape \\u{}",
                                    hex
                                ))
                            }
                        }
                    }
                }
                other => other as char,
            }
        } else {
            match self.bump_char() {
                Some(c) => c,
                None => return TokenKind::Error("Unterminated char literal".to_string()),
            }
        };

        if self.is_eof() || self.peek_byte() != b'\'' {
            return TokenKind::Error("Expected closing single quote for char literal".to_string());
        }
        self.bump(); // consume closing '\''
        TokenKind::Char(ch)
    }

    fn skip_whitespace_and_comments(&mut self) -> bool {
        let mut saw_newline = false;

        while !self.is_eof() {
            let b = self.peek_byte();
            match b {
                b' ' | b'\t' | b'\r' => {
                    self.bump();
                }
                b'\n' => {
                    self.bump();
                    saw_newline = true;
                }
                b'/' => {
                    // Check if it's a line comment `//` or block comment `/*`
                    if self.cursor + 1 < self.bytes.len() {
                        let next = self.bytes[self.cursor + 1];
                        if next == b'/' {
                            // Line comment
                            self.bump();
                            self.bump();
                            while !self.is_eof() && self.peek_byte() != b'\n' {
                                self.bump();
                            }
                        } else if next == b'*' {
                            // Block comment with nesting
                            self.bump(); // '/'
                            self.bump(); // '*'
                            let mut depth = 1usize;
                            while !self.is_eof() && depth > 0 {
                                if self.peek_byte() == b'/'
                                    && self.cursor + 1 < self.bytes.len()
                                    && self.bytes[self.cursor + 1] == b'*'
                                {
                                    self.bump();
                                    self.bump();
                                    depth += 1;
                                } else if self.peek_byte() == b'*'
                                    && self.cursor + 1 < self.bytes.len()
                                    && self.bytes[self.cursor + 1] == b'/'
                                {
                                    self.bump();
                                    self.bump();
                                    depth -= 1;
                                } else {
                                    if self.peek_byte() == b'\n' {
                                        saw_newline = true;
                                    }
                                    self.bump();
                                }
                            }
                            if depth > 0 {
                                self.had_unclosed_comment = true;
                                break;
                            }
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        saw_newline
    }

    fn peek_byte(&self) -> u8 {
        if self.is_eof() {
            0
        } else {
            self.bytes[self.cursor]
        }
    }

    fn bump(&mut self) -> u8 {
        if self.is_eof() {
            return 0;
        }
        let b = self.bytes[self.cursor];
        self.cursor += 1;
        if b == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        b
    }

    fn bump_char(&mut self) -> Option<char> {
        if self.is_eof() {
            return None;
        }
        let remaining = &self.input[self.cursor..];
        let ch = remaining.chars().next()?;
        self.cursor += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn is_eof(&self) -> bool {
        self.cursor >= self.bytes.len()
    }
}
