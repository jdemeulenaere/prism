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

pub struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    /// Number of `parse_expr` / `parse_type` calls currently on the stack.
    depth: usize,
    /// Deepest tree level reached since the innermost open `parse_expr` or
    /// `parse_type` started; `peak - depth` is the height of what it parsed.
    peak: usize,
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

    // --- Source File Parsing ---

    pub fn parse_source_file(&mut self) -> Result<SourceFile, ParseError> {
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
        // package fun / package struct / package val / package var is a package visibility modifier!
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

        let (first, mut end_span) = self.expect_ident()?;
        path.push(first);

        // Either slash-separated (/core/fmt) or dot-separated (doo.util)
        while self.peek_kind() == &TokenKind::Slash || self.peek_kind() == &TokenKind::Dot {
            self.bump();
            let (part, s) = self.expect_ident()?;
            path.push(part);
            end_span = s;
        }

        Ok(ImportDecl {
            is_root,
            path,
            span: start_span.merge(end_span),
        })
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
            TokenKind::Val | TokenKind::Var => {
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
                    "Expected declaration (fun, struct, enum, val, var), found '{}'",
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
        self.expect(TokenKind::RParen)?;

        // Return type: : ReturnType
        let mut return_type = None;
        if self.peek_kind() == &TokenKind::Colon {
            self.bump();
            return_type = Some(self.parse_type()?);
        }

        // Body: = expr OR { ... } OR none (forward declaration / interface)
        let mut body = None;
        let mut end_span = self.peek().span;

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

    fn parse_struct(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<StructDecl, ParseError> {
        self.expect(TokenKind::Struct)?;
        let (name, _) = self.expect_ident()?;
        let type_params = self.parse_optional_type_params()?;

        let mut fields = Vec::new();

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
                if matches!(fty.kind, TypeKind::Reference { .. }) {
                    return Err(ParseError::new(
                        "Struct fields cannot have borrowed reference types ('&T' or '&mut T'); use Heap<T> or own by value (zero-cost memory invariant)",
                        fty.span,
                    ));
                }
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
            self.expect(TokenKind::RParen)?.span
        } else {
            // Block struct: struct Point { val x: Double, val y: Double }
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
                if matches!(fty.kind, TypeKind::Reference { .. }) {
                    return Err(ParseError::new(
                        "Struct fields cannot have borrowed reference types ('&T' or '&mut T'); use Heap<T> or own by value (zero-cost memory invariant)",
                        fty.span,
                    ));
                }
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
                }
            }
            self.expect(TokenKind::RBrace)?.span
        };

        Ok(StructDecl {
            visibility,
            name,
            type_params,
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

        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();

        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            let (vname, vstart) = self.expect_ident()?;
            let mut vfields = Vec::new();
            let mut vend = vstart;

            if self.peek_kind() == &TokenKind::LParen {
                self.bump();
                while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
                    vfields.push(self.parse_type()?);
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
            }
        }

        let end_span = self.expect(TokenKind::RBrace)?.span;

        Ok(EnumDecl {
            visibility,
            name,
            type_params,
            variants,
            span: start_span.merge(end_span),
        })
    }

    fn parse_top_level_var(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<TopLevelVarDecl, ParseError> {
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
        let initializer = self.parse_expr(Precedence::Lowest)?;
        let end_span = initializer.span;

        Ok(TopLevelVarDecl {
            visibility,
            is_var,
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
        if self.peek_kind() == &TokenKind::Amp {
            self.bump();
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

        // 2. Unit or Tuple/Function: ()
        if self.peek_kind() == &TokenKind::LParen {
            self.bump();
            if self.peek_kind() == &TokenKind::RParen {
                let end = self.bump().span;
                let unit = Type::unit(start_span.merge(end));
                return self.parse_type_suffix(unit);
            }
            // Parameter types for function type: (Int, String) -> Bool
            let mut params = Vec::new();
            while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
                params.push(self.parse_type()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::RParen)?;
            if self.peek_kind() == &TokenKind::Arrow {
                self.bump();
                let ret = self.parse_type()?;
                let span = start_span.merge(ret.span);
                let fn_ty = Type::new(
                    TypeKind::Function {
                        params,
                        return_type: Box::new(ret),
                    },
                    span,
                );
                return self.parse_type_suffix(fn_ty);
            } else if params.len() == 1 {
                // Single parenthesized type: (Int) -> just Int
                return self.parse_type_suffix(params.remove(0));
            } else {
                return Err(ParseError::new(
                    "Tuple types are not supported; use a struct",
                    start_span,
                ));
            }
        }

        // 3. Named type: Ident optionally followed by generic arguments <T, U>
        let (name, span) = self.expect_ident()?;
        let mut type_args = Vec::new();
        let mut end_span = span;

        if self.peek_kind() == &TokenKind::Lt {
            self.bump(); // '<'
            while self.peek_kind() != &TokenKind::Gt && !self.is_eof() {
                type_args.push(self.parse_type()?);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            end_span = self.expect(TokenKind::Gt)?.span;
        }

        let base_ty = if type_args.is_empty() {
            Type::named(name, start_span.merge(end_span))
        } else {
            Type::generic(name, type_args, start_span.merge(end_span))
        };

        // 4. Suffix (e.g. nullable ?)
        self.parse_type_suffix(base_ty)
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

    fn parse_optional_type_params(&mut self) -> Result<Vec<String>, ParseError> {
        let mut params = Vec::new();
        if self.peek_kind() == &TokenKind::Lt {
            self.bump();
            while self.peek_kind() != &TokenKind::Gt && !self.is_eof() {
                let (p, _) = self.expect_ident()?;
                params.push(p);
                if self.peek_kind() == &TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
            self.expect(TokenKind::Gt)?;
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
        match &tok.kind {
            TokenKind::Eq => Precedence::Assign,
            TokenKind::PipePipe => Precedence::LogicalOr,
            TokenKind::AmpAmp => Precedence::LogicalAnd,
            TokenKind::EqEq | TokenKind::BangEq => Precedence::Equality,
            TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq => {
                Precedence::Comparison
            }
            TokenKind::QuestionColon => Precedence::Elvis,
            TokenKind::DotDot => Precedence::Range,
            TokenKind::Plus | TokenKind::Minus => Precedence::Sum,
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Precedence::Product,
            TokenKind::Dot | TokenKind::QuestionDot => Precedence::Postfix,
            TokenKind::LBracket if !tok.is_preceded_by_newline => Precedence::Call,
            TokenKind::LParen if !tok.is_preceded_by_newline => Precedence::Call,
            TokenKind::LBrace if !tok.is_preceded_by_newline => Precedence::Call,
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
            TokenKind::Minus | TokenKind::Bang | TokenKind::Amp => self.parse_unary(),

            // Grouping: (expr)
            TokenKind::LParen => self.parse_group(),

            // Block or Closure: { ... }
            TokenKind::LBrace => self.parse_lambda_or_block(),

            // Control flow
            TokenKind::If => self.parse_if(),
            TokenKind::When => self.parse_when(),
            TokenKind::Return => self.parse_return(),

            // Local variable declaration: val x = 1, var y: Int = 2
            TokenKind::Val | TokenKind::Var => self.parse_local_var(),

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
            TokenKind::Int(v) => ExprKind::Literal(Literal::Int(v)),
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
        let tok = self.bump();
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

    fn parse_group(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::LParen)?.span;
        let expr = self.parse_expr(Precedence::Lowest)?;
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
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::If
                | TokenKind::When
                | TokenKind::Return
                | TokenKind::Val
                | TokenKind::Var
        )
    }

    fn parse_lambda_or_block(&mut self) -> Result<Expr, ParseError> {
        if self.is_closure_with_arrow_ahead() {
            self.parse_closure()
        } else {
            let block = self.parse_block_expr()?;
            let span = block.span;
            Ok(Expr::new(ExprKind::Block(block), span))
        }
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
                TokenKind::Ident(_) => {
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

    fn parse_closure(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::LBrace)?.span;
        let mut params = Vec::new();

        while self.peek_kind() != &TokenKind::Arrow && !self.is_eof() {
            let (param, _) = self.expect_ident()?;
            params.push(param);
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokenKind::Arrow)?;

        let mut stmts = self.parse_stmts()?;
        let end = self.expect(TokenKind::RBrace)?.span;

        let body = if stmts.len() == 1 && !matches!(stmts[0].kind, ExprKind::LocalVar { .. }) {
            stmts.remove(0)
        } else {
            let b_span = if stmts.is_empty() {
                start.merge(end)
            } else {
                stmts
                    .first()
                    .unwrap()
                    .span
                    .merge(stmts.last().unwrap().span)
            };
            Expr::new(
                ExprKind::Block(BlockExpr {
                    stmts,
                    span: b_span,
                }),
                b_span,
            )
        };

        Ok(Expr::new(
            ExprKind::Closure {
                params,
                body: Box::new(body),
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
            // Assignment: = (non-associative, disallowing chained assignments)
            TokenKind::Eq => self.parse_assign(left),
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
        let mut args = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            let mut name = None;
            // Check named argument: ident = expr
            if let TokenKind::Ident(n) = self.peek_kind() {
                if self.peek_ahead(1) == &TokenKind::Eq {
                    let n = n.clone();
                    self.bump(); // ident
                    self.bump(); // '='
                    name = Some(n);
                }
            }
            let value = self.parse_expr(Precedence::Lowest)?;
            let arg_span = value.span;
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
        let end = self.expect(TokenKind::RParen)?.span;
        let mut span = left.span.merge(end);

        // Trailing lambda on call with parentheses: callee(args...) { body }
        if self.peek_kind() == &TokenKind::LBrace && !self.peek().is_preceded_by_newline {
            let lambda = self.parse_lambda_or_block()?;
            span = span.merge(lambda.span);
            args.push(CallArg {
                name: None,
                value: lambda,
                span,
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

    fn parse_trailing_lambda_call(&mut self, left: Expr) -> Result<Expr, ParseError> {
        let lambda = self.parse_lambda_or_block()?;
        let span = left.span.merge(lambda.span);
        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(left),
                args: vec![CallArg {
                    name: None,
                    value: lambda,
                    span,
                }],
            },
            span,
        ))
    }

    fn parse_index(&mut self, left: Expr) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LBracket)?;
        let index = self.parse_expr(Precedence::Lowest)?;
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

    fn parse_assign(&mut self, left: Expr) -> Result<Expr, ParseError> {
        let eq_tok = self.bump();
        let right = self.parse_expr(Precedence::Assign)?;
        if matches!(right.kind, ExprKind::Assign { .. })
            || matches!(left.kind, ExprKind::Assign { .. })
        {
            return Err(ParseError::new(
                "Chained assignments are not allowed; use separate statements",
                eq_tok.span,
            ));
        }
        let span = left.span.merge(right.span);
        Ok(Expr::new(
            ExprKind::Assign {
                target: Box::new(left),
                value: Box::new(right),
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
            _ => None,
        }
    }

    fn parse_block_expr(&mut self) -> Result<BlockExpr, ParseError> {
        let start = self.expect(TokenKind::LBrace)?.span;
        let stmts = self.parse_stmts()?;
        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(BlockExpr {
            stmts,
            span: start.merge(end),
        })
    }

    /// Statements up to (not including) the closing `}` of a block or closure.
    fn parse_stmts(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut stmts = Vec::new();
        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            stmts.push(self.parse_expr(Precedence::Lowest)?);
            if self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            }
        }
        Ok(stmts)
    }

    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::If)?.span;
        self.expect(TokenKind::LParen)?;
        let cond = self.parse_expr(Precedence::Lowest)?;
        self.expect(TokenKind::RParen)?;
        let then_branch = self.parse_expr(Precedence::Lowest)?;

        let mut else_branch = None;
        let mut end_span = then_branch.span;

        if self.peek_kind() == &TokenKind::Else {
            self.bump();
            let els = self.parse_expr(Precedence::Lowest)?;
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
            let t = self.parse_expr(Precedence::Lowest)?;
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
        let mut branches = Vec::new();

        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            branches.push(self.parse_when_branch()?);

            if self.peek_kind() == &TokenKind::Comma || self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            }
        }

        let end = self.expect(TokenKind::RBrace)?.span;

        Ok(Expr::new(
            ExprKind::When { target, branches },
            start.merge(end),
        ))
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
        }

        self.expect(TokenKind::Arrow)?;
        let body = self.parse_expr(Precedence::Lowest)?;
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
        if let ExprKind::Binary {
            op: BinaryOp::Range,
            left,
            right,
        } = range_expr.kind
        {
            Ok(WhenPattern::InRange {
                start: *left,
                end: *right,
            })
        } else {
            Err(ParseError::new(
                "Expected range expression 'a..b' after 'in'",
                range_expr.span,
            ))
        }
    }

    fn parse_return(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::Return)?.span;
        // A value must start on the same line; anything else (`}`, `else`,
        // `,`, `)`, a newline...) ends a bare `return`.
        let has_value = !self.peek().is_preceded_by_newline && Self::starts_expr(self.peek_kind());

        if has_value {
            let val = self.parse_expr(Precedence::Lowest)?;
            let span = start.merge(val.span);
            Ok(Expr::new(ExprKind::Return(Some(Box::new(val))), span))
        } else {
            Ok(Expr::new(ExprKind::Return(None), start))
        }
    }

    fn parse_local_var(&mut self) -> Result<Expr, ParseError> {
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
        let initializer = self.parse_expr(Precedence::Lowest)?;
        let span = start.merge(initializer.span);

        Ok(Expr::new(
            ExprKind::LocalVar {
                is_var,
                name,
                ty,
                initializer: Box::new(initializer),
            },
            span,
        ))
    }
}
