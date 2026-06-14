//! Expressions: literals, operators, calls, closures, component-blocks.

use crate::{AstNode, Span};

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// A literal value: `42`, `"hello"`, `true`, etc.
    Literal(LiteralExpression),
    /// An identifier: `foo`, `Bar`, `x`, etc.
    Ident(IdentifierExpression),
    /// A unary operator expression: `!expr`.
    Unary(UnaryExpression),
    /// A binary operator expression: `a + b`, `x == y`, `a && b`, etc.
    Binary(BinaryExpression),
    /// A function or method call: `foo()`, `obj.method(x, y: z)`.
    Call(CallExpression),
    /// Property access: `object.property`.
    Member(MemberExpression),
    /// Index access: `object[index]`.
    Index(IndexExpression),
    /// An assignment expression: `target = value`, `x += 1`.
    Assign(AssignmentExpression),
    /// A parenthesized group expression: `(expression)`.
    Group(GroupExpression),
    /// An List literal: `[a, b, c]`.
    List(ListExpression),
    /// An object literal: `{ key: value }`.
    Object(ObjectExpression),
    /// A template string with interpolation: `"hello {name}"`.
    Template(TemplateExpression),
    /// A closure with pipe parameters: `|x| expression`.
    Closure(ClosureExpression),
    /// A dot-prefixed type-associated value: `.blue`, `.entered`.
    DotIdent(DotIdentifierExpression),
    /// An inline `is` expression: `is x 1 => "one"`.
    Is(IsExpression),
    /// A component call with block children: `VStack { Text("Hi"), Button("Click") }`.
    Component(ComponentExpression),
}

// ---- Literal ----

/// A numeric literal value stored as a string.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberLiteral(pub String);

/// The kind of a literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralKind {
    String(String),
    Number(NumberLiteral),
    Boolean(bool),
    Null,
    /// `50%`, `100%`, etc.
    Percent(NumberLiteral),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiteralExpression {
    pub span: Span,
    pub kind: LiteralKind,
}

// ---- Ident ----

#[derive(Debug, Clone, PartialEq)]
pub struct IdentifierExpression {
    pub span: Span,
    pub name: String,
}

// ---- Unary ----

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpression {
    pub span: Span,
    pub op: UnaryOp,
    pub argument: Box<Expression>,
}

// ---- Binary ----

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpression {
    pub span: Span,
    pub op: BinaryOp,
    pub left: Box<Expression>,
    pub right: Box<Expression>,
}

// ---- CallArg ----

/// A call argument: positional or named.
///
/// ```kome
/// Button("+", color: .blue)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Positional(Expression),
    Named {
        name: String,
        value: Box<Expression>,
        span: Span,
    },
}

// ---- Call ----

/// A function call: `foo()`, `obj.method(x, y: z)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub span: Span,
    pub callee: Box<Expression>,
    pub args: Vec<CallArg>,
}

// ---- Component (block children) ----

/// A component call with block children.
///
/// ```kome
/// VStack { Text("Hi"), Button("Click") }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentExpression {
    pub span: Span,
    pub name: String,
    pub args: Vec<CallArg>,
    pub children: Vec<Expression>,
}

// ---- Member ----

/// Property access: `object.property`.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberExpression {
    pub span: Span,
    pub object: Box<Expression>,
    pub property: String,
}

// ---- Index ----

/// Index access: `object[index]`.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpression {
    pub span: Span,
    pub object: Box<Expression>,
    pub index: Box<Expression>,
}

// ---- Assign ----

#[derive(Debug, Clone, PartialEq)]
pub enum AssignOp {
    Assign,
    AddAssign,
}

/// Assignment: `target op value`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentExpression {
    pub span: Span,
    pub op: AssignOp,
    pub target: Box<Expression>,
    pub value: Box<Expression>,
}

// ---- Group ----

/// Parenthesized expression.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupExpression {
    pub span: Span,
    pub expression: Box<Expression>,
}

// ---- List ----

/// List literal: `[a, b, c]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ListExpression {
    pub span: Span,
    pub elems: Vec<Option<Expression>>,
}

// ---- Object ----

/// Object literal: `{ key: value }`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectExpression {
    pub span: Span,
    pub props: Vec<ObjectProperty>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectProperty {
    KeyValue(KeyValueProperty),
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyValueProperty {
    pub span: Span,
    pub key: PropertyKey,
    pub value: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyKey {
    Ident {
        name: String,
        span: Span,
    },
    String {
        value: String,
        span: Span,
    },
    Number {
        value: String,
        span: Span,
    },
    Computed {
        expression: Box<Expression>,
        span: Span,
    },
}

// ---- Closure ----

/// Closure with pipe parameters: `|x| expression`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosureExpression {
    pub span: Span,
    pub params: Vec<crate::patterns::Pattern>,
    pub body: Box<Expression>,
}

// ---- Is (inline pattern match) ----

/// An inline `is` expression: `is value pattern => body`.
#[derive(Debug, Clone, PartialEq)]
pub struct IsExpression {
    pub span: Span,
    /// The value being matched.
    pub value: Box<Expression>,
    /// The pattern to match against.
    pub pattern: crate::patterns::IsPattern,
    /// The body expression when the pattern matches.
    pub body: Box<Expression>,
}

// ---- DotIdent ----

/// Dot-prefixed enum-like value: `.blue`, `.entered`.
#[derive(Debug, Clone, PartialEq)]
pub struct DotIdentifierExpression {
    pub span: Span,
    pub name: String,
}

// ---- Template ----

/// Template string with interpolation: `"hello {name}"`.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateExpression {
    pub span: Span,
    pub parts: Vec<TemplatePart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    String {
        value: String,
        span: Span,
    },
    Expression {
        expression: Box<Expression>,
        span: Span,
    },
}

// ---- AstNode implementation ----

impl AstNode for Expression {
    fn span(&self) -> Span {
        match self {
            Expression::Literal(e) => e.span,
            Expression::Ident(e) => e.span,
            Expression::Unary(e) => e.span,
            Expression::Binary(e) => e.span,
            Expression::Call(e) => e.span,
            Expression::Member(e) => e.span,
            Expression::Index(e) => e.span,
            Expression::Assign(e) => e.span,
            Expression::Group(e) => e.span,
            Expression::List(e) => e.span,
            Expression::Object(e) => e.span,
            Expression::Template(e) => e.span,
            Expression::Closure(e) => e.span,
            Expression::DotIdent(e) => e.span,
            Expression::Is(e) => e.span,
            Expression::Component(e) => e.span,
        }
    }
}

// ---- Constructor helpers ----

impl Expression {
    pub fn literal(kind: LiteralKind, span: Span) -> Self {
        Expression::Literal(LiteralExpression { span, kind })
    }

    pub fn ident(name: impl Into<String>, span: Span) -> Self {
        Expression::Ident(IdentifierExpression {
            span,
            name: name.into(),
        })
    }

    pub fn binary(left: Expression, op: BinaryOp, right: Expression, span: Span) -> Self {
        Expression::Binary(BinaryExpression {
            span,
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    pub fn list(elems: Vec<Option<Expression>>, span: Span) -> Self {
        Expression::List(ListExpression { span, elems })
    }
}
