//! Statements: blocks, control flow, `is` pattern-match, etc.

use crate::{AstNode, Span};

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Block(BlockStatement),
    Expression(ExpressionStatement),
    Let(crate::declarations::Binding),
    If(IfStatement),
    While(WhileStatement),
    ForIn(ForInStatement),
    Return(ReturnStatement),
    Break(BreakStatement),
    Continue(ContinueStatement),
    Empty(Span),
    Is(IsStatement),
    Declaration(crate::declarations::Declaration),
}

// ---- Block ----

/// A braced block of statements.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStatement {
    pub span: Span,
    pub statements: Vec<Statement>,
}

// ---- Expression ----

/// An expression used as a statement (result discarded).
#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionStatement {
    pub span: Span,
    pub expression: crate::expressions::Expression,
}

// ---- If ----

/// `if` / `else`.
#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub span: Span,
    pub test: crate::expressions::Expression,
    pub consequent: Box<Statement>,
    pub alternative: Option<Box<Statement>>,
}

// ---- While ----

/// `while` loop.
#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    pub span: Span,
    pub test: crate::expressions::Expression,
    pub body: Box<Statement>,
}

// ---- ForIn ----

/// `for item in iter { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForInStatement {
    pub span: Span,
    pub pattern: crate::patterns::Pattern,
    pub right: crate::expressions::Expression,
    pub body: Box<Statement>,
}

// ---- Return ----

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStatement {
    pub span: Span,
    pub argument: Option<crate::expressions::Expression>,
}

// ---- Break / Continue ----

#[derive(Debug, Clone, PartialEq)]
pub struct BreakStatement {
    pub span: Span,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinueStatement {
    pub span: Span,
    pub label: Option<String>,
}

// ---- Is (single-arm pattern match) ----

/// A single-arm `is` pattern-matching statement.
///
/// ```kome
/// is input.event .entered => { handle() }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct IsStatement {
    pub span: Span,
    /// The value being matched; `None` for implicit event matching.
    pub value: Option<crate::expressions::Expression>,
    /// The pattern to match against.
    pub pattern: crate::patterns::IsPattern,
    /// The body executed when the pattern matches.
    pub body: Box<Statement>,
}

// ---- AstNode implementation ----

impl AstNode for Statement {
    fn span(&self) -> Span {
        match self {
            Statement::Block(s) => s.span,
            Statement::Expression(s) => s.span,
            Statement::Let(s) => s.span,
            Statement::If(s) => s.span,
            Statement::While(s) => s.span,
            Statement::ForIn(s) => s.span,
            Statement::Return(s) => s.span,
            Statement::Break(s) => s.span,
            Statement::Continue(s) => s.span,
            Statement::Is(s) => s.span,
            Statement::Empty(s) => *s,
            Statement::Declaration(d) => d.span(),
        }
    }
}
