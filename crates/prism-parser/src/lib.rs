//! `prism-parser`: Pratt expression parser and recursive-descent AST generator for Prism.

pub mod error;
pub mod parser;

pub use error::ParseError;
pub use parser::{Parser, Precedence, MAX_NESTING_DEPTH};

/// Convenience function to parse a source string into a [`prism_ast::SourceFile`].
pub fn parse_source(source: &str) -> Result<prism_ast::SourceFile, ParseError> {
    let mut parser = Parser::from_source(source);
    parser.parse_source_file()
}
