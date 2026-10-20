//! Fast hand-written tokenizer for the Prism language.

use crate::token::{Comment, CommentKind, Token, TokenKind};
use prism_ast::Span;

/// What the lexer is reading inside a string template (D3 of `string-templates`). Outside any
/// template the stack of modes is empty, and the lexer reads ordinary tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A template's text, between its quotes and parts. `start` is the offset, line and column
    /// of its opening quote, which the errors of an unterminated template span from.
    Template { start: (usize, u32, u32) },
    /// The ordinary tokens of an expression part, with the number of `{` opened inside it and
    /// not closed yet: the `}` met with none open closes the part.
    TemplateExpression { open_braces: usize },
}

/// Tokenizer for Prism source code.
pub struct Lexer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    cursor: usize,
    line: u32,
    column: u32,
    unclosed_comment_span: Option<Span>,
    /// The comments skipped so far, collected only in the comment-collecting mode
    /// ([`Self::tokenize_with_comments`]); `None` when compiling, which never needs them.
    comments: Option<Vec<Comment>>,
    /// The templates and expression parts open at the cursor, innermost last.
    modes: Vec<Mode>,
}

impl<'a> Lexer<'a> {
    /// Creates a lexer over `input`. One byte order mark (U+FEFF) at the very start is skipped:
    /// spans keep byte offsets into `input` as given, but the mark takes no column, so the first
    /// token is at column 1. A U+FEFF anywhere else is an error token like any other stray
    /// character.
    pub fn new(input: &'a str) -> Self {
        let byte_order_mark = '\u{FEFF}';
        let skipped = if input.starts_with(byte_order_mark) {
            byte_order_mark.len_utf8()
        } else {
            0
        };
        Self {
            input,
            bytes: input.as_bytes(),
            cursor: skipped,
            line: 1,
            column: 1,
            unclosed_comment_span: None,
            comments: None,
            modes: Vec::new(),
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

    /// Tokenizes the entire source input like [`Self::tokenize_all`], which yields the same
    /// tokens, and also returns every comment in source order, for tools that rewrite source
    /// (the formatter).
    pub fn tokenize_with_comments(mut self) -> (Vec<Token>, Vec<Comment>) {
        self.comments = Some(Vec::new());
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        (tokens, self.comments.take().unwrap_or_default())
    }

    /// Records a comment from `start` (with its line and column) to the cursor, when collecting.
    fn record_comment(&mut self, kind: CommentKind, start: usize, line: u32, column: u32) {
        if let Some(comments) = &mut self.comments {
            comments.push(Comment {
                kind,
                span: Span::new(start, self.cursor, line, column),
            });
        }
    }

    /// Fetches the next token from the input stream.
    pub fn next_token(&mut self) -> Token {
        if let Some(Mode::Template { start }) = self.modes.last().copied() {
            return self.lex_template_text(start);
        }
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
            if let Some(Mode::Template { start }) = self.outermost_template() {
                // An expression part still open at the end of the file: one error for the whole
                // template, then the end of the file.
                self.modes.clear();
                return Token::with_newline(
                    TokenKind::Error("Unterminated string template".to_string()),
                    Span::new(start.0, self.cursor, start.1, start.2),
                    saw_newline,
                );
            }
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
                match self.peek_byte() {
                    b'=' => {
                        self.bump();
                        TokenKind::PlusEqual
                    }
                    b'+' => {
                        self.bump();
                        TokenKind::PlusPlus
                    }
                    _ => TokenKind::Plus,
                }
            }
            b'-' => {
                self.bump();
                match self.peek_byte() {
                    b'>' => {
                        self.bump();
                        TokenKind::Arrow
                    }
                    b'=' => {
                        self.bump();
                        TokenKind::MinusEqual
                    }
                    b'-' => {
                        self.bump();
                        TokenKind::MinusMinus
                    }
                    _ => TokenKind::Minus,
                }
            }
            b'*' => {
                self.bump();
                self.with_equal(TokenKind::Star, TokenKind::StarEqual)
            }
            b'/' => {
                self.bump();
                self.with_equal(TokenKind::Slash, TokenKind::SlashEqual)
            }
            b'%' => {
                self.bump();
                self.with_equal(TokenKind::Percent, TokenKind::PercentEqual)
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
                } else if self.at_not_in() {
                    // `!in` is one token unless `in` starts a longer name (`!inside`).
                    self.bump();
                    self.bump();
                    TokenKind::NotIn
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
                if let Some(Mode::TemplateExpression { open_braces }) = self.modes.last_mut() {
                    *open_braces += 1;
                }
                TokenKind::LBrace
            }
            b'}' => {
                self.bump();
                match self.modes.last_mut() {
                    Some(Mode::TemplateExpression { open_braces: 0 }) => {
                        self.modes.pop();
                        TokenKind::TemplateExpressionEnd
                    }
                    Some(Mode::TemplateExpression { open_braces }) => {
                        *open_braces -= 1;
                        TokenKind::RBrace
                    }
                    _ => TokenKind::RBrace,
                }
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

    /// Lexes a string literal from its opening quote. A literal without parts is one `String`
    /// token. At the first `$` that starts a part, the lexer goes back to just after the opening
    /// quote, enters the `Template` mode and returns `TemplateStart`, so the literal's text is
    /// then read as template text runs and parts (D3 of `string-templates`).
    fn lex_string(&mut self) -> TokenKind {
        let quote = (self.cursor, self.line, self.column);
        self.bump(); // consume opening '"'
        let after_quote = (self.cursor, self.line, self.column);
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
            } else if b == b'$' && self.starts_template_part() {
                (self.cursor, self.line, self.column) = after_quote;
                self.modes.push(Mode::Template { start: quote });
                return TokenKind::TemplateStart;
            } else {
                match self.bump_char() {
                    Some(ch) => result.push(ch),
                    None => break,
                }
            }
        }

        TokenKind::Error("Unterminated string literal".to_string())
    }

    /// Whether the `$` at the cursor begins a template part: a name part when an ASCII letter or
    /// `_` follows it, an expression part when `{` does. Any other `$` is the character `$`.
    fn starts_template_part(&self) -> bool {
        matches!(
            self.bytes.get(self.cursor + 1),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'{')
        )
    }

