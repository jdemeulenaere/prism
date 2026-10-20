//! Token representations for Prism.

use prism_ast::Span;
use std::fmt;

/// The lexical kind of a token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Package,
    Import,
    Fun,
    Val,
    Var,
    Struct,
    Enum,
    Compose,
    Public,
    Private,
    Return,
    If,
    Else,
    When,
    In,
    While,
    Do,
    For,
    Break,
    Continue,
    This,
    Mut,

    // Literals
    /// An integer literal's magnitude. It is at most `i64::MAX`, except that
    /// a decimal `9223372036854775808` is kept so the parser can read
    /// `-9223372036854775808` as `i64::MIN`.
    Int(u64),
    Float(f64),
    String(String),
    Char(char),
    True,
    False,
    Null,

    // Identifiers
    Ident(String),

    // Operators & Punctuation
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    Percent,      // %
    PlusEqual,    // +=
    MinusEqual,   // -=
    StarEqual,    // *=
    SlashEqual,   // /=
    PercentEqual, // %=
    /// `++`, reserved: Prism has no increment operator, and the parser rejects it.
    PlusPlus,
    /// `--`, reserved: Prism has no decrement operator, and the parser rejects it.
    MinusMinus,
    Eq,       // =
    EqEq,     // ==
    BangEq,   // !=
    Lt,       // <
    LtEq,     // <=
    Gt,       // >
    GtEq,     // >=
    AmpAmp,   // &&
    PipePipe, // ||
    Bang,     // !
    /// `!in`, the negated membership operator, lexed as one token when `in` is not the start of
    /// a longer name.
    NotIn,
    Amp,           // &
    Dot,           // .
    DotDot,        // ..
    DotDotLess,    // ..<
    QuestionDot,   // ?.
    QuestionColon, // ?:
    Question,      // ?
    Arrow,         // ->
    Colon,         // :
    Semicolon,     // ;
    Comma,         // ,
    LParen,        // (
    RParen,        // )
    LBrace,        // {
    RBrace,        // }
    LBracket,      // [
    RBracket,      // ]

    // String templates (D3 of `string-templates`): a literal with a `$name` or `${...}` part is
    // a flat run of these tokens, with the ordinary tokens of each expression part between its
    // `TemplateExpressionStart` and `TemplateExpressionEnd`.
    /// The opening quote of a string template.
    TemplateStart,
    /// A run of a template's text between its quotes and parts, escapes decoded; never empty.
    TemplateText(String),
    /// A `$name` part, holding the name (an identifier or `this`); its span covers the `$`.
    TemplateName(String),
    /// The `${` that opens an expression part.
    TemplateExpressionStart,
    /// The `}` that closes an expression part.
    TemplateExpressionEnd,
    /// The closing quote of a string template.
    TemplateEnd,

    // Special
    Eof,
    Error(String),
}

