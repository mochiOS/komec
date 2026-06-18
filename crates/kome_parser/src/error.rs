use std::fmt;

use kome_ast::Span;

/// An error produced while tokenizing Kome source code.
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub kind: LexErrorKind,
    pub span: Span,
}

impl LexError {
    pub const fn new(kind: LexErrorKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The reason tokenization failed.
#[derive(Debug, Clone, PartialEq)]
pub enum LexErrorKind {
    UnexpectedCharacter(char),
    UnterminatedString,
    InvalidEscape(char),
}

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            LexErrorKind::UnexpectedCharacter(character) => write!(
                formatter,
                "unexpected character {character:?} at byte range {}..{}",
                self.span.start,
                self.span.end,
            ),

            LexErrorKind::UnterminatedString => write!(
                formatter,
                "unterminated string literal at byte range {}..{}",
                self.span.start,
                self.span.end,
            ),

            LexErrorKind::InvalidEscape(character) => write!(
                formatter,
                "invalid escape sequence \\{character} at byte range {}..{}",
                self.span.start,
                self.span.end,
            ),
        }
    }
}

impl std::error::Error for LexError {}