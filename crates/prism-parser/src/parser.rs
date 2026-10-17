//! Pratt parser and recursive-descent AST generator for Prism.

use crate::error::ParseError;
use prism_ast::*;
use prism_lexer::{Lexer, Token, TokenKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Lowest = 0,
    Assign = 1,     // =
    LogicalOr = 2,  // ||
    LogicalAnd = 3, // &&
    Equality = 4,   // ==, !=
    Comparison = 5, // <, <=, >, >=
    Elvis = 6,      // ?:
    Range = 7,      // ..
    Sum = 8,        // +, -
    Product = 9,    // *, /, %
    Prefix = 10,    // -, !, &, &mut
    Postfix = 11,   // ., ?.
    Call = 12,      // ()
}

/// The deepest nesting of expressions and types the parser accepts.
///
/// The limit bounds both the parser's own recursion and the height of every
/// tree it builds, so recursive passes over the AST (drop, clone, the type
/// checker) cannot overflow the stack either. Deeper source is rejected with
/// `E2002` instead of aborting the process.
pub const MAX_NESTING_DEPTH: usize = 256;

/// The error for a prefix `*` anywhere but at the start of an assignment target.
const DEREF_OUTSIDE_TARGET: &str = "A prefix '*' writes through a reference and is only allowed on the left of '=', as in '*r = v'; reads through references are implicit";

pub struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    /// Number of `parse_expr` / `parse_type` calls currently on the stack.
    depth: usize,
    /// Deepest tree level reached since the innermost open `parse_expr` or
    /// `parse_type` started; `peak - depth` is the height of what it parsed.
    peak: usize,
    /// Whether a line break ends an expression, innermost delimiter last:
    /// `false` inside `( )` and `[ ]`, `true` inside `{ }` and at the top level.
    newline_modes: Vec<bool>,
}

impl Parser {
    pub fn new(mut tokens: Vec<Token>) -> Self {
        // Lookahead clamps to the last token, so the stream must end in Eof.
        if tokens.last().is_none_or(|t| t.kind != TokenKind::Eof) {
            let eof_span = tokens.last().map_or(Span::new(0, 0, 1, 1), |t| {
                let width = (t.span.end - t.span.start) as u32;
                Span::new(t.span.end, t.span.end, t.span.line, t.span.column + width)
            });
            tokens.push(Token::new(TokenKind::Eof, eof_span));
        }
        Self {
            tokens,
            cursor: 0,
            depth: 0,
            peak: 0,
            newline_modes: vec![true],
        }
    }

    pub fn from_source(source: &str) -> Self {
        let tokens = Lexer::new(source).tokenize_all();
        Self::new(tokens)
    }

    // --- Lookahead and Navigation ---

