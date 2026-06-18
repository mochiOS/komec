pub mod error;
pub mod lexer;
pub mod token;

pub use error::{LexError, LexErrorKind};
pub use lexer::Lexer;
pub use token::{Token, TokenKind};

pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).tokenize()
}