impl TokenKind {
    pub fn keyword_from_str(s: &str) -> Option<TokenKind> {
        match s {
            "package" => Some(TokenKind::Package),
            "import" => Some(TokenKind::Import),
            "fun" => Some(TokenKind::Fun),
            "val" => Some(TokenKind::Val),
            "var" => Some(TokenKind::Var),
            "struct" => Some(TokenKind::Struct),
            "enum" => Some(TokenKind::Enum),
            "compose" => Some(TokenKind::Compose),
            "public" => Some(TokenKind::Public),
            "private" => Some(TokenKind::Private),
            "return" => Some(TokenKind::Return),
            "if" => Some(TokenKind::If),
            "else" => Some(TokenKind::Else),
            "when" => Some(TokenKind::When),
            "in" => Some(TokenKind::In),
            "while" => Some(TokenKind::While),
            "do" => Some(TokenKind::Do),
            "for" => Some(TokenKind::For),
            "break" => Some(TokenKind::Break),
            "continue" => Some(TokenKind::Continue),
            "this" => Some(TokenKind::This),
            "mut" => Some(TokenKind::Mut),
            "true" => Some(TokenKind::True),
            "false" => Some(TokenKind::False),
            "null" => Some(TokenKind::Null),
            _ => None,
        }
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Package => write!(f, "package"),
            TokenKind::Import => write!(f, "import"),
            TokenKind::Fun => write!(f, "fun"),
            TokenKind::Val => write!(f, "val"),
            TokenKind::Var => write!(f, "var"),
            TokenKind::Struct => write!(f, "struct"),
            TokenKind::Enum => write!(f, "enum"),
            TokenKind::Compose => write!(f, "compose"),
            TokenKind::Public => write!(f, "public"),
            TokenKind::Private => write!(f, "private"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::When => write!(f, "when"),
            TokenKind::In => write!(f, "in"),
            TokenKind::While => write!(f, "while"),
            TokenKind::Do => write!(f, "do"),
            TokenKind::For => write!(f, "for"),
            TokenKind::Break => write!(f, "break"),
            TokenKind::Continue => write!(f, "continue"),
            TokenKind::This => write!(f, "this"),
            TokenKind::Mut => write!(f, "mut"),
            TokenKind::Int(v) => write!(f, "{}", v),
            TokenKind::Float(v) => write!(f, "{}", v),
            TokenKind::String(v) => write!(f, "\"{}\"", v),
            TokenKind::Char(v) => write!(f, "'{}'", v),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Null => write!(f, "null"),
            TokenKind::Ident(s) => write!(f, "{}", s),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::PlusEqual => write!(f, "+="),
            TokenKind::MinusEqual => write!(f, "-="),
            TokenKind::StarEqual => write!(f, "*="),
            TokenKind::SlashEqual => write!(f, "/="),
            TokenKind::PercentEqual => write!(f, "%="),
            TokenKind::PlusPlus => write!(f, "++"),
            TokenKind::MinusMinus => write!(f, "--"),
            TokenKind::Eq => write!(f, "="),
            TokenKind::EqEq => write!(f, "=="),
            TokenKind::BangEq => write!(f, "!="),
            TokenKind::Lt => write!(f, "<"),
            TokenKind::LtEq => write!(f, "<="),
            TokenKind::Gt => write!(f, ">"),
            TokenKind::GtEq => write!(f, ">="),
            TokenKind::AmpAmp => write!(f, "&&"),
            TokenKind::PipePipe => write!(f, "||"),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::NotIn => write!(f, "!in"),
            TokenKind::Amp => write!(f, "&"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::DotDot => write!(f, ".."),
            TokenKind::DotDotLess => write!(f, "..<"),
            TokenKind::QuestionDot => write!(f, "?."),
            TokenKind::QuestionColon => write!(f, "?:"),
            TokenKind::Question => write!(f, "?"),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Semicolon => write!(f, ";"),
            TokenKind::Comma => write!(f, ","),
            TokenKind::LParen => write!(f, "("),
            TokenKind::RParen => write!(f, ")"),
            TokenKind::LBrace => write!(f, "{{"),
            TokenKind::RBrace => write!(f, "}}"),
            TokenKind::LBracket => write!(f, "["),
            TokenKind::RBracket => write!(f, "]"),
            TokenKind::TemplateStart | TokenKind::TemplateEnd => write!(f, "\""),
            TokenKind::TemplateText(text) => write!(f, "{}", text),
            TokenKind::TemplateName(name) => write!(f, "${}", name),
            TokenKind::TemplateExpressionStart => write!(f, "${{"),
            TokenKind::TemplateExpressionEnd => write!(f, "}}"),
            TokenKind::Eof => write!(f, "<EOF>"),
            TokenKind::Error(err) => write!(f, "<Error: {}>", err),
        }
    }
}

/// A token combined with its location in source code.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub is_preceded_by_newline: bool,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self {
            kind,
            span,
            is_preceded_by_newline: false,
        }
    }

    pub fn with_newline(kind: TokenKind, span: Span, is_preceded_by_newline: bool) -> Self {
        Self {
            kind,
            span,
            is_preceded_by_newline,
        }
    }
}

/// Whether a comment is a `//` line comment or a `/* */` block comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// `// ...`, up to (not including) the newline that ends it.
    Line,
    /// `/* ... */`, including any nested block comments.
    Block,
}

/// A comment skipped by the lexer, reported only in the comment-collecting mode
/// ([`crate::Lexer::tokenize_with_comments`]). Its text is the source slice of its span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    pub span: Span,
}