    fn peek(&self) -> &Token {
        &self.tokens[self.cursor.min(self.tokens.len() - 1)]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_ahead(&self, n: usize) -> &TokenKind {
        let idx = (self.cursor + n).min(self.tokens.len() - 1);
        &self.tokens[idx].kind
    }

    fn is_eof(&self) -> bool {
        self.peek_kind() == &TokenKind::Eof
    }

    fn bump(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.cursor < self.tokens.len() - 1 {
            self.cursor += 1;
        }
        tok
    }

    fn expect(&mut self, expected: TokenKind) -> Result<Token, ParseError> {
        let current = self.peek();
        if current.kind == expected {
            Ok(self.bump())
        } else {
            Err(ParseError::new(
                format!("Expected '{}', found '{}'", expected, current.kind),
                current.span,
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Ident(s) => {
                self.bump();
                Ok((s, tok.span))
            }
            TokenKind::Compose => {
                self.bump();
                Ok(("compose".to_string(), tok.span))
            }
            _ => Err(ParseError::new(
                format!("Expected identifier, found '{}'", tok.kind),
                tok.span,
            )),
        }
    }

    /// Consumes the first character of the current two-character token as a
    /// `first` token and leaves the rest as a `rest` token, for operators the
    /// lexer joined where the grammar needs them apart (`Heap<Int>= x`, `&&x`).
    fn split_token(&mut self, first: TokenKind, rest: TokenKind) -> Token {
        let tok = &mut self.tokens[self.cursor];
        let span = tok.span;
        let head = Token::with_newline(
            first,
            Span::new(span.start, span.start + 1, span.line, span.column),
            tok.is_preceded_by_newline,
        );
        tok.kind = rest;
        tok.span = Span::new(span.start + 1, span.end, span.line, span.column + 1);
        tok.is_preceded_by_newline = false;
        head
    }

    /// Whether the current token closes a generic argument list (`>`, or the
    /// `>` of a `>=` that the lexer joined).
    fn at_closing_angle(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Gt | TokenKind::GtEq)
    }

    fn expect_closing_angle(&mut self) -> Result<Token, ParseError> {
        if self.peek_kind() == &TokenKind::GtEq {
            Ok(self.split_token(TokenKind::Gt, TokenKind::Eq))
        } else {
            self.expect(TokenKind::Gt)
        }
    }

    /// Consumes a prefix `&`, splitting `&&` into two borrows.
    fn bump_amp(&mut self) -> Token {
        if self.peek_kind() == &TokenKind::AmpAmp {
            self.split_token(TokenKind::Amp, TokenKind::Amp)
        } else {
            self.bump()
        }
    }

    // --- Newline Significance ---

    /// Whether the current token starts a new line in a context where line
    /// breaks end an expression (outside parentheses and brackets).
    fn at_line_break(&self) -> bool {
        self.peek().is_preceded_by_newline && self.newline_modes.last() != Some(&false)
    }

    /// Runs `f` with line breaks significant (`{ }`) or insignificant (`( )`, `[ ]`).
    fn with_newlines<T>(
        &mut self,
        significant: bool,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        self.newline_modes.push(significant);
        let result = f(self);
        self.newline_modes.pop();
        result
    }

    /// Reports a missing separator unless the current token is `}`, the end of
    /// the file, or on a new line; `what` names the items and `separators` the
    /// accepted separators, for the message.
    fn expect_separated(&self, what: &str, separators: &str) -> Result<(), ParseError> {
        if matches!(self.peek_kind(), TokenKind::RBrace | TokenKind::Eof)
            || self.peek().is_preceded_by_newline
        {
            Ok(())
        } else {
            Err(ParseError::new(
                format!(
                    "Expected {} between {}, found '{}'",
                    separators,
                    what,
                    self.peek_kind()
                ),
                self.peek().span,
            ))
        }
    }

    // --- Source File Parsing ---

    pub fn parse_source_file(&mut self) -> Result<SourceFile, ParseError> {
        self.parse_source_file_inner()
            .map_err(|err| self.surface_lexer_error(err))
    }

    /// The parser never consumes a lexer `Error` token, so a syntax error
    /// raised while one is current is caused by it: report the lexer's own
    /// message and span instead of "unexpected token '<Error: ...>'".
    fn surface_lexer_error(&self, err: ParseError) -> ParseError {
        match self.peek_kind() {
            TokenKind::Error(message) if err.code == "E2001" => {
                ParseError::new(message.clone(), self.peek().span)
            }
            _ => err,
        }
    }

    fn parse_source_file_inner(&mut self) -> Result<SourceFile, ParseError> {
        let start_span = self.peek().span;
        let mut package = None;
        let mut imports = Vec::new();
        let mut declarations = Vec::new();

        // 1. Check optional package declaration (only at file top)
        if self.peek_kind() == &TokenKind::Package && self.is_package_decl_ahead() {
            package = Some(self.parse_package_decl()?);
        }

        // 2. Parse imports
        while self.peek_kind() == &TokenKind::Import {
            imports.push(self.parse_import_decl()?);
        }

        // 3. Parse declarations until EOF
        while !self.is_eof() {
            declarations.push(self.parse_declaration()?);
        }

        let end_span = self.peek().span;
        let full_span = start_span.merge(end_span);

        Ok(SourceFile::new(package, imports, declarations, full_span))
    }

    fn is_package_decl_ahead(&self) -> bool {
        // package <ident> ... is a package decl.
        // package fun / package struct / package enum / package val is a package visibility modifier.
        matches!(self.peek_ahead(1), TokenKind::Ident(_))
    }

    fn parse_package_decl(&mut self) -> Result<PackageDecl, ParseError> {
        let start_span = self.expect(TokenKind::Package)?.span;
        let mut path = Vec::new();

        let (first, mut end_span) = self.expect_ident()?;
        path.push(first);

        while self.peek_kind() == &TokenKind::Dot {
            self.bump(); // '.'
            let (part, s) = self.expect_ident()?;
            path.push(part);
            end_span = s;
        }

        Ok(PackageDecl {
            path,
            span: start_span.merge(end_span),
        })
    }

    fn parse_import_decl(&mut self) -> Result<ImportDecl, ParseError> {
        let start_span = self.expect(TokenKind::Import)?.span;
        let mut is_root = false;
        let mut path = Vec::new();

        // Check if leading slash for root-anchored import: import /core/fmt
        if self.peek_kind() == &TokenKind::Slash {
            self.bump(); // '/'
            is_root = true;
        }

        self.reject_relative_import_segment()?;
        let (first, mut end_span) = self.expect_ident()?;
        path.push(first);

        // Either slash-separated (/core/fmt) or dot-separated (doo.util)
        while self.peek_kind() == &TokenKind::Slash || self.peek_kind() == &TokenKind::Dot {
            self.bump();
            self.reject_relative_import_segment()?;
            let (part, s) = self.expect_ident()?;
            path.push(part);
            end_span = s;
        }
        if self.peek_kind() == &TokenKind::DotDot {
            // `import a..b`: the lexer reads `..` as one token right after a segment.
            self.reject_relative_import_segment()?;
        }

        Ok(ImportDecl {
            is_root,
            path,
            span: start_span.merge(end_span),
        })
    }

    /// Rejects a `.` or `..` segment where an import path segment is expected. Import paths
    /// consist of identifiers only; relative navigation is expressed with root-anchored imports.
    fn reject_relative_import_segment(&self) -> Result<(), ParseError> {
        let segment = match self.peek_kind() {
            TokenKind::Dot => ".",
            TokenKind::DotDot => "..",
            _ => return Ok(()),
        };
        Err(ParseError::new(
            format!(
                "import paths cannot contain '.' or '..' (found '{}'); use a root-anchored import such as 'import /core/fmt'",
                segment
            ),
            self.peek().span,
        ))
    }

    // --- Declarations ---

    pub fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        // Parse visibility: default is Private (single-file compilation unit invariant)
        let mut visibility = Visibility::Private;
        let mut start_span = self.peek().span;

        match self.peek_kind() {
            TokenKind::Public => {
                start_span = self.bump().span;
                visibility = Visibility::Public;
            }
            TokenKind::Package => {
                start_span = self.bump().span;
                visibility = Visibility::Package;
            }
            TokenKind::Private => {
                start_span = self.bump().span;
                visibility = Visibility::Private;
            }
            _ => {}
        }

        // Check compose modifier (`compose fun`)
        let mut is_compose = false;
        if self.peek_kind() == &TokenKind::Compose {
            self.bump();
            is_compose = true;
        }

        match self.peek_kind() {
            TokenKind::Fun => {
                let func = self.parse_function(visibility, is_compose, start_span)?;
                Ok(Declaration::Function(func))
            }
            TokenKind::Struct => {
                if is_compose {
                    return Err(ParseError::new(
                        "'compose' modifier is only valid on functions ('compose fun')",
                        start_span,
                    ));
                }
                let st = self.parse_struct(visibility, start_span)?;
                Ok(Declaration::Struct(st))
            }
            TokenKind::Enum => {
                if is_compose {
                    return Err(ParseError::new(
                        "'compose' modifier is only valid on functions ('compose fun')",
                        start_span,
                    ));
                }
                let en = self.parse_enum(visibility, start_span)?;
                Ok(Declaration::Enum(en))
            }
            TokenKind::Var => Err(ParseError::new(
                "Top-level 'var' is not allowed: top-level declarations are immutable; keep mutable state in locals and parameters, or in a state handle in UI code",
                self.peek().span,
            )),
            TokenKind::Val => {
                if is_compose {
                    return Err(ParseError::new(
                        "'compose' modifier is only valid on functions ('compose fun')",
                        start_span,
                    ));
                }
                let v = self.parse_top_level_var(visibility, start_span)?;
                Ok(Declaration::Var(v))
            }
            _ => Err(ParseError::new(
                format!(
                    "Expected declaration (fun, struct, enum, val), found '{}'",
                    self.peek_kind()
                ),
                self.peek().span,
            )),
        }
    }

    fn parse_function(
        &mut self,
        visibility: Visibility,
        is_compose: bool,
        start_span: Span,
    ) -> Result<FunctionDecl, ParseError> {
        self.expect(TokenKind::Fun)?;
        let (name, _) = self.expect_ident()?;

        // Generic type parameters: <T, U>
        let type_params = self.parse_optional_type_params()?;

        // Parameters: (a: Int, b: Double = 0.0)
        self.expect(TokenKind::LParen)?;
        let params = self.with_newlines(false, Self::parse_params)?;
        let rparen_span = self.expect(TokenKind::RParen)?.span;

        // Return type: : ReturnType
        let mut return_type = None;
        if self.peek_kind() == &TokenKind::Colon {
            self.bump();
            return_type = Some(self.parse_type()?);
        }

        // Body: = expr OR { ... } OR none (forward declaration / interface)
        let mut body = None;
        let mut end_span = return_type.as_ref().map_or(rparen_span, |t| t.span);

        if self.peek_kind() == &TokenKind::Eq {
            self.bump(); // '='
            let expr = self.parse_expr(Precedence::Lowest)?;
            end_span = expr.span;
            body = Some(FunctionBody::Expr(expr));
        } else if self.peek_kind() == &TokenKind::LBrace {
            let block = self.parse_block_expr()?;
            end_span = block.span;
            body = Some(FunctionBody::Block(block));
        }

        Ok(FunctionDecl {
            visibility,
            is_compose,
            name,
            type_params,
            params,
            return_type,
            body,
            span: start_span.merge(end_span),
        })
    }

    /// Function parameters up to (not including) the closing `)`.
    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            let (param_name, p_span) = self.expect_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let mut default_value = None;
            if self.peek_kind() == &TokenKind::Eq {
                self.bump();
                default_value = Some(self.parse_expr(Precedence::Lowest)?);
            }
            let end_p_span = default_value.as_ref().map(|d| d.span).unwrap_or(ty.span);
            params.push(Param {
                name: param_name,
                ty,
                default_value,
                span: p_span.merge(end_p_span),
            });

            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_struct(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<StructDecl, ParseError> {
        self.expect(TokenKind::Struct)?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_optional_type_params()?;

        let mut fields = Vec::new();
        let marker;

        let end_span = if self.peek_kind() == &TokenKind::LParen {
            // Shorthand struct constructor: struct EmployeeId(val id: UInt32)
            self.bump();
            while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
                let f_start = self.peek().span;
                let is_var = match self.peek_kind() {
                    TokenKind::Var => {
                        self.bump();
                        true
                    }
                    TokenKind::Val => {
                        self.bump();
                        false
                    }
                    _ => false,
                };
                let (fname, _) = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let fty = self.parse_type()?;
                reject_stored_borrow(&fty, "Struct fields")?;
                let f_end = fty.span;
                fields.push(StructField {
                    is_var,
                    name: fname,
                    ty: fty,
                    span: f_start.merge(f_end),
                });
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            let rparen_span = self.expect(TokenKind::RParen)?.span;
            let parsed = self.parse_optional_marker()?;
            marker = parsed.map(|(parsed, _)| parsed);
            parsed.map_or(rparen_span, |(_, marker_span)| marker_span)
        } else {
            // Block struct: struct Point : Copy { val x: Double, val y: Double }
            marker = self.parse_optional_marker()?.map(|(parsed, _)| parsed);
            self.expect(TokenKind::LBrace)?;
            while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
                let f_start = self.peek().span;
                let is_var = match self.peek_kind() {
                    TokenKind::Var => {
                        self.bump();
                        true
                    }
                    TokenKind::Val => {
                        self.bump();
                        false
                    }
                    _ => {
                        return Err(ParseError::new(
                            format!(
                                "Expected 'val' or 'var' for struct field, found '{}'",
                                self.peek_kind()
                            ),
                            f_start,
                        ));
                    }
                };
                let (fname, _) = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let fty = self.parse_type()?;
                reject_stored_borrow(&fty, "Struct fields")?;
                let f_end = fty.span;
                fields.push(StructField {
                    is_var,
                    name: fname,
                    ty: fty,
                    span: f_start.merge(f_end),
                });
                if self.peek_kind() == &TokenKind::Comma
                    || self.peek_kind() == &TokenKind::Semicolon
                {
                    self.bump();
                } else {
                    self.expect_separated("struct fields", "a newline, ',' or ';'")?;
                }
            }
            self.expect(TokenKind::RBrace)?.span
        };

        Ok(StructDecl {
            visibility,
            name,
            type_params,
            marker,
            fields,
            span: start_span.merge(end_span),
        })
    }

    fn parse_enum(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<EnumDecl, ParseError> {
        self.expect(TokenKind::Enum)?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_optional_type_params()?;
        let marker = self.parse_optional_marker()?.map(|(parsed, _)| parsed);

        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();

        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            let (vname, vstart) = self.expect_ident()?;
            let mut vfields = Vec::new();
            let mut vend = vstart;

            if self.peek_kind() == &TokenKind::LParen {
                self.bump();
                while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
                    let payload = self.parse_type()?;
                    reject_stored_borrow(&payload, "Enum variant payloads")?;
                    vfields.push(payload);
                    if self.peek_kind() == &TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
                vend = self.expect(TokenKind::RParen)?.span;
            }

            variants.push(EnumVariant {
                name: vname,
                fields: vfields,
                span: vstart.merge(vend),
            });

            if self.peek_kind() == &TokenKind::Comma || self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            } else {
                self.expect_separated("enum variants", "a newline, ',' or ';'")?;
            }
        }

        let end_span = self.expect(TokenKind::RBrace)?.span;

        Ok(EnumDecl {
            visibility,
            name,
            type_params,
            marker,
            variants,
            span: start_span.merge(end_span),
        })
    }

