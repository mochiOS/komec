use kome_ast::Span;
use std::fmt;

#[derive(Debug, Clone)]
pub enum ResolutionError {
    UndefinedName {
        name: String,
        span: Span,
    },
    DuplicateDefinition {
        name: String,
        first: Span,
        second: Span,
    },
    AssignmentToImmutable {
        name: String,
        span: Span,
    },
    ScopeStackEmpty,
    InvalidLetLocation {
        span: Span,
    },
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedName { name, span } => {
                write!(
                    formatter,
                    "undefined name `{name}` at byte range {}..{}",
                    span.start, span.end,
                )
            }

            Self::DuplicateDefinition {
                name,
                first,
                second,
            } => {
                write!(
                    formatter,
                    "duplicate definition of `{name}` at byte range {}..{}; \
                     first defined at byte range {}..{}",
                    second.start, second.end, first.start, first.end,
                )
            }

            Self::AssignmentToImmutable { name, span } => {
                write!(
                    formatter,
                    "cannot assign to immutable variable `{name}` at byte range {}..{}",
                    span.start, span.end,
                )
            }

            Self::ScopeStackEmpty => {
                write!(formatter, "internal error: scope stack is empty")
            }

            Self::InvalidLetLocation { span } => {
                write!(
                    formatter,
                    "`let` is not allowed here at byte range {}..{}",
                    span.start, span.end,
                )
            }
        }
    }
}

impl std::error::Error for ResolutionError {}
