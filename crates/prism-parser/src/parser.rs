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
    Membership = 6, // in, !in
    Elvis = 7,      // ?:
    Range = 8,      // ..
    Sum = 9,        // +, -
    Product = 10,   // *, /, %
    Prefix = 11,    // -, !, &, &mut
    Postfix = 12,   // ., ?.
    Call = 13,      // ()
}

/// The deepest nesting of expressions and types the parser accepts.
///
/// The limit bounds both the parser's own recursion and the height of every
/// tree it builds, so recursive passes over the AST (drop, clone, the type
/// checker) cannot overflow the stack either. Deeper source is rejected with
/// `E2002` instead of aborting the process.
pub const MAX_NESTING_DEPTH: usize = 256;

/// The error for `_`, a binding or a guard in a `when` without a subject (D2 of
/// `payload-patterns`).
const NO_SUBJECT_PATTERN: &str = "A branch of a 'when' without a subject is a condition: '_', 'val' and 'var' bindings and 'if' guards need a subject (use '&&' to combine conditions)";

/// The error for a reference written as a function type's receiver (`&User.() -> Unit`).
const RECEIVER_REFERENCE: &str = "A function type's receiver cannot be a reference: write 'User.() -> Unit' for a shared receiver or 'mut User.() -> Unit' for a mutable one";

/// The error for a prefix `*` anywhere but at the start of an assignment target.
/// The message for `++` or `--` (`operator`) anywhere: Prism has no increment or decrement
/// operators (D8 of `compound-assignment`).
fn no_increment_message(operator: &TokenKind) -> String {
    let compound = if operator == &TokenKind::PlusPlus {
        "+="
    } else {
        "-="
    };
    format!(
        "Prism has no '{}' operator; write 'x {} 1' as a statement instead",
        operator, compound
    )
}

/// The arithmetic operator of a compound assignment token (`+=` is `Add`), if `kind` is one.
fn compound_operator(kind: &TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::PlusEqual => Some(BinaryOp::Add),
        TokenKind::MinusEqual => Some(BinaryOp::Sub),
        TokenKind::StarEqual => Some(BinaryOp::Mul),
        TokenKind::SlashEqual => Some(BinaryOp::Div),
        TokenKind::PercentEqual => Some(BinaryOp::Mod),
        _ => None,
    }
}