    fn parse_top_level_var(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<TopLevelVarDecl, ParseError> {
        self.expect(TokenKind::Val)?;

        let (name, _) = self.expect_ident()?;
        let mut ty = None;
        if self.peek_kind() == &TokenKind::Colon {
            self.bump();
            ty = Some(self.parse_type()?);
        }

        self.expect(TokenKind::Eq)?;
        let initializer = self.parse_initializer()?;
        let end_span = initializer.span;

        Ok(TopLevelVarDecl {
            visibility,
            name,
            ty,
            initializer,
            span: start_span.merge(end_span),
        })
    }

    // --- Nesting Depth ---

    /// Enters one nesting level, failing with `E2002` past
    /// [`MAX_NESTING_DEPTH`]. Returns the outer peak for [`Self::exit`].
    fn enter(&mut self) -> Result<usize, ParseError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(ParseError::nesting_too_deep(
                MAX_NESTING_DEPTH,
                self.peek().span,
            ));
        }
        self.depth += 1;
        Ok(std::mem::replace(&mut self.peak, self.depth))
    }

    /// Leaves the level opened by [`Self::enter`]; `self.peak` keeps covering
    /// the tree just parsed.
    fn exit(&mut self, outer_peak: usize) {
        self.peak = self.peak.max(outer_peak);
        self.depth -= 1;
    }

    /// Runs `f` one nesting level deeper, for constructs parsed outside
    /// `parse_expr` (statement-position blocks and declarations, braced branches).
    fn nested<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<T, ParseError> {
        let outer_peak = self.enter()?;
        let result = f(self);
        self.exit(outer_peak);
        result
    }

    // --- Type Parsing ---

    pub fn parse_type(&mut self) -> Result<Type, ParseError> {
        let outer_peak = self.enter()?;
        let result = self.parse_type_inner();
        self.exit(outer_peak);
        result
    }

    fn parse_type_inner(&mut self) -> Result<Type, ParseError> {
        let start_span = self.peek().span;

        // 1. Borrow reference: &T or &mut T
        if matches!(self.peek_kind(), TokenKind::Amp | TokenKind::AmpAmp) {
            let start_span = self.bump_amp().span;
            let is_mut = if self.peek_kind() == &TokenKind::Mut {
                self.bump();
                true
            } else {
                false
            };
            let inner = self.parse_type()?;
            let span = start_span.merge(inner.span);
            return Ok(if is_mut {
                Type::reference_mut(inner, span)
            } else {
                Type::reference(inner, span)
            });
        }

        // 2. Composable function type: compose (Int) -> Unit
        if self.peek_kind() == &TokenKind::Compose && self.peek_ahead(1) == &TokenKind::LParen {
            self.bump(); // 'compose'
            return self.parse_paren_type(start_span, true);
        }

        // 3. Unit, parenthesized type or function type: (), (Int), (Int, String) -> Bool
        if self.peek_kind() == &TokenKind::LParen {
            return self.parse_paren_type(start_span, false);
        }

        // 4. Named type: Ident optionally followed by generic arguments <T, U>
        let (name, span) = self.expect_ident()?;
        let mut type_args = Vec::new();
        let mut end_span = span;

        if self.peek_kind() == &TokenKind::Lt {
            self.bump(); // '<'
            while !self.at_closing_angle() && !self.is_eof() {
                type_args.push(self.parse_type()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            end_span = self.expect_closing_angle()?.span;
        }

        let base_ty = if type_args.is_empty() {
            Type::named(name, start_span.merge(end_span))
        } else {
            Type::generic(name, type_args, start_span.merge(end_span))
        };

        // 5. Suffix (e.g. nullable ?)
        self.parse_type_suffix(base_ty)
    }

    /// A type starting with `(`: unit `()`, a parenthesized type `(Int)`, or a
    /// function type `(A, B) -> R`, which is the only form allowed after `compose`.
    fn parse_paren_type(&mut self, start_span: Span, is_compose: bool) -> Result<Type, ParseError> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            params.push(self.parse_type()?);
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        let rparen_span = self.expect(TokenKind::RParen)?.span;

        if self.peek_kind() == &TokenKind::Arrow {
            self.bump();
            // Right-associative: `(Int) -> (Int) -> Int` returns a function.
            let ret = self.parse_type()?;
            let span = start_span.merge(ret.span);
            return Ok(Type::new(
                TypeKind::Function {
                    is_compose,
                    params,
                    return_type: Box::new(ret),
                },
                span,
            ));
        }
        if is_compose {
            return Err(ParseError::new(
                format!(
                    "Expected '->' after the parameter types of a 'compose' function type, found '{}'",
                    self.peek_kind()
                ),
                self.peek().span,
            ));
        }
        match params.len() {
            0 => self.parse_type_suffix(Type::unit(start_span.merge(rparen_span))),
            // Single parenthesized type: (Int) is just Int
            1 => self.parse_type_suffix(params.remove(0)),
            _ => Err(ParseError::new(
                "Tuple types are not supported; use a struct",
                start_span,
            )),
        }
    }

    fn parse_type_suffix(&mut self, base: Type) -> Result<Type, ParseError> {
        if self.peek_kind() == &TokenKind::Question {
            let q_span = self.bump().span;
            let full_span = base.span.merge(q_span);
            Ok(Type::nullable(base, full_span))
        } else {
            Ok(base)
        }
    }

    /// Parses an optional `: Copy` or `: Clone` marker after a struct or enum's name, type
    /// parameters or shorthand parameter list. Exactly one marker name may follow the colon.
    fn parse_optional_marker(&mut self) -> Result<Option<(TypeMarker, Span)>, ParseError> {
        const ONE_MARKER: &str = "Exactly one of 'Copy' or 'Clone' may follow ':' in a struct or enum declaration ('Copy' already implies 'Clone')";
        if self.peek_kind() != &TokenKind::Colon {
            return Ok(None);
        }
        self.bump();
        let tok = self.peek().clone();
        let marker = match &tok.kind {
            TokenKind::Ident(name) if name == "Copy" => TypeMarker::Copy,
            TokenKind::Ident(name) if name == "Clone" => TypeMarker::Clone,
            _ => return Err(ParseError::new(ONE_MARKER, tok.span)),
        };
        self.bump();
        if self.peek_kind() == &TokenKind::Comma {
            return Err(ParseError::new(ONE_MARKER, self.peek().span));
        }
        Ok(Some((marker, tok.span)))
    }

    fn parse_optional_type_params(&mut self) -> Result<Vec<String>, ParseError> {
        let mut params = Vec::new();
        if self.peek_kind() == &TokenKind::Lt {
            self.bump();
            while !self.at_closing_angle() && !self.is_eof() {
                let (p, _) = self.expect_ident()?;
                params.push(p);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect_closing_angle()?;
        }
        Ok(params)
    }

    // --- Expression Pratt Parser ---

    pub fn parse_expr(&mut self, precedence: Precedence) -> Result<Expr, ParseError> {
        let outer_peak = self.enter()?;
        let result = self.parse_expr_inner(precedence);
        self.exit(outer_peak);
        result
    }

    fn parse_expr_inner(&mut self, precedence: Precedence) -> Result<Expr, ParseError> {
        let mut left = self.parse_prefix()?;
        // The infix loop is iterative but each step wraps `left` in a new
        // node, so track the tree's height rather than the recursion depth.
        let mut height = self.peak - self.depth;

        while !self.is_eof() && precedence < self.current_precedence() {
            let op_span = self.peek().span;
            self.peak = self.depth;
            left = self.parse_infix(left)?;
            height = (height + 1).max(self.peak - self.depth);
            if self.depth + height > MAX_NESTING_DEPTH {
                return Err(ParseError::nesting_too_deep(MAX_NESTING_DEPTH, op_span));
            }
        }

        self.peak = self.depth + height;
        Ok(left)
    }

    fn current_precedence(&self) -> Precedence {
        let tok = self.peek();
        // A line break ends the expression, except before the operators that
        // conventionally start a continuation line (Kotlin's rule).
        if self.at_line_break()
            && !matches!(
                tok.kind,
                TokenKind::Dot
                    | TokenKind::QuestionDot
                    | TokenKind::AmpAmp
                    | TokenKind::PipePipe
                    | TokenKind::QuestionColon
            )
        {
            return Precedence::Lowest;
        }
        match &tok.kind {
            TokenKind::Eq => Precedence::Assign,
            TokenKind::PipePipe => Precedence::LogicalOr,
            TokenKind::AmpAmp => Precedence::LogicalAnd,
            TokenKind::EqEq | TokenKind::BangEq => Precedence::Equality,
            TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq => {
                Precedence::Comparison
            }
            TokenKind::QuestionColon => Precedence::Elvis,
            TokenKind::DotDot | TokenKind::DotDotLess => Precedence::Range,
            TokenKind::Plus | TokenKind::Minus => Precedence::Sum,
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Precedence::Product,
            TokenKind::Dot | TokenKind::QuestionDot => Precedence::Postfix,
            TokenKind::LBracket | TokenKind::LParen | TokenKind::LBrace => Precedence::Call,
            _ => Precedence::Lowest,
        }
    }

    // Every recursion level of the parser passes through `parse_prefix` or
    // `parse_infix`, so they only dispatch: each construct gets its own method
    // and its temporaries stay out of the recursive frames.
    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::String(_)
            | TokenKind::Char(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::Ident(_)
            | TokenKind::Compose => self.parse_atom(),

            // Unary operators
            TokenKind::Minus | TokenKind::Bang | TokenKind::Amp | TokenKind::AmpAmp => {
                self.parse_unary()
            }

            // `*` only writes through a reference, as an assignment target, which
            // `parse_assignment_or_expression` parses before reaching here.
            TokenKind::Star => Err(ParseError::new(DEREF_OUTSIDE_TARGET, self.peek().span)),

            // Grouping: (expr)
            TokenKind::LParen => self.parse_group(),

            // In expression position a brace is always a closure; blocks are
            // parsed only in block position (bodies, branches, statements).
            TokenKind::LBrace => self.parse_closure(),

            // Control flow
            TokenKind::If => self.parse_if(),
            TokenKind::When => self.parse_when(),
            TokenKind::Return => self.parse_return(),
            TokenKind::Break | TokenKind::Continue => Ok(self.parse_jump()),

            TokenKind::While | TokenKind::Do | TokenKind::For => Err(ParseError::new(
                format!(
                    "A '{}' loop is a statement and cannot be used as a value",
                    self.peek_kind()
                ),
                self.peek().span,
            )),

            TokenKind::Val | TokenKind::Var => Err(ParseError::new(
                format!(
                    "A '{}' declaration is only allowed as a statement; wrap it in braces",
                    self.peek_kind()
                ),
                self.peek().span,
            )),

            _ => Err(ParseError::new(
                format!("Unexpected token in expression: '{}'", self.peek_kind()),
                self.peek().span,
            )),
        }
    }

    /// Literals and identifiers.
    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        let tok = self.bump();
        let kind = match tok.kind {
            TokenKind::Int(v) => match i64::try_from(v) {
                Ok(v) => ExprKind::Literal(Literal::Int(v)),
                Err(_) => {
                    return Err(ParseError::new(
                        format!("Integer literal out of range: {}", v),
                        tok.span,
                    ))
                }
            },
            TokenKind::Float(v) => ExprKind::Literal(Literal::Float(v)),
            TokenKind::String(s) => ExprKind::Literal(Literal::String(s)),
            TokenKind::Char(c) => ExprKind::Literal(Literal::Char(c)),
            TokenKind::True => ExprKind::Literal(Literal::Bool(true)),
            TokenKind::False => ExprKind::Literal(Literal::Bool(false)),
            TokenKind::Null => ExprKind::Literal(Literal::Null),
            TokenKind::Ident(name) => ExprKind::Ident(name),
            TokenKind::Compose => ExprKind::Ident("compose".to_string()),
            _ => unreachable!("parse_prefix only dispatches atoms here"),
        };
        Ok(Expr::new(kind, tok.span))
    }

    /// Prefix operators: `-`, `!`, `&` and `&mut`.
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if let Some(min) = self.try_parse_int_min() {
            return Ok(min);
        }
        let tok = self.bump_amp();
        let op = match tok.kind {
            TokenKind::Minus => UnaryOp::Neg,
            TokenKind::Bang => UnaryOp::Not,
            TokenKind::Amp if self.peek_kind() == &TokenKind::Mut => {
                self.bump();
                UnaryOp::BorrowMut
            }
            TokenKind::Amp => UnaryOp::Borrow,
            _ => unreachable!("parse_prefix only dispatches unary operators here"),
        };
        let inner = self.parse_expr(Precedence::Prefix)?;
        let span = tok.span.merge(inner.span);
        Ok(Expr::new(
            ExprKind::Unary {
                op,
                expr: Box::new(inner),
            },
            span,
        ))
    }

    /// Reads `-9223372036854775808` as the literal `i64::MIN`, whose magnitude
    /// does not fit an `Int` on its own. Only when nothing postfix follows the
    /// literal, since `-x.f()` negates `x.f()`.
    fn try_parse_int_min(&mut self) -> Option<Expr> {
        if self.peek_kind() != &TokenKind::Minus
            || self.peek_ahead(1) != &TokenKind::Int(i64::MIN.unsigned_abs())
        {
            return None;
        }
        let start = self.cursor;
        let minus = self.bump();
        let literal = self.bump();
        if self.current_precedence() >= Precedence::Postfix {
            self.cursor = start;
            return None;
        }
        Some(Expr::new(
            ExprKind::Literal(Literal::Int(i64::MIN)),
            minus.span.merge(literal.span),
        ))
    }

    fn parse_group(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::LParen)?.span;
        let expr = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
        let end = self.expect(TokenKind::RParen)?.span;
        Ok(Expr::new(expr.kind, start.merge(end)))
    }

    /// Whether `kind` can begin an expression, i.e. `parse_prefix` accepts it.
    fn starts_expr(kind: &TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::Char(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Ident(_)
                | TokenKind::Compose
                | TokenKind::Minus
                | TokenKind::Bang
                | TokenKind::Amp
                | TokenKind::AmpAmp
                | TokenKind::Star
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::If
                | TokenKind::When
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
        )
    }

    fn is_closure_with_arrow_ahead(&self) -> bool {
        let mut idx = self.cursor + 1;
        let mut has_arrow = false;
        while idx < self.tokens.len() {
            match &self.tokens[idx].kind {
                TokenKind::Arrow => {
                    has_arrow = true;
                    break;
                }
                TokenKind::Ident(_) | TokenKind::Compose => {
                    idx += 1;
                    if idx < self.tokens.len() && self.tokens[idx].kind == TokenKind::Comma {
                        idx += 1;
                    }
                }
                _ => return false,
            }
        }
        has_arrow
    }

    /// A closure `{ a, b -> ... }`, or `{ ... }` with zero parameters.
    fn parse_closure(&mut self) -> Result<Expr, ParseError> {
        let has_header = self.is_closure_with_arrow_ahead();
        let start = self.expect(TokenKind::LBrace)?.span;
        let mut params = Vec::new();
        let mut arrow = None;

        if has_header {
            while self.peek_kind() != &TokenKind::Arrow && !self.is_eof() {
                let (param, _) = self.expect_ident()?;
                params.push(param);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            arrow = Some(self.expect(TokenKind::Arrow)?.span);
        }

        let (statements, tail) = self.with_newlines(true, Self::parse_block_contents)?;
        let end = self.expect(TokenKind::RBrace)?.span;

        // The body block runs from `->` (or its first item) to `}`, so it never
        // shares a span with the closure or with its own first item.
        let first_item = statements
            .first()
            .map(|s| s.span)
            .or(tail.as_ref().map(|t| t.span));
        let body_start = arrow.or(first_item).unwrap_or(end);
        Ok(Expr::new(
            ExprKind::Closure {
                params,
                body: Block {
                    statements,
                    tail,
                    span: body_start.merge(end),
                },
            },
            start.merge(end),
        ))
    }

    fn parse_infix(&mut self, left: Expr) -> Result<Expr, ParseError> {
        match self.peek_kind() {
            // Member access: .ident, safe navigation: ?.ident
            TokenKind::Dot | TokenKind::QuestionDot => self.parse_member_access(left),
            // Function Call: (args...)
            TokenKind::LParen => self.parse_call(left),
            // Trailing lambda call without parentheses: callee { body }
            TokenKind::LBrace => self.parse_trailing_lambda_call(left),
            // Indexing: target[index]
            TokenKind::LBracket => self.parse_index(left),
            // Assignments are statements, parsed by `parse_statement_or_expression`.
            TokenKind::Eq => Err(ParseError::new(
                "An assignment is a statement, not an expression; it cannot be used as a value",
                self.peek().span,
            )),
            // Binary operators
            _ => self.parse_binary(left),
        }
    }

    fn parse_member_access(&mut self, left: Expr) -> Result<Expr, ParseError> {
        let is_safe_nav = self.bump().kind == TokenKind::QuestionDot;
        let (member, m_span) = self.expect_ident()?;
        let span = left.span.merge(m_span);
        Ok(Expr::new(
            ExprKind::MemberAccess {
                target: Box::new(left),
                member,
                is_safe_nav,
            },
            span,
        ))
    }

    fn parse_call(&mut self, left: Expr) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LParen)?;
        let mut args = self.with_newlines(false, Self::parse_call_args)?;
        let end = self.expect(TokenKind::RParen)?.span;
        let mut span = left.span.merge(end);

        // Trailing lambda on call with parentheses: callee(args...) { body }
        if self.peek_kind() == &TokenKind::LBrace && !self.at_line_break() {
            let lambda = self.parse_closure()?;
            span = span.merge(lambda.span);
            args.push(CallArg {
                name: None,
                span: lambda.span,
                value: lambda,
            });
        }

        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(left),
                args,
            },
            span,
        ))
    }

    /// Call arguments up to (not including) the closing `)`.
    fn parse_call_args(&mut self) -> Result<Vec<CallArg>, ParseError> {
        let mut args = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            let mut name = None;
            let mut arg_start = self.peek().span;
            // Check named argument: ident = expr (`compose` is a contextual keyword)
            if matches!(self.peek_kind(), TokenKind::Ident(_) | TokenKind::Compose)
                && self.peek_ahead(1) == &TokenKind::Eq
            {
                let (n, n_span) = self.expect_ident()?;
                self.bump(); // '='
                name = Some(n);
                arg_start = n_span;
            }
            let value = self.parse_expr(Precedence::Lowest)?;
            let arg_span = arg_start.merge(value.span);
            args.push(CallArg {
                name,
                value,
                span: arg_span,
            });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(args)
    }

    fn parse_trailing_lambda_call(&mut self, left: Expr) -> Result<Expr, ParseError> {
        let lambda = self.parse_closure()?;
        let span = left.span.merge(lambda.span);
        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(left),
                args: vec![CallArg {
                    name: None,
                    span: lambda.span,
                    value: lambda,
                }],
            },
            span,
        ))
    }

    fn parse_index(&mut self, left: Expr) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LBracket)?;
        let index = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
        let end = self.expect(TokenKind::RBracket)?.span;
        let span = left.span.merge(end);
        Ok(Expr::new(
            ExprKind::Index {
                target: Box::new(left),
                index: Box::new(index),
            },
            span,
        ))
    }

    fn parse_binary(&mut self, left: Expr) -> Result<Expr, ParseError> {
        let op = match self.token_to_binary_op(self.peek_kind()) {
            Some(op) => op,
            None => {
                return Err(ParseError::new(
                    format!("Unexpected infix operator: '{}'", self.peek_kind()),
                    self.peek().span,
                ))
            }
        };
        let prec = self.current_precedence();
        self.bump();
        let right = self.parse_expr(prec)?;
        let span = left.span.merge(right.span);
        Ok(Expr::new(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        ))
    }

    fn token_to_binary_op(&self, kind: &TokenKind) -> Option<BinaryOp> {
        match kind {
            TokenKind::Plus => Some(BinaryOp::Add),
            TokenKind::Minus => Some(BinaryOp::Sub),
            TokenKind::Star => Some(BinaryOp::Mul),
            TokenKind::Slash => Some(BinaryOp::Div),
            TokenKind::Percent => Some(BinaryOp::Mod),
            TokenKind::EqEq => Some(BinaryOp::Eq),
            TokenKind::BangEq => Some(BinaryOp::NotEq),
            TokenKind::Lt => Some(BinaryOp::Lt),
            TokenKind::LtEq => Some(BinaryOp::LtEq),
            TokenKind::Gt => Some(BinaryOp::Gt),
            TokenKind::GtEq => Some(BinaryOp::GtEq),
            TokenKind::AmpAmp => Some(BinaryOp::And),
            TokenKind::PipePipe => Some(BinaryOp::Or),
            TokenKind::QuestionColon => Some(BinaryOp::Elvis),
            TokenKind::DotDot => Some(BinaryOp::Range),
            TokenKind::DotDotLess => Some(BinaryOp::HalfOpenRange),
            _ => None,
        }
    }

    fn parse_block_expr(&mut self) -> Result<Block, ParseError> {
        let start = self.expect(TokenKind::LBrace)?.span;
        let (statements, tail) = self.with_newlines(true, Self::parse_block_contents)?;
        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(Block {
            statements,
            tail,
            span: start.merge(end),
        })
    }

    /// The items of a block or closure body up to (not including) the closing
    /// `}`. Two items on one line must be separated by `;`. A final expression
    /// statement becomes the tail, whether or not a `;` follows it.
    fn parse_block_contents(&mut self) -> Result<(Vec<Statement>, Option<Box<Expr>>), ParseError> {
        let mut statements = Vec::new();
        loop {
            while self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            }
            if self.peek_kind() == &TokenKind::RBrace || self.is_eof() {
                break;
            }
            statements.push(self.parse_statement()?);
            if self.peek_kind() != &TokenKind::Semicolon {
                self.expect_separated("statements", "a newline or ';'")?;
            }
        }
        let tail = match statements.last().map(|s| &s.kind) {
            Some(StatementKind::Expression(_)) => match statements.pop().map(|s| s.kind) {
                Some(StatementKind::Expression(expr)) => Some(Box::new(expr)),
                _ => unreachable!("the last statement is an expression statement"),
            },
            _ => None,
        };
        Ok((statements, tail))
    }

    /// One statement: a declaration, a nested scope block, an assignment or an
    /// expression.
    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match self.peek_kind() {
            TokenKind::Val | TokenKind::Var => self.nested(Self::parse_local_var),
            TokenKind::While => self.nested(Self::parse_while),
            TokenKind::Do => self.nested(Self::parse_do_while),
            TokenKind::For => self.nested(Self::parse_for),
            // A brace at the start of a statement opens a scope, not a closure.
            TokenKind::LBrace => {
                let block = self.nested(Self::parse_block)?;
                let span = block.span;
                Ok(Statement::new(StatementKind::Expression(block), span))
            }
            _ => self.parse_assignment_or_expression(),
        }
    }

    /// An assignment `target = value`, or an expression statement. The target
    /// is parsed above assignment precedence so that `=` ends it; assignments
    /// are non-associative, so `a = b = c` is rejected.
    fn parse_assignment_or_expression(&mut self) -> Result<Statement, ParseError> {
        let is_write_through = self.peek_kind() == &TokenKind::Star;
        let start_span = self.peek().span;
        let target = if is_write_through {
            self.nested(Self::parse_deref_target)?
        } else {
            self.parse_expr(Precedence::Assign)?
        };
        if self.peek_kind() != &TokenKind::Eq || self.at_line_break() {
            if is_write_through {
                return Err(ParseError::new(DEREF_OUTSIDE_TARGET, start_span));
            }
            let span = target.span;
            return Ok(Statement::new(StatementKind::Expression(target), span));
        }
        self.bump(); // '='
        let value = self.parse_expr(Precedence::Assign)?;
        if self.peek_kind() == &TokenKind::Eq && !self.at_line_break() {
            return Err(ParseError::new(
                "Chained assignments are not allowed; use separate statements",
                self.peek().span,
            ));
        }
        let is_place = match &target.kind {
            ExprKind::Ident(_)
            | ExprKind::Index { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref, ..
            } => true,
            ExprKind::MemberAccess { is_safe_nav, .. } => !is_safe_nav,
            _ => false,
        };
        if !is_place {
            return Err(ParseError::new(
                "Invalid assignment target; expected a variable, field access or index expression",
                target.span,
            ));
        }
        let span = target.span.merge(value.span);
        Ok(Statement::new(
            StatementKind::Assign { target, value },
            span,
        ))
    }

    /// A write-through target `*e`, `**e`, ...: one `Deref` per `*` around an operand parsed
    /// at prefix precedence, so postfix operators bind tighter (`*r.x` is `*(r.x)`). Each `*`
    /// is one nesting level.
    fn parse_deref_target(&mut self) -> Result<Expr, ParseError> {
        let star = self.expect(TokenKind::Star)?.span;
        let inner = if self.peek_kind() == &TokenKind::Star {
            self.nested(Self::parse_deref_target)?
        } else {
            self.parse_expr(Precedence::Prefix)?
        };
        let span = star.merge(inner.span);
        Ok(Expr::new(
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: Box::new(inner),
            },
            span,
        ))
    }

    /// The body of an `if`/`else` or `when` branch: a block when braced,
    /// otherwise a single expression, or an assignment wrapped in a block with
    /// no tail.
    fn parse_branch_body(&mut self) -> Result<Expr, ParseError> {
        if self.peek_kind() == &TokenKind::LBrace {
            return self.nested(Self::parse_block);
        }
        let statement = self.parse_assignment_or_expression()?;
        Ok(Self::statement_as_branch(statement))
    }

    /// An unbraced branch body as an expression: an expression statement is the
    /// expression itself, and anything else becomes a block with no tail.
    fn statement_as_branch(statement: Statement) -> Expr {
        match statement.kind {
            StatementKind::Expression(expr) => expr,
            kind => {
                let span = statement.span;
                Expr::new(
                    ExprKind::Block(Block {
                        statements: vec![Statement::new(kind, span)],
                        tail: None,
                        span,
                    }),
                    span,
                )
            }
        }
    }

    /// A parenthesized loop condition: `(condition)`.
    fn parse_loop_condition(&mut self) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LParen)?;
        let condition = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
        self.expect(TokenKind::RParen)?;
        Ok(condition)
    }

    /// The body of a `while` or `for` loop: a block when braced, otherwise a
    /// single expression or assignment, as the block's only item.
    fn parse_loop_body(&mut self) -> Result<Block, ParseError> {
        if self.peek_kind() == &TokenKind::LBrace {
            return self.nested(Self::parse_block_expr);
        }
        let statement = self.parse_assignment_or_expression()?;
        let span = statement.span;
        Ok(match statement.kind {
            StatementKind::Expression(expr) => Block {
                statements: Vec::new(),
                tail: Some(Box::new(expr)),
                span,
            },
            kind => Block {
                statements: vec![Statement::new(kind, span)],
                tail: None,
                span,
            },
        })
    }

    /// `while (condition) body`.
    fn parse_while(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(TokenKind::While)?.span;
        let condition = self.parse_loop_condition()?;
        let body = self.parse_loop_body()?;
        let span = start.merge(body.span);
        Ok(Statement::new(
            StatementKind::While { condition, body },
            span,
        ))
    }

    /// `do { body } while (condition)`. The body must be braced.
    fn parse_do_while(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(TokenKind::Do)?.span;
        if self.peek_kind() != &TokenKind::LBrace {
            return Err(ParseError::new(
                format!(
                    "Expected '{{' after 'do'; a 'do' loop body must be braced, found '{}'",
                    self.peek_kind()
                ),
                self.peek().span,
            ));
        }
        let body = self.nested(Self::parse_block_expr)?;
        self.expect(TokenKind::While)?;
        let condition = self.parse_loop_condition()?;
        // The statement ends at the closing parenthesis, the last token consumed.
        let span = start.merge(self.tokens[self.cursor - 1].span);
        Ok(Statement::new(
            StatementKind::DoWhile { body, condition },
            span,
        ))
    }

    /// `for (variable in iterable) body`.
    fn parse_for(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(TokenKind::For)?.span;
        self.expect(TokenKind::LParen)?;
        let (variable, variable_span) = self.expect_ident()?;
        self.expect(TokenKind::In)?;
        let iterable = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
        self.expect(TokenKind::RParen)?;
        let body = self.parse_loop_body()?;
        let span = start.merge(body.span);
        Ok(Statement::new(
            StatementKind::For {
                variable,
                variable_span,
                iterable,
                body,
            },
            span,
        ))
    }

    /// `break` or `continue`. Neither takes an operand, so anything after it on
    /// the same line needs a separator.
    fn parse_jump(&mut self) -> Expr {
        let token = self.bump();
        let kind = match token.kind {
            TokenKind::Break => ExprKind::Break,
            TokenKind::Continue => ExprKind::Continue,
            _ => unreachable!("parse_prefix only dispatches jumps here"),
        };
        Expr::new(kind, token.span)
    }

    /// A block in block position, as an expression.
    fn parse_block(&mut self) -> Result<Expr, ParseError> {
        let block = self.parse_block_expr()?;
        let span = block.span;
        Ok(Expr::new(ExprKind::Block(block), span))
    }

    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::If)?.span;
        self.expect(TokenKind::LParen)?;
        let cond = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
        self.expect(TokenKind::RParen)?;
        let then_branch = self.parse_branch_body()?;

        let mut else_branch = None;
        let mut end_span = then_branch.span;

        if self.peek_kind() == &TokenKind::Else {
            self.bump();
            let els = self.parse_branch_body()?;
            end_span = els.span;
            else_branch = Some(Box::new(els));
        }

        Ok(Expr::new(
            ExprKind::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch,
            },
            start.merge(end_span),
        ))
    }

    fn parse_when(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::When)?.span;

        let target = if self.peek_kind() == &TokenKind::LParen {
            self.bump();
            let t = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
            self.expect(TokenKind::RParen)?;
            Some(Box::new(t))
        } else if self.peek_kind() == &TokenKind::LBrace {
            None
        } else {
            return Err(ParseError::new(
                format!(
                    "Expected '(' or '{{' after 'when', found '{}'",
                    self.peek_kind()
                ),
                self.peek().span,
            ));
        };

        self.expect(TokenKind::LBrace)?;
        let branches = self.with_newlines(true, Self::parse_when_branches)?;
        let end = self.expect(TokenKind::RBrace)?.span;

        Ok(Expr::new(
            ExprKind::When { target, branches },
            start.merge(end),
        ))
    }

    /// `when` branches up to (not including) the closing `}`.
    fn parse_when_branches(&mut self) -> Result<Vec<WhenBranch>, ParseError> {
        let mut branches = Vec::new();
        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            branches.push(self.parse_when_branch()?);

            if self.peek_kind() == &TokenKind::Comma || self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            } else {
                self.expect_separated("'when' branches", "a newline, ',' or ';'")?;
            }
        }
        Ok(branches)
    }

    fn parse_when_branch(&mut self) -> Result<WhenBranch, ParseError> {
        let mut patterns = Vec::new();
        let b_start = self.peek().span;

        if self.peek_kind() == &TokenKind::Else {
            self.bump();
            patterns.push(WhenPattern::Else);
        } else {
            while self.peek_kind() != &TokenKind::Arrow && !self.is_eof() {
                patterns.push(self.parse_when_pattern()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            if patterns.is_empty() {
                return Err(ParseError::new(
                    "Expected a pattern or 'else' before '->' in 'when' branch",
                    b_start,
                ));
            }
        }

        self.expect(TokenKind::Arrow)?;
        let body = self.parse_branch_body()?;
        let b_end = body.span;

        Ok(WhenBranch {
            patterns,
            body,
            span: b_start.merge(b_end),
        })
    }

    fn parse_when_pattern(&mut self) -> Result<WhenPattern, ParseError> {
        if self.peek_kind() != &TokenKind::In {
            return Ok(WhenPattern::Expr(self.parse_expr(Precedence::Lowest)?));
        }
        self.bump(); // 'in'
        let range_expr = self.parse_expr(Precedence::Lowest)?;
        match range_expr.kind {
            ExprKind::Binary {
                op: op @ (BinaryOp::Range | BinaryOp::HalfOpenRange),
                left,
                right,
            } => Ok(WhenPattern::InRange {
                start: *left,
                end: *right,
                end_inclusive: op == BinaryOp::Range,
            }),
            _ => Err(ParseError::new(
                "Expected range expression 'a..b' or 'a..<b' after 'in'",
                range_expr.span,
            )),
        }
    }

    fn parse_return(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::Return)?.span;
        // A value must start on the same line; anything else (`}`, `else`,
        // `,`, `)`, a newline...) ends a bare `return`.
        let has_value = !self.at_line_break() && Self::starts_expr(self.peek_kind());

        if has_value {
            let val = self.parse_expr(Precedence::Lowest)?;
            let span = start.merge(val.span);
            Ok(Expr::new(ExprKind::Return(Some(Box::new(val))), span))
        } else {
            Ok(Expr::new(ExprKind::Return(None), start))
        }
    }

    fn parse_local_var(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        let is_var = match self.peek_kind() {
            TokenKind::Var => {
                self.bump();
                true
            }
            TokenKind::Val => {
                self.bump();
                false
            }
            _ => unreachable!(),
        };

        let (name, _) = self.expect_ident()?;
        let mut ty = None;
        if self.peek_kind() == &TokenKind::Colon {
            self.bump();
            ty = Some(self.parse_type()?);
        }

        self.expect(TokenKind::Eq)?;
        let initializer = self.parse_initializer()?;
        let span = start.merge(initializer.span);

        Ok(Statement::new(
            StatementKind::Local {
                is_var,
                name,
                ty,
                initializer,
            },
            span,
        ))
    }

    /// A declaration's initializer. It is parsed above assignment precedence, so
    /// `val a = b = c` is a chained assignment rather than `val a = (b = c)`.
    fn parse_initializer(&mut self) -> Result<Expr, ParseError> {
        let initializer = self.parse_expr(Precedence::Assign)?;
        if self.peek_kind() == &TokenKind::Eq && !self.at_line_break() {
            return Err(ParseError::new(
                "Chained assignments are not allowed; use separate statements",
                self.peek().span,
            ));
        }
        Ok(initializer)
    }
}

/// Rejects a struct field or enum payload type that stores a borrow, so no value can outlive what
/// it borrows (the zero-cost memory-safety invariant). A borrow nested in `?` or a type argument
/// (`(&String)?`, `Heap<&Int>`) is stored too. One inside a function type (`(&User) -> Unit`) is
/// not: it exists only for the duration of a call.
fn reject_stored_borrow(ty: &Type, members: &str) -> Result<(), ParseError> {
    match stored_borrow(ty) {
        Some(borrow) => Err(ParseError::new(
            format!(
                "{members} cannot have borrowed reference types ('&T' or '&mut T'), even inside '?' or a type argument; use Heap<T> or own by value (zero-cost memory-safety invariant)"
            ),
            borrow.span,
        )),
        None => Ok(()),
    }
}

/// The first borrow that a value of type `ty` would store, outside any function type.
fn stored_borrow(ty: &Type) -> Option<&Type> {
    match &ty.kind {
        TypeKind::Reference { .. } => Some(ty),
        TypeKind::Nullable(inner) => stored_borrow(inner),
        TypeKind::Named { type_args, .. } => type_args.iter().find_map(stored_borrow),
        TypeKind::Unit | TypeKind::Function { .. } => None,
    }
}
