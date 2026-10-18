//! `prism-lexer`: Fast streaming tokenizer for the Prism language.

pub mod lexer;
pub mod token;

pub use lexer::Lexer;
pub use token::{Comment, CommentKind, Token, TokenKind};
