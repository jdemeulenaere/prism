//! `prism-formatter`: the canonical layout of Prism source, behind `prism fmt`.
//!
//! [`format_source`] parses a file, prints its syntax tree through the document algebra of
//! [`document`] at a width of 100 characters, and checks that the output parses to the same
//! tree, ignoring positions, with the same comments ("Formatting Preserves Meaning" in
//! `openspec/specs/formatting`). Formatting depends only on the file's own text, per the
//! single-file compilation unit invariant: no import is resolved.

pub mod document;
pub mod files;
mod printer;
mod verify;

use prism_lexer::{Comment, Lexer, Token};
use prism_parser::{ParseError, Parser};
use std::fmt;

/// Why a source could not be formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The source does not parse (`E2001` or `E2002`); nothing is formatted.
    Syntax(ParseError),
    /// The formatter's output would not keep the source's meaning or comments. This is a bug in
    /// the formatter, not in the source, so it carries no diagnostic code.
    Internal(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(err) => write!(f, "{}", err),
            FormatError::Internal(reason) => write!(f, "internal formatter error: {}", reason),
        }
    }
}

impl std::error::Error for FormatError {}

/// A lexed and parsed source, with its comments.
struct Parsed {
    tokens: Vec<Token>,
    comments: Vec<Comment>,
    file: prism_ast::SourceFile,
}

fn parse(source: &str) -> Result<Parsed, ParseError> {
    let (tokens, comments) = Lexer::new(source).tokenize_with_comments();
    let file = Parser::new(tokens.clone()).parse_source_file()?;
    Ok(Parsed {
        tokens,
        comments,
        file,
    })
}

/// Formats `source` with the canonical layout. Line endings are normalized to `\n`, a leading
/// byte order mark is dropped, and a non-empty result ends with exactly one newline.
pub fn format_source(source: &str) -> Result<String, FormatError> {
    let source = source
        .strip_prefix('\u{FEFF}')
        .unwrap_or(source)
        .replace("\r\n", "\n");
    let input = parse(&source).map_err(FormatError::Syntax)?;
    let output = printer::print(&source, &input.tokens, &input.comments, &input.file);
    let reparsed = parse(&output).map_err(|err| {
        FormatError::Internal(format!(
            "the formatted source does not parse ({}); the input was left unchanged",
            err
        ))
    })?;
    verify::same_meaning(&source, &input, &output, &reparsed).map_err(FormatError::Internal)?;
    Ok(output)
}
