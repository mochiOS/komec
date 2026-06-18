pub mod error;
pub mod lexer;
pub mod parser;
pub mod token;

pub use error::{
    FrontendError, LexError, LexErrorKind, ParseError,
    ParseErrorKind,
};
pub use lexer::Lexer;
pub use parser::Parser;
pub use token::{Token, TokenKind};

use kome_ast::declarations::Module;

pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).tokenize()
}

pub fn parse(source: &str) -> Result<Module, FrontendError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser::new(tokens);

    Ok(parser.parse_module()?)
}