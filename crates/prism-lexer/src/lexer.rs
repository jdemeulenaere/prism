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
    unclosed_comment_span: Option<Span>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            cursor: 0,
            line: 1,
            column: 1,
            unclosed_comment_span: None,
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

        if let Some(comment_span) = self.unclosed_comment_span.take() {
            return Token::with_newline(
                TokenKind::Error("Unterminated block comment".to_string()),
                comment_span,
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
                    if self.peek_byte() == b'<' {
                        self.bump();
                        TokenKind::DotDotLess
                    } else {
                        TokenKind::DotDot
                    }
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
                    Ok(val) => TokenKind::Int(val as u64),
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
                    Ok(val) => TokenKind::Int(val as u64),
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
                Ok(val) if val.is_infinite() => {
                    TokenKind::Error(format!("Float literal out of range: {}", raw))
                }
                Ok(val) => TokenKind::Float(val),
                Err(_) => TokenKind::Error(format!("Invalid float literal: {}", raw)),
            }
        } else {
            match raw.parse::<u64>() {
                Ok(val) if val <= i64::MIN.unsigned_abs() => TokenKind::Int(val),
                Ok(_) => TokenKind::Error(format!("Integer literal out of range: {}", raw)),
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
            } else if b == b'\n' {
                // Reject unescaped raw newlines in single-line strings without consuming the newline
                return TokenKind::Error(
                    "Unterminated string literal: raw unescaped newline".to_string(),
                );
            } else if b == b'\\' {
                self.bump(); // consume '\\'
                if self.is_eof() {
                    return TokenKind::Error("Unterminated string escape".to_string());
                }
                match self.lex_escape(b'"') {
                    Ok(c) => result.push(c),
                    Err(msg) => return TokenKind::Error(msg),
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
        if self.peek_byte() == b'\n' {
            return TokenKind::Error("Unescaped newline in char literal".to_string());
        }
        if self.peek_byte() == b'\'' {
            self.bump();
            return TokenKind::Error("Empty char literal".to_string());
        }

        let ch = if self.peek_byte() == b'\\' {
            self.bump();
            if self.is_eof() {
                return TokenKind::Error("Unterminated char escape".to_string());
            }
            match self.lex_escape(b'\'') {
                Ok(c) => c,
                Err(msg) => return TokenKind::Error(msg),
            }
        } else {
            if self.peek_byte() == b'\n' {
                return TokenKind::Error("Unescaped newline in char literal".to_string());
            }
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

    /// Decodes the escape sequence after a consumed `\`. `quote` is the
    /// literal's closing delimiter, which ends a braced unicode escape early.
    fn lex_escape(&mut self, quote: u8) -> Result<char, String> {
        // Read a whole character: a byte-wise read would stop inside a
        // multi-byte UTF-8 sequence such as `\é`.
        let esc = match self.bump_char() {
            Some(c) => c,
            None => return Err("Unterminated escape sequence".to_string()),
        };
        match esc {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            '\\' => Ok('\\'),
            '"' => Ok('"'),
            '\'' => Ok('\''),
            '0' => Ok('\0'),
            'u' => self.lex_unicode_escape(quote),
            other => Err(format!(
                "Unknown escape sequence '\\{}'",
                other.escape_debug()
            )),
        }
    }

    /// Decodes `{1F680}` or `0041` after a consumed `\u`.
    fn lex_unicode_escape(&mut self, quote: u8) -> Result<char, String> {
        if self.peek_byte() == b'{' {
            self.bump();
            let mut hex = String::new();
            while !self.is_eof()
                && self.peek_byte() != b'}'
                && self.peek_byte() != quote
                && self.peek_byte() != b'\n'
            {
                let ch = self.peek_byte();
                if ch.is_ascii_hexdigit() {
                    hex.push(self.bump() as char);
                } else {
                    break;
                }
            }
            if self.peek_byte() == b'}' {
                self.bump();
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("Invalid unicode escape \\u{{{}}}", hex))
            } else {
                Err("Unterminated unicode escape".to_string())
            }
        } else {
            let mut hex = String::new();
            for _ in 0..4 {
                if !self.is_eof() && self.peek_byte().is_ascii_hexdigit() {
                    hex.push(self.bump() as char);
                } else {
                    break;
                }
            }
            if hex.len() == 4 {
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("Invalid unicode escape \\u{}", hex))
            } else {
                Err(format!(
                    "Invalid unicode escape: expected 4 hex digits, found '{}'",
                    hex
                ))
            }
        }
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
                            let comment_start_pos = self.cursor;
                            let comment_start_line = self.line;
                            let comment_start_col = self.column;
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
                                self.unclosed_comment_span = Some(Span::new(
                                    comment_start_pos,
                                    self.cursor,
                                    comment_start_line,
                                    comment_start_col,
                                ));
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
        } else if b & 0xC0 != 0x80 {
            // Columns count characters: UTF-8 continuation bytes don't advance.
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
