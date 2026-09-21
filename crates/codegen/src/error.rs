//! Errors produced while compiling Kome modules to native code.

use kome_ast::Span;
use std::fmt;

/// A compilation failure with optional source location.
#[derive(Debug, Clone)]
pub struct CodegenError {
    message: String,
    span: Option<Span>,
}

impl CodegenError {
    pub fn new(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn at(message: impl Into<String>, span: Span) -> Self {
        Self::new(message, Some(span))
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> Option<Span> {
        self.span
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)?;

        if let Some(span) = self.span {
            write!(formatter, " at byte range {}..{}", span.start, span.end)?;
        }

        Ok(())
    }
}

impl std::error::Error for CodegenError {}

pub type CodegenResult<T> = Result<T, CodegenError>;
