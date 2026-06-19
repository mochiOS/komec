pub mod error;
pub mod lexer;
pub mod parser;
pub mod token;

pub use error::{FrontendError, LexError, LexErrorKind, ParseError, ParseErrorKind};
pub use lexer::Lexer;
pub use parser::Parser;
pub use token::{Token, TokenKind};

use kome_ast::{declarations::Module, expressions::Expression};

/// Tokenizes an entire Kome source file.
pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).tokenize()
}

/// Tokenizes and parses an entire Kome source file.
pub fn parse(source: &str) -> Result<Module, FrontendError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);

    Ok(parser.parse_module()?)
}

/// Tokenizes and parses one complete expression.
pub fn parse_expression(source: &str) -> Result<Expression, FrontendError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);

    Ok(parser.parse_expression()?)
}
