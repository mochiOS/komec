use std::fmt;

use kome_ast::Span;

use crate::token::TokenKind;

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

    /// A template interpolation was opened with `{`
    /// but no matching `}` was found.
    UnterminatedInterpolation,

    InvalidEscape(char),
}

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            LexErrorKind::UnexpectedCharacter(character) => {
                write!(
                    formatter,
                    "unexpected character {character:?} at byte range {}..{}",
                    self.span.start, self.span.end,
                )
            }

            LexErrorKind::UnterminatedString => {
                write!(
                    formatter,
                    "unterminated string literal at byte range {}..{}",
                    self.span.start, self.span.end,
                )
            }

            LexErrorKind::UnterminatedInterpolation => {
                write!(
                    formatter,
                    "unterminated template interpolation at byte range {}..{}",
                    self.span.start, self.span.end,
                )
            }

            LexErrorKind::InvalidEscape(character) => {
                write!(
                    formatter,
                    "invalid escape sequence \\{character} at byte range {}..{}",
                    self.span.start, self.span.end,
                )
            }
        }
    }
}

impl std::error::Error for LexError {}

/// An error produced while parsing Kome tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub span: Span,
}

impl ParseError {
    pub const fn new(kind: ParseErrorKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// The reason parsing failed.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseErrorKind {
    Expected {
        expected: &'static str,
        found: TokenKind,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::Expected { expected, found } => {
                write!(
                    formatter,
                    "expected {expected}, found {found:?} at byte range {}..{}",
                    self.span.start, self.span.end,
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// An error produced while tokenizing or parsing a source file.
#[derive(Debug)]
pub enum FrontendError {
    Lex(LexError),
    Parse(ParseError),
}

impl fmt::Display for FrontendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FrontendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lex(error) => Some(error),
            Self::Parse(error) => Some(error),
        }
    }
}

impl From<LexError> for FrontendError {
    fn from(error: LexError) -> Self {
        Self::Lex(error)
    }
}

impl From<ParseError> for FrontendError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}