/// Whether `kind` is `=` or a compound assignment operator.
fn is_assignment_operator(kind: &TokenKind) -> bool {
    kind == &TokenKind::Eq || compound_operator(kind).is_some()
}

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
    /// Whether the parser is trying a call's type-argument list that it may rewind
    /// ([`Self::try_parse_call_type_arguments`]): a joined `>=` then never closes a list.
    speculating: bool,
    /// The tokens [`Self::split_token`] changed while speculating, with their original
    /// contents, so a rewind restores the stream exactly.
    split_log: Vec<(usize, Token)>,
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
            speculating: false,
            split_log: Vec::new(),
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
        if self.speculating {
            self.split_log
                .push((self.cursor, self.tokens[self.cursor].clone()));
        }
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
        // A call's type-argument list closed by the `>` of `>=` is a comparison instead
        // (`a < b >= c`), so speculation never splits it.
        if self.peek_kind() == &TokenKind::GtEq && !self.speculating {
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

    /// Whether the current token is `=` or a compound assignment operator on the same line.
    fn at_assignment_operator(&self) -> bool {
        is_assignment_operator(self.peek_kind()) && !self.at_line_break()
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
        // `package consuming fun T.name()` is the visibility modifier followed by the receiver
        // modifier: a package declaration never continues with `fun` on its own line.
        let consuming_modifier = matches!(self.peek_ahead(1), TokenKind::Ident(name) if name == "consuming")
            && self.peek_ahead(2) == &TokenKind::Fun
            && !self.tokens[(self.cursor + 2).min(self.tokens.len() - 1)].is_preceded_by_newline;
        matches!(self.peek_ahead(1), TokenKind::Ident(_)) && !consuming_modifier
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
        // A context parameter, before every other modifier (D6 of `receiver-lambdas`).
        let context = self.parse_context_parameter()?;

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
        if self.at_context_parameter() {
            return Err(ParseError::new(
                "A context parameter comes before every other modifier, as in 'context(scope: RowScope) public fun Modifier.weight()'",
                self.peek().span,
            ));
        }
        if let Some(context) = &context {
            start_span = context.span;
        }

        // Library modifier (`external fun`, `intrinsic fun`, `intrinsic val`), after the
        // visibility and before everything else (D12 of `standard-library`). It is a modifier
        // only directly before `compose`, `mut`, `consuming`, `fun` or `val`, so the words stay
        // names everywhere else.
        let library = self.parse_library_modifier()?;

        // The `operator` modifier (D1 of `operator-conventions`), after the library modifier and
        // before `compose` and the receiver mode.
        let operator = self.parse_operator_modifier()?;

        // The `capture` modifier (`intrinsic capture fun T.weak()`, D12 of `owned-state`),
        // directly before `fun`. Before `compose`, `mut` or `consuming` it is read as the
        // modifier too, since an identifier never starts a declaration, for a better error.
        let capture = self.parse_capture_modifier(library, false)?;

        // Check compose modifier (`compose fun`)
        let mut is_compose = false;
        if self.peek_kind() == &TokenKind::Compose {
            self.bump();
            is_compose = true;
        }
        let capture = match capture {
            Some(span) => Some(span),
            None => self.parse_capture_modifier(library, is_compose)?,
        };

        // Receiver mode modifier (`mut fun T.name()`, `consuming fun T.name()`), always last.
        let mut mode = None;
        // An identifier never starts a declaration, so `consuming` is read as the modifier even
        // when `fun` does not follow, for a better error.
        let at_consuming =
            matches!(self.peek_kind(), TokenKind::Ident(name) if name == "consuming");
        if self.peek_kind() == &TokenKind::Mut || at_consuming {
            let tok = self.bump();
            let receiver_mode = if tok.kind == TokenKind::Mut {
                ReceiverMode::Mutable
            } else {
                ReceiverMode::Consuming
            };
            if self.peek_kind() != &TokenKind::Fun {
                return Err(ParseError::new(
                    format!(
                        "Expected 'fun' after '{}'; modifiers go in the order visibility, 'compose', then 'mut' or 'consuming', as in 'public {} fun T.name()'",
                        receiver_mode.as_str(),
                        receiver_mode.as_str()
                    ),
                    self.peek().span,
                ));
            }
            mode = Some((receiver_mode, tok.span));
        }

        if let Some(context) = &context {
            if let Some(modifier) = library {
                return Err(ParseError::new(
                    format!(
                        "An {} declaration cannot have a context parameter; declare a function with a body",
                        modifier.kind.as_str()
                    ),
                    context.span,
                ));
            }
            if self.peek_kind() != &TokenKind::Fun {
                return Err(ParseError::new(
                    "A context parameter applies to a function declaration, as in 'context(scope: RowScope) fun Modifier.weight()'",
                    context.span,
                ));
            }
        }

        if let Some(modifier) = library {
            if matches!(self.peek_kind(), TokenKind::Val | TokenKind::Var)
                && !is_compose
                && mode.is_none()
            {
                let property = self.parse_extension_property(visibility, modifier, start_span)?;
                return Ok(Declaration::ExtensionProperty(property));
            }
            if self.peek_kind() == &TokenKind::Struct && !is_compose && mode.is_none() {
                let st = self.parse_intrinsic_struct(visibility, modifier, start_span)?;
                return Ok(Declaration::Struct(st));
            }
            if self.peek_kind() != &TokenKind::Fun {
                return Err(ParseError::new(
                    format!(
                        "Expected 'fun' after '{}'; modifiers go in the order visibility, 'external' or 'intrinsic', 'compose', then 'mut' or 'consuming', as in 'public {} fun f()'",
                        modifier.kind.as_str(),
                        modifier.kind.as_str()
                    ),
                    self.peek().span,
                ));
            }
        }

        match self.peek_kind() {
            TokenKind::Fun => {
                let mut func =
                    self.parse_function(visibility, is_compose, mode, library, start_span)?;
                func.is_operator = operator.is_some();
                if let Some(span) = capture {
                    if func.receiver.is_none() {
                        return Err(ParseError::new(
                            format!(
                                "'capture' marks the extension that captures a value of its receiver's type, as in 'intrinsic capture fun <T> MutableState<T>.weak()'; '{}' has no receiver",
                                func.name
                            ),
                            span,
                        ));
                    }
                    func.is_capture = true;
                }
                if let Some(context) = &context {
                    Self::reject_context_type_parameters(context, &func.type_parameters)?;
                }
                func.context = context.map(Box::new);
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

    /// A function declaration from `fun` on. `mode` is the receiver modifier read before `fun`
    /// (`mut` or `consuming`), with its span; it requires a receiver.
    fn parse_function(
        &mut self,
        visibility: Visibility,
        is_compose: bool,
        mode: Option<(ReceiverMode, Span)>,
        library: Option<LibraryModifier>,
        start_span: Span,
    ) -> Result<FunctionDecl, ParseError> {
        self.expect(TokenKind::Fun)?;
        // Generic type parameters go between `fun` and the name or receiver: `fun <T> id()`.
        let type_parameters = if self.peek_kind() == &TokenKind::Lt {
            self.parse_type_parameter_list(true)?
        } else {
            Vec::new()
        };
        self.reject_unsupported_receiver_start()?;
        let (first_name, first_span) = self.expect_ident()?;

        // A receiver's type arguments (`fun <T> Box<T>.get()`), or type parameters written
        // after the name in the old placement (`fun id<T>()`).
        let mut receiver_arguments = Vec::new();
        let mut receiver_end = first_span;
        if self.peek_kind() == &TokenKind::Lt {
            let lt_span = self.peek().span;
            match self.try_parse_receiver_type_arguments() {
                Some((arguments, end)) => {
                    receiver_arguments = arguments;
                    receiver_end = end;
                }
                None => return Err(Self::type_parameters_after_name(lt_span)),
            }
        }

        // Extension receiver: `fun User.greet()`.
        let mut receiver = None;
        let name = if self.peek_kind() == &TokenKind::Dot {
            let receiver_span = first_span.merge(receiver_end);
            if first_name == "Heap" && !receiver_arguments.is_empty() {
                return Err(ParseError::new(
                    "An extension receiver cannot be a 'Heap<...>': calls look through 'Heap', so declare the extension on its contents, as in 'fun User.size()'",
                    receiver_span,
                ));
            }
            if type_parameters.iter().any(|p| p.name == first_name) {
                return Err(ParseError::new(
                    format!(
                        "A bare type parameter cannot be a receiver: '{}' is a type parameter of this function; declare the extension on a named type, as in 'fun <T> Box<T>.get()'",
                        first_name
                    ),
                    receiver_span,
                ));
            }
            self.bump(); // '.'
            let (name, name_span) = self.expect_ident()?;
            if self.peek_kind() == &TokenKind::Dot {
                return Err(ParseError::new(
                    "Type names are not qualified: an extension receiver is a single type name, as in 'fun User.greet()'",
                    first_span.merge(name_span),
                ));
            }
            if self.peek_kind() == &TokenKind::Lt {
                return Err(Self::type_parameters_after_name(self.peek().span));
            }
            let ty = if receiver_arguments.is_empty() {
                Type::named(first_name, receiver_span)
            } else {
                Type::generic(first_name, receiver_arguments, receiver_span)
            };
            receiver = Some(Box::new(Receiver {
                ty,
                mode: mode.map_or(ReceiverMode::Shared, |(mode, _)| mode),
                span: mode.map_or(receiver_span, |(_, span)| span.merge(receiver_span)),
            }));
            name
        } else if matches!(
            self.peek_kind(),
            TokenKind::Question | TokenKind::QuestionDot
        ) {
            return Err(ParseError::new(
                format!(
                    "An extension receiver cannot be nullable; declare it on '{}' and call it with '?.'",
                    first_name
                ),
                first_span.merge(self.peek().span),
            ));
        } else {
            if !receiver_arguments.is_empty() {
                return Err(Self::type_parameters_after_name(first_span));
            }
            if let Some((mode, span)) = mode {
                return Err(ParseError::new(
                    format!(
                        "'{}' modifies the receiver of an extension function ('{} fun T.name()'); '{}' has no receiver",
                        mode.as_str(),
                        mode.as_str(),
                        first_name
                    ),
                    span,
                ));
            }
            first_name
        };

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

        // Body: `= expr` or `{ ... }`, or none for a library declaration (D12 of
        // `standard-library`).
        let mut body = None;
        let mut end_span = return_type.as_ref().map_or(rparen_span, |t| t.span);

        let has_body = matches!(self.peek_kind(), TokenKind::Eq | TokenKind::LBrace);
        match library {
            Some(modifier) if has_body => {
                return Err(ParseError::new(
                    format!(
                        "An {} declaration has no body: the {} it declares",
                        modifier.kind.as_str(),
                        match modifier.kind {
                            LibraryKind::Intrinsic => "compiler provides what",
                            LibraryKind::External => "runtime provides the function",
                        }
                    ),
                    self.peek().span,
                ));
            }
            None if !has_body => {
                return Err(ParseError::new(
                    format!(
                        "A function needs a body: write '= expression' or '{{ ... }}' after the signature of '{}'",
                        name
                    ),
                    end_span,
                ));
            }
            _ => {}
        }

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
            context: None,
            visibility,
            is_compose,
            is_capture: false,
            is_operator: false,
            receiver,
            name,
            type_parameters,
            params,
            return_type,
            body,
            library,
            span: start_span.merge(end_span),
        })
    }

    /// The `capture` modifier at the cursor, if any: its span. It is a modifier directly before
    /// `fun`, `compose`, `mut` or `consuming` on the same line, and a name everywhere else. It
    /// must follow `intrinsic`, come directly before `fun` and not join `compose` (`compose`
    /// written before it), `mut` or `consuming`.
    fn parse_capture_modifier(
        &mut self,
        library: Option<LibraryModifier>,
        is_compose: bool,
    ) -> Result<Option<Span>, ParseError> {
        let following = &self.tokens[(self.cursor + 1).min(self.tokens.len() - 1)];
        let at_modifier = matches!(self.peek_kind(), TokenKind::Ident(word) if word == "capture")
            && !following.is_preceded_by_newline
            && match &following.kind {
                TokenKind::Fun | TokenKind::Compose | TokenKind::Mut => true,
                TokenKind::Ident(word) => word == "consuming",
                _ => false,
            };
        if !at_modifier {
            return Ok(None);
        }
        let span = self.bump().span;
        let misplaced = |message: &str| Err(ParseError::new(message.to_string(), span));
        if !matches!(
            library,
            Some(LibraryModifier {
                kind: LibraryKind::Intrinsic,
                ..
            })
        ) {
            return misplaced(
                "'capture' marks an intrinsic extension of the library, written 'intrinsic capture fun T.weak()'",
            );
        }
        if is_compose || self.peek_kind() != &TokenKind::Fun {
            return misplaced(
                "'capture' comes directly before 'fun', and a capture function is neither 'compose', 'mut' nor 'consuming'",
            );
        }
        Ok(Some(span))
    }

    /// The `operator` modifier at the cursor, if any: its span. At a declaration's start an
    /// identifier never starts a declaration, so `operator` there is the modifier; it must come
    /// directly before `compose`, `mut`, `consuming` or `fun` (after the visibility and the
    /// library modifier), once.
    fn parse_operator_modifier(&mut self) -> Result<Option<Span>, ParseError> {
        if !matches!(self.peek_kind(), TokenKind::Ident(word) if word == "operator") {
            return Ok(None);
        }
        let span = self.bump().span;
        let in_order = match self.peek_kind() {
            TokenKind::Fun | TokenKind::Compose | TokenKind::Mut => true,
            TokenKind::Ident(word) => word == "consuming",
            _ => false,
        };
        if !in_order {
            return Err(ParseError::new(
                "'operator' is written once, after the visibility and 'external' or 'intrinsic' and before 'compose', 'mut', 'consuming' and 'fun', as in 'public operator mut fun Bag.get(index: Int): &mut Item'",
                span,
            ));
        }
        Ok(Some(span))
    }

    /// Whether the next tokens are `context(`, the start of a context parameter: `context` is a
    /// modifier only there, at a top-level declaration's start, and a name everywhere else.
    fn at_context_parameter(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(word) if word == "context")
            && self.peek_ahead(1) == &TokenKind::LParen
    }

    /// Reads a context parameter `context(name: Type)` at a declaration's start (D6 of
    /// `receiver-lambdas`). The type follows the rules of an extension receiver.
    fn parse_context_parameter(&mut self) -> Result<Option<ContextParameter>, ParseError> {
        if !self.at_context_parameter() {
            return Ok(None);
        }
        let start = self.bump().span; // 'context'
        self.bump(); // '('
        if self.peek_kind() == &TokenKind::RParen {
            return Err(ParseError::new(
                "A context parameter needs a name and a type, as in 'context(scope: RowScope)'",
                start.merge(self.peek().span),
            ));
        }
        if self.at_vararg_modifier() {
            return Err(Self::vararg_misplaced(
                "a context parameter",
                self.peek().span,
            ));
        }
        let name_token = self.peek().clone();
        let TokenKind::Ident(name) = name_token.kind else {
            return Err(ParseError::new(
                "A context parameter needs a name and a type, as in 'context(scope: RowScope)'",
                name_token.span,
            ));
        };
        if self.peek_ahead(1) != &TokenKind::Colon {
            return Err(ParseError::new(
                format!(
                    "A context parameter needs a name and a type, as in 'context(scope: {})'",
                    name
                ),
                name_token.span,
            ));
        }
        self.bump(); // name
        self.bump(); // ':'
        let ty = self.parse_type()?;
        let message = match &ty.kind {
            TypeKind::Named { name, type_args } if name == "Heap" && !type_args.is_empty() => {
                Some("A context parameter's type cannot be a 'Heap<...>'; use its contents' type")
            }
            TypeKind::Named { .. } => None,
            TypeKind::Nullable(_) => Some("A context parameter's type cannot be nullable"),
            TypeKind::Reference { .. } => Some(
                "A context parameter's type cannot be a reference: the context is always passed as a shared borrow",
            ),
            TypeKind::Unit | TypeKind::Function { .. } => Some(
                "A context parameter's type must be a single type name, not a parenthesized or function type",
            ),
        };
        if let Some(message) = message {
            return Err(ParseError::new(message, ty.span));
        }
        if self.peek_kind() == &TokenKind::Comma {
            return Err(ParseError::new(
                "A declaration takes one context parameter",
                self.peek().span,
            ));
        }
        let end = self.expect(TokenKind::RParen)?.span;
        Ok(Some(ContextParameter {
            name,
            ty,
            span: start.merge(end),
        }))
    }

    /// Rejects a context type mentioning the declaration's own type parameters
    /// (`context(s: Box<T>) fun <T> k()`): the context is found by its type alone.
    fn reject_context_type_parameters(
        context: &ContextParameter,
        type_parameters: &[TypeParameter],
    ) -> Result<(), ParseError> {
        fn mentions(ty: &Type, type_parameters: &[TypeParameter]) -> Option<String> {
            match &ty.kind {
                TypeKind::Named { name, type_args } => {
                    if type_args.is_empty() && type_parameters.iter().any(|p| &p.name == name) {
                        return Some(name.clone());
                    }
                    type_args.iter().find_map(|a| mentions(a, type_parameters))
                }
                TypeKind::Nullable(inner) | TypeKind::Reference { inner, .. } => {
                    mentions(inner, type_parameters)
                }
                TypeKind::Unit => None,
                TypeKind::Function {
                    receiver,
                    params,
                    return_type,
                    ..
                } => receiver
                    .as_ref()
                    .and_then(|r| mentions(&r.ty, type_parameters))
                    .or_else(|| params.iter().find_map(|p| mentions(p, type_parameters)))
                    .or_else(|| mentions(return_type, type_parameters)),
            }
        }
        match mentions(&context.ty, type_parameters) {
            Some(name) => Err(ParseError::new(
                format!(
                    "A context parameter's type cannot mention the declaration's type parameter '{}': the context is found by its type alone",
                    name
                ),
                context.ty.span,
            )),
            None => Ok(()),
        }
    }

    /// Reads an `external` or `intrinsic` modifier: the word directly before `compose`, `mut`,
    /// `consuming`, `fun` or `val`. Two library modifiers in a row are rejected.
    fn parse_library_modifier(&mut self) -> Result<Option<LibraryModifier>, ParseError> {
        let kind = match self.peek_kind() {
            TokenKind::Ident(word) if word == "external" => LibraryKind::External,
            TokenKind::Ident(word) if word == "intrinsic" => LibraryKind::Intrinsic,
            _ => return Ok(None),
        };
        let follows_modifier = |kind: &TokenKind| match kind {
            TokenKind::Compose
            | TokenKind::Mut
            | TokenKind::Fun
            | TokenKind::Val
            | TokenKind::Var
            | TokenKind::Struct => true,
            TokenKind::Ident(word) => {
                word == "consuming" || word == "capture" || word == "operator"
            }
            _ => false,
        };
        let next = self.peek_ahead(1).clone();
        if let TokenKind::Ident(word) = &next {
            if word == "external" || word == "intrinsic" {
                return Err(ParseError::new(
                    "A declaration takes one of 'external' and 'intrinsic', once",
                    self.tokens[(self.cursor + 1).min(self.tokens.len() - 1)].span,
                ));
            }
        }
        if !follows_modifier(&next) {
            return Ok(None);
        }
        let span = self.bump().span;
        Ok(Some(LibraryModifier { kind, span }))
    }

    /// An intrinsic extension property from `val` or `var` on: `val <T> List<T>.size: Int`,
    /// with no initializer (D6 of `standard-library`); `var` declares a writable one (D4 of
    /// `state-handles`).
    fn parse_extension_property(
        &mut self,
        visibility: Visibility,
        library: LibraryModifier,
        start_span: Span,
    ) -> Result<ExtensionPropertyDecl, ParseError> {
        let writable = self.bump().kind == TokenKind::Var;
        if library.kind == LibraryKind::External {
            return Err(ParseError::new(
                "An external declaration is a function: 'external fun name(...)'; properties are 'intrinsic val'",
                library.span,
            ));
        }
        let type_parameters = if self.peek_kind() == &TokenKind::Lt {
            self.parse_type_parameter_list(true)?
        } else {
            Vec::new()
        };
        let (receiver_name, receiver_span) = self.expect_ident()?;
        let mut receiver_arguments = Vec::new();
        let mut receiver_end = receiver_span;
        if self.peek_kind() == &TokenKind::Lt {
            let lt_span = self.peek().span;
            match self.try_parse_receiver_type_arguments() {
                Some((arguments, end)) => {
                    receiver_arguments = arguments;
                    receiver_end = end;
                }
                None => {
                    return Err(ParseError::new(
                        "Expected '.' and the property name after the receiver type of an intrinsic property",
                        lt_span,
                    ))
                }
            }
        }
        if self.peek_kind() != &TokenKind::Dot {
            return Err(ParseError::new(
                "An intrinsic property has a receiver, as in 'intrinsic val String.length: Int'",
                self.peek().span,
            ));
        }
        let receiver_span = receiver_span.merge(receiver_end);
        let receiver = if receiver_arguments.is_empty() {
            Type::named(receiver_name, receiver_span)
        } else {
            Type::generic(receiver_name, receiver_arguments, receiver_span)
        };
        self.bump(); // '.'
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_type()?;
        if matches!(self.peek_kind(), TokenKind::Eq | TokenKind::LBrace) {
            return Err(ParseError::new(
                "An intrinsic property has no initializer: the compiler provides its value",
                self.peek().span,
            ));
        }
        let end_span = ty.span;
        Ok(ExtensionPropertyDecl {
            visibility,
            library,
            type_parameters,
            receiver,
            name,
            ty,
            writable,
            span: start_span.merge(end_span),
        })
    }

    /// Rejects a receiver that starts with `&` or `(`: a receiver is a single type name, and the
    /// receiver mode decides how it is borrowed.
    fn reject_unsupported_receiver_start(&self) -> Result<(), ParseError> {
        let message = match self.peek_kind() {
            TokenKind::Amp | TokenKind::AmpAmp => {
                "An extension receiver cannot be a reference: the receiver mode ('fun', 'mut fun', 'consuming fun') decides how the receiver is borrowed"
            }
            TokenKind::LParen => {
                "An extension receiver must be a single type name, not a parenthesized or function type"
            }
            _ => return Ok(()),
        };
        Err(ParseError::new(message, self.peek().span))
    }

    /// The error for type parameters written after a function's name (`fun id<T>()`).
    fn type_parameters_after_name(span: Span) -> ParseError {
        ParseError::new(
            "Type parameters go before the name, as in 'fun <T> id(...)', or before the receiver, as in 'fun <T> Box<T>.get()'",
            span,
        )
    }

    /// Reads the type arguments of a receiver (`Box<T>` in `fun <T> Box<T>.get()`) when they
    /// parse as types and a `.` follows them; otherwise rewinds and returns `None`. Returns the
    /// arguments and the span of the closing `>`.
    fn try_parse_receiver_type_arguments(&mut self) -> Option<(Vec<Type>, Span)> {
        let (arguments, end) = self.speculate(|parser| {
            let arguments = parser.parse_type_argument_list()?;
            let end = parser.tokens[parser.cursor - 1].span;
            if parser.peek_kind() == &TokenKind::Dot {
                Ok((arguments, end))
            } else {
                Err(ParseError::new("not a receiver", end))
            }
        })?;
        Some((arguments, end))
    }

    /// Whether the current token is `vararg` used as a modifier: followed by a parameter's name
    /// (D1 of `vararg`). Anywhere else `vararg` is a name.
    fn at_vararg_modifier(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(word) if word == "vararg")
            && matches!(self.peek_ahead(1), TokenKind::Ident(_) | TokenKind::Compose)
    }

    /// The error for `vararg` where only a `fun` declaration's parameter may carry it.
    fn vararg_misplaced(place: &str, span: Span) -> ParseError {
        ParseError::new(
            format!(
                "'vararg' marks a parameter of a 'fun' declaration, not {}; take a 'List<T>' instead",
                place
            ),
            span,
        )
    }

    /// Function parameters up to (not including) the closing `)`.
    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params: Vec<Param> = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            // `vararg` before a parameter's name is the modifier (D1 of `vararg`); followed by
            // `:` it is the parameter's own name.
            let vararg_span = if self.at_vararg_modifier() {
                Some(self.bump().span)
            } else {
                None
            };
            let (param_name, name_span) = self.expect_ident()?;
            let p_span = vararg_span.unwrap_or(name_span);
            if let Some(span) = vararg_span {
                if params.iter().any(|param| param.is_vararg) {
                    return Err(ParseError::new(
                        "A function has at most one vararg parameter; pass the other values as a list",
                        span,
                    ));
                }
            }
            self.expect(TokenKind::Colon)?;
            if let (Some(_), Some(kind)) = (vararg_span, self.parameter_modifier_before_type()) {
                let word = match kind {
                    ParameterKind::Attached => "attached",
                    _ => "escaping",
                };
                return Err(ParseError::new(
                    format!(
                        "A vararg parameter cannot be '{}': it is a list the callee owns, whose function-typed elements are stored values",
                        word
                    ),
                    self.peek().span,
                ));
            }
            let (kind, ty) = self.parse_parameter_type()?;
            let mut default_value = None;
            if self.peek_kind() == &TokenKind::Eq {
                if vararg_span.is_some() {
                    return Err(ParseError::new(
                        "A vararg parameter has no default value: a call with no element passes the empty list",
                        self.peek().span,
                    ));
                }
                self.bump();
                default_value = Some(self.parse_expr(Precedence::Lowest)?);
            }
            let end_p_span = default_value.as_ref().map(|d| d.span).unwrap_or(ty.span);
            params.push(Param {
                name: param_name,
                ty,
                kind,
                default_value,
                is_vararg: vararg_span.is_some(),
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
        let (name, name_span) = self.expect_ident()?;
        let type_parameters = self.parse_optional_type_parameters()?;

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
                if self.at_vararg_modifier() {
                    return Err(Self::vararg_misplaced("a struct's field", self.peek().span));
                }
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
            name_span,
            type_parameters,
            marker,
            fields,
            intrinsic: None,
            span: start_span.merge(end_span),
        })
    }

    /// An `intrinsic struct` from `struct` on: a name, optional type parameters and an optional
    /// marker, and no fields or block (D16 of `compose-transform`). An `external struct` is
    /// rejected: externals are functions.
    fn parse_intrinsic_struct(
        &mut self,
        visibility: Visibility,
        library: LibraryModifier,
        start_span: Span,
    ) -> Result<StructDecl, ParseError> {
        if library.kind == LibraryKind::External {
            return Err(ParseError::new(
                "An external declaration is a function: 'external fun name(...)'; types the compiler provides are 'intrinsic struct'",
                library.span,
            ));
        }
        self.expect(TokenKind::Struct)?;
        let (name, name_span) = self.expect_ident()?;
        let type_parameters = self.parse_optional_type_parameters()?;
        let mut end_span = self.tokens[self.cursor.saturating_sub(1)].span;
        let marker = match self.parse_optional_marker()? {
            Some((marker, marker_span)) => {
                end_span = marker_span;
                Some(marker)
            }
            None => None,
        };
        if matches!(self.peek_kind(), TokenKind::LParen | TokenKind::LBrace) {
            return Err(ParseError::new(
                format!(
                    "An intrinsic struct has no fields and no constructor: the compiler provides its representation, as in 'intrinsic struct {}'",
                    name
                ),
                self.peek().span,
            ));
        }
        Ok(StructDecl {
            visibility,
            name,
            name_span,
            type_parameters,
            marker,
            fields: Vec::new(),
            intrinsic: Some(library),
            span: start_span.merge(end_span),
        })
    }

    fn parse_enum(
        &mut self,
        visibility: Visibility,
        start_span: Span,
    ) -> Result<EnumDecl, ParseError> {
        self.expect(TokenKind::Enum)?;
        let (name, name_span) = self.expect_ident()?;
        let type_parameters = self.parse_optional_type_parameters()?;
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
            name_span,
            type_parameters,
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

        let (name, name_span) = self.expect_ident()?;
        if matches!(self.peek_kind(), TokenKind::Dot | TokenKind::Lt) {
            return Err(ParseError::new(
                format!(
                    "A top-level 'val' has no receiver: extension properties such as 'val {}.name' are declared only by the bundled library, with 'intrinsic val'",
                    name
                ),
                name_span.merge(self.peek().span),
            ));
        }
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

    /// The parameter modifier at the cursor, `escaping` or `attached` written before a type on
    /// the same line: the modifier, directly before `(`, `compose`, `mut` or a receiver's type
    /// name, or misplaced before a type that cannot take it (`escaping Int`, `attached &T`).
    /// Before anything else either word is a name, also a type name.
    fn parameter_modifier_before_type(&self) -> Option<ParameterKind> {
        let kind = match self.peek_kind() {
            TokenKind::Ident(name) if name == "escaping" => ParameterKind::Escaping,
            TokenKind::Ident(name) if name == "attached" => ParameterKind::Attached,
            _ => return None,
        };
        let following = &self.tokens[(self.cursor + 1).min(self.tokens.len() - 1)];
        let before_type = matches!(
            following.kind,
            TokenKind::LParen
                | TokenKind::Compose
                | TokenKind::Mut
                | TokenKind::Ident(_)
                | TokenKind::Amp
                | TokenKind::AmpAmp
        );
        (!following.is_preceded_by_newline && before_type).then_some(kind)
    }

    fn parameter_modifier_misplaced(kind: ParameterKind, span: Span) -> ParseError {
        match kind {
            ParameterKind::Attached => ParseError::new(
                "'attached' marks parameters of function type that the composition keeps",
                span,
            ),
            _ => ParseError::new("'escaping' marks parameters of function type", span),
        }
    }

    /// The type of a parameter of a declaration or a function type, with its `escaping` or
    /// `attached` modifier: `escaping () -> Unit`, `attached compose () -> Unit`,
    /// `escaping mut Builder.() -> Unit` or `attached (() -> Unit)?`. A modifier is accepted
    /// only before a function type or a parenthesized nullable function type, and only one.
    fn parse_parameter_type(&mut self) -> Result<(ParameterKind, Type), ParseError> {
        let Some(kind) = self.parameter_modifier_before_type() else {
            return Ok((ParameterKind::Plain, self.parse_type()?));
        };
        let modifier = self.bump().span;
        if !matches!(
            self.peek_kind(),
            TokenKind::LParen | TokenKind::Compose | TokenKind::Mut | TokenKind::Ident(_)
        ) {
            return Err(Self::parameter_modifier_misplaced(kind, modifier));
        }
        let ty = self.parse_type()?;
        let is_function = match &ty.kind {
            TypeKind::Function { .. } => true,
            TypeKind::Nullable(inner) => matches!(inner.kind, TypeKind::Function { .. }),
            _ => false,
        };
        if !is_function {
            return Err(Self::parameter_modifier_misplaced(kind, modifier));
        }
        Ok((kind, ty))
    }

    pub fn parse_type(&mut self) -> Result<Type, ParseError> {
        let outer_peak = self.enter()?;
        let result = self.parse_type_inner();
        self.exit(outer_peak);
        result
    }

    fn parse_type_inner(&mut self) -> Result<Type, ParseError> {
        let start_span = self.peek().span;

        // `escaping` or `attached` outside a parameter's type (`val h: escaping () -> Unit`),
        // or after another modifier (`escaping attached () -> Unit`).
        if let Some(kind) = self.parameter_modifier_before_type() {
            return Err(Self::parameter_modifier_misplaced(kind, start_span));
        }

        // 1. Borrow reference: &T or &mut T
        if matches!(self.peek_kind(), TokenKind::Amp | TokenKind::AmpAmp) {
            let start_span = self.bump_amp().span;
            let is_mut = if self.peek_kind() == &TokenKind::Mut {
                self.bump();
                true
            } else {
                false
            };
            // `&User.() -> Unit` reads as a reference receiver, which a function type's receiver
            // cannot be (D1 of `receiver-lambdas`); `&(User.() -> Unit)` is a reference to one.
            let unparenthesized = !matches!(self.peek_kind(), TokenKind::LParen);
            let inner = self.parse_type()?;
            if unparenthesized {
                if let TypeKind::Function {
                    receiver: Some(_), ..
                } = &inner.kind
                {
                    return Err(ParseError::new(RECEIVER_REFERENCE, start_span));
                }
            }
            let span = start_span.merge(inner.span);
            return Ok(if is_mut {
                Type::reference_mut(inner, span)
            } else {
                Type::reference(inner, span)
            });
        }

        // 2. Composable function type: `compose (Int) -> Unit`, or with a receiver,
        // `compose ColumnScope.() -> Unit` and `compose mut Builder.() -> Unit`.
        let is_compose = self.peek_kind() == &TokenKind::Compose
            && matches!(
                self.peek_ahead(1),
                TokenKind::LParen | TokenKind::Mut | TokenKind::Ident(_)
            );
        if is_compose {
            self.bump(); // 'compose'
            if self.peek_kind() == &TokenKind::LParen {
                return self.parse_paren_type(start_span, true, None);
            }
        }

        // 3. A mutable receiver: `mut Builder.(String) -> Unit`.
        let mut mutable_receiver = false;
        if self.peek_kind() == &TokenKind::Mut {
            let mut_span = self.bump().span;
            mutable_receiver = true;
            if self.peek_kind() == &TokenKind::Compose {
                return Err(ParseError::new(
                    "'compose' comes before 'mut' in a function type, as in 'compose mut Builder.() -> Unit'",
                    self.peek().span,
                ));
            }
            if !matches!(self.peek_kind(), TokenKind::Ident(_)) {
                return Err(ParseError::new(
                    "'mut' before a type marks the receiver of a function type as mutable, as in 'mut Builder.(String) -> Unit'",
                    mut_span,
                ));
            }
        }
        if matches!(self.peek_kind(), TokenKind::Ident(word) if word == "consuming")
            && matches!(self.peek_ahead(1), TokenKind::Ident(_))
            && !self.tokens[(self.cursor + 1).min(self.tokens.len() - 1)].is_preceded_by_newline
        {
            return Err(ParseError::new(
                "A function type's receiver is shared ('T.() -> R') or 'mut' ('mut T.() -> R'); 'consuming' receivers are not supported in function types",
                self.peek().span,
            ));
        }

        // 4. Unit, parenthesized type or function type: (), (Int), (Int, String) -> Bool
        if self.peek_kind() == &TokenKind::LParen && !is_compose && !mutable_receiver {
            return self.parse_paren_type(start_span, false, None);
        }

        // 5. Named type: Ident optionally followed by generic arguments <T, U>
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
            Type::named(name, span.merge(end_span))
        } else {
            Type::generic(name, type_args, span.merge(end_span))
        };

        // 6. A function type with a receiver: the named type directly followed by `.(`.
        if self.peek_kind() == &TokenKind::Dot && self.peek_ahead(1) == &TokenKind::LParen {
            if let TypeKind::Named { name, type_args } = &base_ty.kind {
                if name == "Heap" && !type_args.is_empty() {
                    return Err(ParseError::new(
                        "A function type's receiver cannot be a 'Heap<...>': calls look through 'Heap', so use its contents as the receiver, as in 'User.() -> Unit'",
                        base_ty.span,
                    ));
                }
            }
            self.bump(); // '.'
            let receiver = Box::new(FunctionTypeReceiver {
                mutable: mutable_receiver,
                ty: base_ty,
            });
            return self.parse_paren_type(start_span, is_compose, Some(receiver));
        }
        if self.peek_kind() == &TokenKind::QuestionDot && self.peek_ahead(1) == &TokenKind::LParen {
            return Err(ParseError::new(
                format!(
                    "A function type's receiver cannot be nullable; use '{}' as the receiver and call the function with '?.'",
                    base_ty
                ),
                base_ty.span.merge(self.peek().span),
            ));
        }
        if is_compose || mutable_receiver {
            return Err(ParseError::new(
                format!(
                    "Expected a function type after '{}': a parameter list, or a receiver followed by one, as in '{} {}.() -> Unit'",
                    if mutable_receiver { "mut" } else { "compose" },
                    if mutable_receiver { "mut" } else { "compose" },
                    base_ty
                ),
                self.peek().span,
            ));
        }

        // 7. Suffix (e.g. nullable ?)
        self.parse_type_suffix(base_ty)
    }

    /// A type starting with `(`: unit `()`, a parenthesized type `(Int)`, or a
    /// function type `(A, B) -> R`, which is the only form allowed after `compose` or a
    /// `receiver`.
    fn parse_paren_type(
        &mut self,
        start_span: Span,
        is_compose: bool,
        receiver: Option<Box<FunctionTypeReceiver>>,
    ) -> Result<Type, ParseError> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut parameter_kinds = Vec::new();
        let mut first_modifier = None;
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            if self.at_vararg_modifier() {
                return Err(Self::vararg_misplaced(
                    "a function type's parameter",
                    self.peek().span,
                ));
            }
            if first_modifier.is_none() {
                if let Some(kind) = self.parameter_modifier_before_type() {
                    first_modifier = Some((kind, self.peek().span));
                }
            }
            let (kind, param) = self.parse_parameter_type()?;
            params.push(param);
            parameter_kinds.push(kind);
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
                    receiver,
                    params,
                    parameter_kinds,
                    return_type: Box::new(ret),
                },
                span,
            ));
        }
        // `escaping` or `attached` inside parentheses that are not a function type's parameter
        // list, as in `val h: (escaping () -> Unit)?`.
        if let Some((kind, span)) = first_modifier {
            return Err(Self::parameter_modifier_misplaced(kind, span));
        }
        if is_compose || receiver.is_some() {
            return Err(ParseError::new(
                format!(
                    "Expected '->' after the parameter types of a {} function type, found '{}'",
                    if receiver.is_some() {
                        "receiver's"
                    } else {
                        "'compose'"
                    },
                    self.peek_kind()
                ),
                self.peek().span,
            ));
        }
        // `(User).() -> Unit` or `(() -> Unit).() -> Unit`: a receiver is a single type name.
        if self.peek_kind() == &TokenKind::Dot && self.peek_ahead(1) == &TokenKind::LParen {
            return Err(ParseError::new(
                "A function type's receiver must be a single type name, not a parenthesized or function type",
                start_span.merge(rparen_span),
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

    /// The type parameters of a struct or enum, after its name, if any.
    fn parse_optional_type_parameters(&mut self) -> Result<Vec<TypeParameter>, ParseError> {
        if self.peek_kind() == &TokenKind::Lt {
            self.parse_type_parameter_list(false)
        } else {
            Ok(Vec::new())
        }
    }

    /// A type parameter list `<T, U : Copy,>` from its `<`: one or more names, each with an
    /// optional bound `: Copy` or `: Clone` when `allow_bounds` (function type parameters), and
    /// an optional trailing comma.
    fn parse_type_parameter_list(
        &mut self,
        allow_bounds: bool,
    ) -> Result<Vec<TypeParameter>, ParseError> {
        const ONE_BOUND: &str = "Only 'Copy' or 'Clone' can bound a type parameter until traits exist, and a type parameter has exactly one bound, as in '<T : Copy>' ('Copy' already implies 'Clone')";
        let lt_span = self.expect(TokenKind::Lt)?.span;
        let mut parameters = Vec::new();
        while !self.at_closing_angle() && !self.is_eof() {
            let (name, span) = self.expect_ident()?;
            if name == "Copy" || name == "Clone" {
                return Err(ParseError::new(ONE_BOUND, span));
            }
            let mut bound = None;
            let mut end = span;
            if self.peek_kind() == &TokenKind::Colon {
                let colon = self.bump().span;
                if !allow_bounds {
                    return Err(ParseError::new(
                        "Type parameters of a struct or enum cannot be bounded yet; only a function's type parameters take a bound, as in 'fun <T : Copy> f()'",
                        colon,
                    ));
                }
                let tok = self.peek().clone();
                bound = Some(match &tok.kind {
                    TokenKind::Ident(marker) if marker == "Copy" => TypeMarker::Copy,
                    TokenKind::Ident(marker) if marker == "Clone" => TypeMarker::Clone,
                    _ => return Err(ParseError::new(ONE_BOUND, tok.span)),
                });
                end = self.bump().span;
            }
            parameters.push(TypeParameter {
                name,
                bound,
                span: span.merge(end),
            });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        if parameters.is_empty() {
            return Err(ParseError::new(
                "A type parameter list needs at least one type parameter, as in '<T>'",
                lt_span.merge(self.peek().span),
            ));
        }
        self.expect_closing_angle()?;
        Ok(parameters)
    }

    /// A type-argument list `<Int, Box<String>>` from its `<`: one or more types separated by
    /// commas.
    fn parse_type_argument_list(&mut self) -> Result<Vec<Type>, ParseError> {
        self.expect(TokenKind::Lt)?;
        let mut arguments = vec![self.parse_type()?];
        while self.peek_kind() == &TokenKind::Comma {
            self.bump();
            arguments.push(self.parse_type()?);
        }
        self.expect_closing_angle()?;
        Ok(arguments)
    }

    /// Runs `attempt` and keeps what it consumed when it succeeds. On any error it rewinds the
    /// cursor and every token it split, reports nothing and returns `None`.
    fn speculate<T>(
        &mut self,
        attempt: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Option<T> {
        let cursor = self.cursor;
        let peak = self.peak;
        let outer_speculating = std::mem::replace(&mut self.speculating, true);
        let log_start = self.split_log.len();
        let result = attempt(self);
        self.speculating = outer_speculating;
        match result {
            Ok(value) => {
                if !self.speculating {
                    self.split_log.clear();
                }
                Some(value)
            }
            Err(_) => {
                while self.split_log.len() > log_start {
                    let (index, token) = self.split_log.pop().expect("a logged split");
                    self.tokens[index] = token;
                }
                self.cursor = cursor;
                self.peak = peak;
                None
            }
        }
    }

    /// After an identifier or member name, tries `<` as the call's type-argument list
    /// (`id<Int>(1)`, `remember<Int> { 0 }`): it is one when the tokens up to the matching `>`
    /// parse as types, the list closes with `>` (not the `>` of `>=`), and the next token is a
    /// `(` or `{` on the same line. Otherwise the stream is unchanged and the `<` is a
    /// comparison.
    fn try_parse_call_type_arguments(&mut self) -> Option<Vec<Type>> {
        self.speculate(|parser| {
            let arguments = parser.parse_type_argument_list()?;
            let opens_call = matches!(parser.peek_kind(), TokenKind::LParen | TokenKind::LBrace)
                && !parser.at_line_break();
            if opens_call {
                Ok(arguments)
            } else {
                Err(ParseError::new("not a call", parser.peek().span))
            }
        })
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

        while !self.is_eof() {
            let op_span = self.peek().span;
            // Explicit type arguments of a call bind like the call itself, tighter than `<`.
            let type_arguments = if self.peek_kind() == &TokenKind::Lt
                && !self.at_line_break()
                && matches!(
                    left.kind,
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. }
                ) {
                self.try_parse_call_type_arguments()
            } else {
                None
            };
            if type_arguments.is_none() && precedence >= self.current_precedence() {
                break;
            }
            self.peak = self.depth;
            left = match type_arguments {
                Some(type_arguments) if self.peek_kind() == &TokenKind::LParen => {
                    self.parse_call(left, type_arguments)?
                }
                Some(type_arguments) => self.parse_trailing_lambda_call(left, type_arguments)?,
                None => self.parse_infix(left)?,
            };
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
            TokenKind::Eq
            | TokenKind::PlusEqual
            | TokenKind::MinusEqual
            | TokenKind::StarEqual
            | TokenKind::SlashEqual
            | TokenKind::PercentEqual => Precedence::Assign,
            TokenKind::PipePipe => Precedence::LogicalOr,
            TokenKind::AmpAmp => Precedence::LogicalAnd,
            TokenKind::EqEq | TokenKind::BangEq => Precedence::Equality,
            TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq => {
                Precedence::Comparison
            }
            TokenKind::In | TokenKind::NotIn => Precedence::Membership,
            TokenKind::QuestionColon => Precedence::Elvis,
            TokenKind::DotDot | TokenKind::DotDotLess => Precedence::Range,
            TokenKind::Plus | TokenKind::Minus => Precedence::Sum,
            TokenKind::Star | TokenKind::Slash | TokenKind::Percent => Precedence::Product,
            TokenKind::Dot
            | TokenKind::QuestionDot
            | TokenKind::PlusPlus
            | TokenKind::MinusMinus => Precedence::Postfix,
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
            | TokenKind::This
            | TokenKind::Compose => self.parse_atom(),

            TokenKind::TemplateStart => self.parse_template(),

            // Unary operators
            TokenKind::Minus | TokenKind::Bang | TokenKind::Amp | TokenKind::AmpAmp => {
                self.parse_unary()
            }

            TokenKind::PlusPlus | TokenKind::MinusMinus => Err(ParseError::new(
                no_increment_message(self.peek_kind()),
                self.peek().span,
            )),

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
            TokenKind::This => ExprKind::This,
            _ => unreachable!("parse_prefix only dispatches atoms here"),
        };
        Ok(Expr::new(kind, tok.span))
    }

    /// A string template, from its `TemplateStart` to its `TemplateEnd` (D5 of
    /// `string-templates`). A `$name` part is the identifier (or `this`) spanning the name
    /// without its `$`; a `${...}` part holds one expression, parsed with newlines insignificant
    /// as inside parentheses.
    fn parse_template(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokenKind::TemplateStart)?.span;
        let mut parts = Vec::new();
        loop {
            let token = self.peek().clone();
            match token.kind {
                TokenKind::TemplateText(text) => {
                    self.bump();
                    parts.push(TemplatePart::Text(text));
                }
                TokenKind::TemplateName(name) => {
                    self.bump();
                    let span = token.span;
                    let name_span = Span::new(span.start + 1, span.end, span.line, span.column + 1);
                    let kind = if name == "this" {
                        ExprKind::This
                    } else {
                        ExprKind::Ident(name)
                    };
                    parts.push(TemplatePart::Expression(Expr::new(kind, name_span)));
                }
                TokenKind::TemplateExpressionStart => {
                    self.bump();
                    if self.peek_kind() == &TokenKind::TemplateExpressionEnd {
                        return Err(ParseError::new(
                            "Expected an expression in '${}'",
                            self.peek().span,
                        ));
                    }
                    let part = self.with_newlines(false, |p| p.parse_expr(Precedence::Lowest))?;
                    if self.peek_kind() != &TokenKind::TemplateExpressionEnd {
                        return Err(ParseError::new(
                            format!("Expected '}}' to close '${{', found '{}'", self.peek_kind()),
                            self.peek().span,
                        ));
                    }
                    self.bump();
                    parts.push(TemplatePart::Expression(part));
                }
                TokenKind::TemplateEnd => {
                    let end = self.bump().span;
                    return Ok(Expr::new(ExprKind::Template(parts), start.merge(end)));
                }
                other => {
                    return Err(ParseError::new(
                        format!("Unexpected token in string template: '{}'", other),
                        token.span,
                    ))
                }
            }
        }
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
                | TokenKind::TemplateStart
                | TokenKind::Char(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Ident(_)
                | TokenKind::This
                | TokenKind::Compose
                | TokenKind::Minus
                | TokenKind::Bang
                | TokenKind::Amp
                | TokenKind::AmpAmp
                | TokenKind::Star
                | TokenKind::PlusPlus
                | TokenKind::MinusMinus
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

    /// A closure `{ a, b -> ... }`, `{ -> ... }` declaring zero parameters, or `{ ... }` with no
    /// header, whose parameters the type checker decides (D1 of `implicit-it`).
    fn parse_closure(&mut self) -> Result<Expr, ParseError> {
        let has_header = self.is_closure_with_arrow_ahead();
        let start = self.expect(TokenKind::LBrace)?.span;
        let mut params = Vec::new();
        let mut arrow = None;

        if has_header {
            while self.peek_kind() != &TokenKind::Arrow && !self.is_eof() {
                if self.at_vararg_modifier() {
                    return Err(Self::vararg_misplaced(
                        "a closure's parameter",
                        self.peek().span,
                    ));
                }
                let (name, span) = self.expect_ident()?;
                params.push(ClosureParameter { name, span });
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
                arrow,
                body: Block {
                    statements,
                    tail,
                    span: body_start.merge(end),
                    close_brace: end,
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
            TokenKind::LParen => self.parse_call(left, Vec::new()),
            // Trailing lambda call without parentheses: callee { body }
            TokenKind::LBrace => self.parse_trailing_lambda_call(left, Vec::new()),
            // Indexing: target[index]
            TokenKind::LBracket => self.parse_index(left),
            TokenKind::PlusPlus | TokenKind::MinusMinus => Err(ParseError::new(
                no_increment_message(self.peek_kind()),
                self.peek().span,
            )),
            // Assignments are statements, parsed by `parse_statement_or_expression`.
            kind if is_assignment_operator(kind) => Err(ParseError::new(
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

    fn parse_call(&mut self, left: Expr, type_arguments: Vec<Type>) -> Result<Expr, ParseError> {
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
                is_trailing_lambda: true,
                is_spread: false,
            });
        }

        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(left),
                type_arguments,
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
            // A spread `*xs` passes a whole list to a vararg parameter (D5 of `vararg`); which
            // spreads a call accepts is the type checker's rule.
            let is_spread = self.peek_kind() == &TokenKind::Star;
            if is_spread {
                let star = self.bump().span;
                if name.is_none() {
                    arg_start = star;
                }
            }
            let value = self.parse_expr(Precedence::Lowest)?;
            let arg_span = arg_start.merge(value.span);
            args.push(CallArg {
                name,
                value,
                span: arg_span,
                is_trailing_lambda: false,
                is_spread,
            });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(args)
    }

    fn parse_trailing_lambda_call(
        &mut self,
        left: Expr,
        type_arguments: Vec<Type>,
    ) -> Result<Expr, ParseError> {
        let lambda = self.parse_closure()?;
        let span = left.span.merge(lambda.span);
        Ok(Expr::new(
            ExprKind::Call {
                callee: Box::new(left),
                type_arguments,
                args: vec![CallArg {
                    name: None,
                    span: lambda.span,
                    value: lambda,
                    is_trailing_lambda: true,
                    is_spread: false,
                }],
            },
            span,
        ))
    }

    /// An index `target[i]` or index list `target[i, j]`, with an optional trailing comma (D9 of
    /// `operator-conventions`). `target[]` is rejected.
    fn parse_index(&mut self, left: Expr) -> Result<Expr, ParseError> {
        self.expect(TokenKind::LBracket)?;
        if self.peek_kind() == &TokenKind::RBracket {
            return Err(ParseError::new(
                "An index expression needs at least one index, as in 'm[0]' or 'm[row, column]'",
                self.peek().span,
            ));
        }
        let indices = self.with_newlines(false, |p| {
            let mut indices = vec![p.parse_expr(Precedence::Lowest)?];
            while p.peek_kind() == &TokenKind::Comma {
                p.bump();
                if p.peek_kind() == &TokenKind::RBracket {
                    break;
                }
                indices.push(p.parse_expr(Precedence::Lowest)?);
            }
            Ok(indices)
        })?;
        let end = self.expect(TokenKind::RBracket)?.span;
        let span = left.span.merge(end);
        Ok(Expr::new(
            ExprKind::Index {
                target: Box::new(left),
                indices,
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
        // Comparisons are non-associative: `a < b < c` never means anything, since `Bool` is
        // not ordered.
        if prec == Precedence::Comparison && self.current_precedence() == Precedence::Comparison {
            return Err(ParseError::new(
                "Comparisons cannot be chained; join them with '&&', as in 'a < b && b < c'",
                self.peek().span,
            ));
        }
        // Membership tests are non-associative too: `a in b in c` would test a `Bool` for
        // membership (D8 of `operator-conventions`).
        if prec == Precedence::Membership && self.current_precedence() == Precedence::Membership {
            return Err(ParseError::new(
                "Membership tests cannot be chained; parenthesize one, as in '(a in b) in c', or join them with '&&'",
                self.peek().span,
            ));
        }
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
            TokenKind::In => Some(BinaryOp::In),
            TokenKind::NotIn => Some(BinaryOp::NotIn),
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
            close_brace: end,
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
        if !is_assignment_operator(self.peek_kind()) || self.at_line_break() {
            if is_write_through {
                return Err(ParseError::new(DEREF_OUTSIDE_TARGET, start_span));
            }
            let span = target.span;
            return Ok(Statement::new(StatementKind::Expression(target), span));
        }
        let operator = compound_operator(&self.bump().kind); // '=' or 'op='
        let value = self.parse_expr(Precedence::Assign)?;
        if self.at_assignment_operator() {
            return Err(ParseError::new(
                "Chained assignments are not allowed; use separate statements",
                self.peek().span,
            ));
        }
        let is_place = match &target.kind {
            ExprKind::Ident(_)
            | ExprKind::This
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
        let kind = match operator {
            Some(operator) => StatementKind::CompoundAssign {
                operator,
                target,
                value,
            },
            None => StatementKind::Assign { target, value },
        };
        Ok(Statement::new(kind, span))
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
                        close_brace: span,
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
                close_brace: span,
            },
            kind => Block {
                statements: vec![Statement::new(kind, span)],
                tail: None,
                span,
                close_brace: span,
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
        let has_subject = target.is_some();
        let branches = self.with_newlines(true, |p| p.parse_when_branches(has_subject))?;
        let end = self.expect(TokenKind::RBrace)?.span;

        Ok(Expr::new(
            ExprKind::When { target, branches },
            start.merge(end),
        ))
    }

    /// `when` branches up to (not including) the closing `}`. `has_subject` tells whether
    /// branches hold patterns or, without a subject, conditions.
    fn parse_when_branches(&mut self, has_subject: bool) -> Result<Vec<WhenBranch>, ParseError> {
        let mut branches = Vec::new();
        while self.peek_kind() != &TokenKind::RBrace && !self.is_eof() {
            branches.push(self.parse_when_branch(has_subject)?);

            if self.peek_kind() == &TokenKind::Comma || self.peek_kind() == &TokenKind::Semicolon {
                self.bump();
            } else {
                self.expect_separated("'when' branches", "a newline, ',' or ';'")?;
            }
        }
        Ok(branches)
    }

    fn parse_when_branch(&mut self, has_subject: bool) -> Result<WhenBranch, ParseError> {
        let mut patterns = Vec::new();
        let b_start = self.peek().span;

        if self.peek_kind() == &TokenKind::Else {
            self.bump();
            patterns.push(WhenPattern::Else);
            if self.peek_kind() == &TokenKind::If {
                return Err(ParseError::new(
                    "An 'else' branch cannot have a guard; write the condition as a pattern or a guard of another branch",
                    self.peek().span,
                ));
            }
        } else {
            while self.peek_kind() != &TokenKind::Arrow && !self.is_eof() {
                let pattern = if has_subject {
                    self.parse_pattern(false)?
                } else {
                    self.parse_condition_pattern()?
                };
                patterns.push(WhenPattern::Pattern(pattern));
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

        let guard = if self.peek_kind() == &TokenKind::If {
            if !has_subject {
                return Err(ParseError::new(NO_SUBJECT_PATTERN, self.peek().span));
            }
            self.bump();
            Some(self.parse_expr(Precedence::Lowest)?)
        } else {
            None
        };

        self.expect(TokenKind::Arrow)?;
        let body = self.parse_branch_body()?;
        let b_end = body.span;

        Ok(WhenBranch {
            patterns,
            guard,
            body,
            span: b_start.merge(b_end),
        })
    }

    /// A branch pattern of a `when` without a subject: a condition, or a range (which the type
    /// checker rejects). `_`, bindings and guards need a subject (D2 of `payload-patterns`).
    fn parse_condition_pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.peek_kind() {
            TokenKind::In => self.parse_range_pattern(),
            TokenKind::Val | TokenKind::Var => {
                Err(ParseError::new(NO_SUBJECT_PATTERN, self.peek().span))
            }
            TokenKind::Ident(name) if name == "_" => {
                Err(ParseError::new(NO_SUBJECT_PATTERN, self.peek().span))
            }
            _ => {
                let expr = self.parse_expr(Precedence::Lowest)?;
                let span = expr.span;
                Ok(Pattern::new(PatternKind::Value(expr), span))
            }
        }
    }

    /// A pattern (D1 to D3 of `payload-patterns`): `_`, a binding `val x` or `var x`, a range
    /// `in a..b`, a constructor pattern `Name.Variant(p, ...)`, or any value expression. A nested
    /// pattern is a sub-pattern of a constructor pattern.
    fn parse_pattern(&mut self, nested: bool) -> Result<Pattern, ParseError> {
        let outer_peak = self.enter()?;
        let result = self.parse_pattern_inner(nested);
        self.exit(outer_peak);
        result
    }

    fn parse_pattern_inner(&mut self, nested: bool) -> Result<Pattern, ParseError> {
        let token = self.peek().clone();
        match &token.kind {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                return Ok(Pattern::new(PatternKind::Wildcard, token.span));
            }
            TokenKind::Val | TokenKind::Var => {
                let is_var = token.kind == TokenKind::Var;
                self.bump();
                let (name, name_span) = self.expect_ident()?;
                return Ok(Pattern::new(
                    PatternKind::Binding { is_var, name },
                    token.span.merge(name_span),
                ));
            }
            TokenKind::In => return self.parse_range_pattern(),
            TokenKind::Ident(_) | TokenKind::Compose if self.is_constructor_pattern_ahead() => {
                if let Some(pattern) = self.speculate(|p| p.parse_constructor_pattern(nested)) {
                    return Ok(pattern);
                }
            }
            _ => {}
        }
        let expr = self.parse_expr(Precedence::Lowest)?;
        let span = expr.span;
        Ok(Pattern::new(PatternKind::Value(expr), span))
    }

    /// Whether a dotted name followed by `(` on the same line starts here (D2).
    fn is_constructor_pattern_ahead(&self) -> bool {
        let mut index = self.cursor;
        loop {
            match &self.tokens[index.min(self.tokens.len() - 1)].kind {
                TokenKind::Ident(_) | TokenKind::Compose => index += 1,
                _ => return false,
            }
            let next = &self.tokens[index.min(self.tokens.len() - 1)];
            match next.kind {
                TokenKind::Dot => index += 1,
                TokenKind::LParen => return !next.is_preceded_by_newline,
                _ => return false,
            }
        }
    }

    /// A constructor pattern `Name.Variant(p, name = p, ...)`, kept only when a pattern
    /// terminator follows its `)`: `,`, `->` or `if` at the top of a branch, `,` or `)` nested.
    /// Otherwise it is an error, so that [`Self::speculate`] restores the stream and the
    /// pattern is parsed as a value expression (`g(1) + 1`, `f(x).y`).
    fn parse_constructor_pattern(&mut self, nested: bool) -> Result<Pattern, ParseError> {
        let (name, name_span) = self.expect_ident()?;
        let mut callee = Expr::new(ExprKind::Ident(name), name_span);
        while self.peek_kind() == &TokenKind::Dot {
            self.bump();
            let (member, member_span) = self.expect_ident()?;
            let span = callee.span.merge(member_span);
            callee = Expr::new(
                ExprKind::MemberAccess {
                    target: Box::new(callee),
                    member,
                    is_safe_nav: false,
                },
                span,
            );
        }
        self.expect(TokenKind::LParen)?;
        let arguments = self.with_newlines(false, Self::parse_pattern_arguments)?;
        let end = self.expect(TokenKind::RParen)?.span;
        let terminated = if nested {
            matches!(self.peek_kind(), TokenKind::Comma | TokenKind::RParen)
        } else {
            matches!(
                self.peek_kind(),
                TokenKind::Comma | TokenKind::Arrow | TokenKind::If
            )
        };
        if !terminated {
            return Err(ParseError::new(
                "not a constructor pattern",
                self.peek().span,
            ));
        }
        let span = callee.span.merge(end);
        Ok(Pattern::new(
            PatternKind::Constructor { callee, arguments },
            span,
        ))
    }

    /// The sub-patterns of a constructor pattern up to (not including) the closing `)`, each
    /// optionally named (`y = val y`), with an optional trailing comma.
    fn parse_pattern_arguments(&mut self) -> Result<Vec<PatternArgument>, ParseError> {
        let mut arguments = Vec::new();
        while self.peek_kind() != &TokenKind::RParen && !self.is_eof() {
            let mut name = None;
            let mut start = self.peek().span;
            if matches!(self.peek_kind(), TokenKind::Ident(_) | TokenKind::Compose)
                && self.peek_ahead(1) == &TokenKind::Eq
            {
                let (n, n_span) = self.expect_ident()?;
                self.bump(); // '='
                name = Some(n);
                start = n_span;
            }
            let pattern = self.parse_pattern(true)?;
            let span = start.merge(pattern.span);
            arguments.push(PatternArgument {
                name,
                pattern,
                span,
            });
            if self.peek_kind() == &TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(arguments)
    }

    /// `in start..end` or `in start..<end`.
    fn parse_range_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start_span = self.expect(TokenKind::In)?.span;
        let range_expr = self.parse_expr(Precedence::Lowest)?;
        let span = start_span.merge(range_expr.span);
        match range_expr.kind {
            ExprKind::Binary {
                op: op @ (BinaryOp::Range | BinaryOp::HalfOpenRange),
                left,
                right,
            } => Ok(Pattern::new(
                PatternKind::InRange {
                    start: *left,
                    end: *right,
                    end_inclusive: op == BinaryOp::Range,
                },
                span,
            )),
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

        if self.peek_kind() == &TokenKind::LParen {
            return self.parse_destructure(is_var, start);
        }

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

    /// The rest of a destructuring declaration `val (field, field as name, ...) = value`, from
    /// its `(` (D13 of `payload-patterns`). `as` is a keyword only inside the list.
    fn parse_destructure(&mut self, is_var: bool, start: Span) -> Result<Statement, ParseError> {
        let open = self.expect(TokenKind::LParen)?.span;
        let entries = self.with_newlines(false, |p| {
            let mut entries = Vec::new();
            while p.peek_kind() != &TokenKind::RParen && !p.is_eof() {
                let (field, field_span) = p.destructure_name()?;
                let (name, name_span) = if p.peek_kind() == &TokenKind::Ident("as".to_string())
                {
                    p.bump();
                    p.destructure_name()?
                } else {
                    (field.clone(), field_span)
                };
                entries.push(DestructureEntry {
                    field,
                    field_span,
                    name,
                    name_span,
                });
                match p.peek_kind() {
                    TokenKind::Comma => {
                        p.bump();
                    }
                    TokenKind::RParen => break,
                    _ => {
                        return Err(ParseError::new(
                            format!(
                                "Expected ',' or ')' after a destructuring entry, found '{}'; an entry is a field name, optionally followed by 'as' and a name",
                                p.peek_kind()
                            ),
                            p.peek().span,
                        ))
                    }
                }
            }
            Ok(entries)
        })?;
        let close = self.expect(TokenKind::RParen)?.span;
        if entries.is_empty() {
            return Err(ParseError::new(
                "A destructuring declaration needs at least one field name",
                open.merge(close),
            ));
        }
        if self.peek_kind() == &TokenKind::Colon {
            return Err(ParseError::new(
                "A destructuring declaration takes no type annotation; its locals have the types of the fields they bind",
                self.peek().span,
            ));
        }
        self.expect(TokenKind::Eq)?;
        let initializer = self.parse_initializer()?;
        let span = start.merge(initializer.span);
        Ok(Statement::new(
            StatementKind::Destructure {
                is_var,
                entries,
                initializer,
            },
            span,
        ))
    }

    /// A field or local name of a destructuring entry: an identifier other than `_` and `as`.
    fn destructure_name(&mut self) -> Result<(String, Span), ParseError> {
        let token = self.peek().clone();
        match &token.kind {
            TokenKind::Ident(name) if name == "_" => Err(ParseError::new(
                "A destructuring entry names a field; '_' is not needed, since unnamed fields are not bound",
                token.span,
            )),
            TokenKind::Ident(name) if name == "as" => Err(ParseError::new(
                "Expected a field name before 'as' in a destructuring entry",
                token.span,
            )),
            TokenKind::Ident(_) | TokenKind::Compose => self.expect_ident(),
            _ => Err(ParseError::new(
                format!(
                    "Expected a field name in a destructuring declaration, found '{}'",
                    token.kind
                ),
                token.span,
            )),
        }
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
        if self.at_assignment_operator() {
            return Err(ParseError::new(
                "An assignment is a statement, not an expression; it cannot be used as a value",
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
