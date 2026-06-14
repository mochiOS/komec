//! Patterns: identifier, literal, dot-ident.

use crate::{AstNode, Span};

/// A destructuring pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(LiteralPattern),
    Ident(IdentifierPattern),
}

/// A pattern for `is` expressions/statements.
#[derive(Debug, Clone, PartialEq)]
pub enum IsPattern {
    Literal(LiteralPattern),
    Ident(IdentifierPattern),
    DotIdent(DotIdentPattern),
}

/// Matches a literal value.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralPattern {
    pub span: Span,
    pub value: crate::expressions::LiteralKind,
}

/// `name` or `name: Type`.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentifierPattern {
    pub span: Span,
    pub name: String,
    pub type_annotation: Option<crate::types::Type>,
}

/// `.name` pattern for `is` matching.
#[derive(Debug, Clone, PartialEq)]
pub struct DotIdentPattern {
    pub span: Span,
    pub name: String,
}

impl AstNode for Pattern {
    fn span(&self) -> Span {
        match self {
            Pattern::Literal(p) => p.span,
            Pattern::Ident(p) => p.span,
        }
    }
}

impl AstNode for IsPattern {
    fn span(&self) -> Span {
        match self {
            IsPattern::Literal(p) => p.span,
            IsPattern::Ident(p) => p.span,
            IsPattern::DotIdent(p) => p.span,
        }
    }
}