    /// The outermost template open at the cursor, which an error that ends every open mode
    /// spans from.
    fn outermost_template(&self) -> Option<Mode> {
        self.modes
            .iter()
            .find(|mode| matches!(mode, Mode::Template { .. }))
            .copied()
    }

    /// Lexes the next token of a template's text, in the `Template` mode: its closing quote, a
    /// `$name` part, the `${` of an expression part, or a run of text up to one of those. The
    /// text is read as written, without skipping whitespace or comments. A raw newline ends the
    /// template's mode with an error, without consuming the newline, and the end of the file ends
    /// every open mode with one error. An error inside the text (an unknown escape, a keyword
    /// after `$`) is one error token, after which the template goes on, so that its closing quote
    /// still closes it.
    fn lex_template_text(&mut self, template_start: (usize, u32, u32)) -> Token {
        let (start, line, column) = (self.cursor, self.line, self.column);
        let span = |lexer: &Self| Span::new(start, lexer.cursor, line, column);
        if self.is_eof() {
            let outermost = match self.outermost_template() {
                Some(Mode::Template { start }) => start,
                _ => template_start,
            };
            self.modes.clear();
            return Token::new(
                TokenKind::Error("Unterminated string template".to_string()),
                Span::new(outermost.0, self.cursor, outermost.1, outermost.2),
            );
        }
        let kind = match self.peek_byte() {
            b'"' => {
                self.bump();
                self.modes.pop();
                TokenKind::TemplateEnd
            }
            b'\n' => {
                self.modes.pop();
                let (start, line, column) = template_start;
                return Token::new(
                    TokenKind::Error(
                        "Unterminated string template: raw unescaped newline".to_string(),
                    ),
                    Span::new(start, self.cursor, line, column),
                );
            }
            b'$' if self.bytes.get(self.cursor + 1) == Some(&b'{') => {
                self.bump();
                self.bump();
                self.modes.push(Mode::TemplateExpression { open_braces: 0 });
                TokenKind::TemplateExpressionStart
            }
            b'$' if self.starts_template_part() => {
                self.bump();
                let name_start = self.cursor;
                while self.peek_byte().is_ascii_alphanumeric() || self.peek_byte() == b'_' {
                    self.bump();
                }
                let name = &self.input[name_start..self.cursor];
                match TokenKind::keyword_from_str(name) {
                    // `compose` is a soft keyword, an identifier wherever one is expected.
                    None | Some(TokenKind::This | TokenKind::Compose) => {
                        TokenKind::TemplateName(name.to_string())
                    }
                    Some(_) => TokenKind::Error(format!(
                        "'${name}' starts no template part, since '{name}' is a keyword: write \
                         '\\${name}' for the text, or '${{...}}' around an expression"
                    )),
                }
            }
            _ => self.lex_template_text_run(),
        };
        Token::new(kind, span(self))
    }

    /// A run of template text from the cursor up to the closing quote, a raw newline, the end of
    /// the file or a `$` that starts a part, with escapes decoded; or the error of a malformed
    /// escape, consuming the escape. The run is never empty, since the cursor is at none of those.
    fn lex_template_text_run(&mut self) -> TokenKind {
        let mut text = String::new();
        while !self.is_eof() {
            match self.peek_byte() {
                b'"' | b'\n' => break,
                b'$' if self.starts_template_part() => break,
                b'\\' => {
                    self.bump();
                    if self.is_eof() {
                        return TokenKind::Error("Unterminated string escape".to_string());
                    }
                    match self.lex_escape(b'"') {
                        Ok(c) => text.push(c),
                        Err(message) => return TokenKind::Error(message),
                    }
                }
                _ => match self.bump_char() {
                    Some(ch) => text.push(ch),
                    None => break,
                },
            }
        }
        TokenKind::TemplateText(text)
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
            '$' => Ok('$'),
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
                            let (start, line, column) = (self.cursor, self.line, self.column);
                            self.bump();
                            self.bump();
                            while !self.is_eof() && self.peek_byte() != b'\n' {
                                self.bump();
                            }
                            self.record_comment(CommentKind::Line, start, line, column);
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
                            self.record_comment(
                                CommentKind::Block,
                                comment_start_pos,
                                comment_start_line,
                                comment_start_col,
                            );
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

    /// `joined` when the next byte is `=`, which it consumes (`*` then `=` is `*=`), otherwise
    /// `single`.
    fn with_equal(&mut self, single: TokenKind, joined: TokenKind) -> TokenKind {
        if self.peek_byte() == b'=' {
            self.bump();
            joined
        } else {
            single
        }
    }

    /// Whether the input continues with `in` not followed by a character that could continue an
    /// identifier, right after a `!`.
    fn at_not_in(&self) -> bool {
        let rest = &self.bytes[self.cursor..];
        rest.starts_with(b"in")
            && rest
                .get(2)
                .is_none_or(|&b| !(b.is_ascii_alphanumeric() || b == b'_'))
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
