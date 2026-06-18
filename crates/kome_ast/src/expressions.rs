//! Expressions: literals, operators, calls, closures, blocks,
//! and component expressions.

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

    /// A binary operator expression: `a + b`, `x == y`, etc.
    Binary(BinaryExpression),

    /// A function or method call: `foo()`, `obj.method(x, y: z)`.
    Call(CallExpression),

    /// Property access: `object.property`.
    Member(MemberExpression),

    /// Index access: `object[index]`.
    Index(IndexExpression),

    /// An assignment expression: `target = value`, `x += 1`.
    Assign(AssignmentExpression),

    /// A parenthesized expression: `(expression)`.
    Group(GroupExpression),

    /// A block expression: `{ statements; tail_expression }`.
    Block(BlockExpression),

    /// A list literal: `[a, b, c]`.
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

    /// A component call with block children.
    ///
    /// ```kome
    /// VStack {
    ///     Text("Hello")
    /// }
    /// ```
    Component(ComponentExpression),
}

// ---- Literal ----

/// A numeric literal value stored as source text.
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

/// A literal expression.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralExpression {
    pub span: Span,
    pub kind: LiteralKind,
}

// ---- Identifier ----

/// An identifier expression.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentifierExpression {
    pub span: Span,
    pub name: String,
}

// ---- Unary ----

/// A unary operator.
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,
}

/// A unary operator expression.
#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpression {
    pub span: Span,
    pub op: UnaryOp,
    pub argument: Box<Expression>,
}

// ---- Binary ----

/// A binary operator.
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

/// A binary operator expression.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpression {
    pub span: Span,
    pub op: BinaryOp,
    pub left: Box<Expression>,
    pub right: Box<Expression>,
}

// ---- Call argument ----

/// A positional or named call argument.
///
/// ```kome
/// Button("Click", color: .blue)
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

/// A function or method call.
///
/// ```kome
/// foo()
/// object.method(value)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub span: Span,
    pub callee: Box<Expression>,
    pub args: Vec<CallArg>,
}

// ---- Component ----

/// A component expression with block children.
///
/// ```kome
/// VStack {
///     Text("Hello")
///     Button("Click")
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentExpression {
    pub span: Span,
    pub name: String,
    pub args: Vec<CallArg>,
    pub children: Vec<Expression>,
}

// ---- Member ----

/// Property access.
///
/// ```kome
/// object.property
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MemberExpression {
    pub span: Span,
    pub object: Box<Expression>,
    pub property: String,
}

// ---- Index ----

/// Index access.
///
/// ```kome
/// object[index]
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpression {
    pub span: Span,
    pub object: Box<Expression>,
    pub index: Box<Expression>,
}

// ---- Assignment ----

/// An assignment operator.
#[derive(Debug, Clone, PartialEq)]
pub enum AssignOp {
    Assign,
    AddAssign,
}

/// An assignment expression.
///
/// ```kome
/// value = 1
/// value += 1
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentExpression {
    pub span: Span,
    pub op: AssignOp,
    pub target: Box<Expression>,
    pub value: Box<Expression>,
}

// ---- Group ----

/// A parenthesized expression.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupExpression {
    pub span: Span,
    pub expression: Box<Expression>,
}

// ---- Block ----

/// A block expression.
///
/// Statements are evaluated in order. If `tail` is present, its value is
/// the value of the entire block.
///
/// ```kome
/// {
///     let message = "Hello"
///     Text(message)
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BlockExpression {
    pub span: Span,
    pub statements: Vec<crate::statements::Statement>,
    pub tail: Option<Box<Expression>>,
}

// ---- List ----

/// A list literal.
///
/// ```kome
/// [a, b, c]
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ListExpression {
    pub span: Span,
    pub elems: Vec<Option<Expression>>,
}

// ---- Object ----

/// An object literal.
///
/// ```kome
/// {
///     name: "Kome",
///     version: 1,
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectExpression {
    pub span: Span,
    pub props: Vec<ObjectProperty>,
}

/// A property inside an object literal.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectProperty {
    KeyValue(KeyValueProperty),
}

/// A key-value object property.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyValueProperty {
    pub span: Span,
    pub key: PropertyKey,
    pub value: Box<Expression>,
}

/// An object property key.
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

/// A closure with pipe parameters.
///
/// ```kome
/// |value| print(value)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ClosureExpression {
    pub span: Span,
    pub params: Vec<crate::patterns::Pattern>,
    pub body: Box<Expression>,
}

// ---- Is ----

/// An inline `is` expression.
///
/// ```kome
/// is value pattern => body
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct IsExpression {
    pub span: Span,

    /// The value being matched.
    pub value: Box<Expression>,

    /// The pattern to match against.
    pub pattern: crate::patterns::IsPattern,

    /// The expression evaluated when the pattern matches.
    pub body: Box<Expression>,
}

// ---- Dot identifier ----

/// A dot-prefixed enum-like or type-associated value.
///
/// ```kome
/// .blue
/// .entered
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DotIdentifierExpression {
    pub span: Span,
    pub name: String,
}

// ---- Template ----

/// A template string with interpolation.
///
/// ```kome
/// "Hello, {name}"
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateExpression {
    pub span: Span,
    pub parts: Vec<TemplatePart>,
}

/// One part of a template string.
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
            Expression::Literal(expression) => expression.span,
            Expression::Ident(expression) => expression.span,
            Expression::Unary(expression) => expression.span,
            Expression::Binary(expression) => expression.span,
            Expression::Call(expression) => expression.span,
            Expression::Member(expression) => expression.span,
            Expression::Index(expression) => expression.span,
            Expression::Assign(expression) => expression.span,
            Expression::Group(expression) => expression.span,
            Expression::Block(expression) => expression.span,
            Expression::List(expression) => expression.span,
            Expression::Object(expression) => expression.span,
            Expression::Template(expression) => expression.span,
            Expression::Closure(expression) => expression.span,
            Expression::DotIdent(expression) => expression.span,
            Expression::Is(expression) => expression.span,
            Expression::Component(expression) => expression.span,
        }
    }
}

impl AstNode for LiteralExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for IdentifierExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for UnaryExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for BinaryExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for CallExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ComponentExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for MemberExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for IndexExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for AssignmentExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for GroupExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for BlockExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ListExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ObjectExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for KeyValueProperty {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ClosureExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for IsExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for DotIdentifierExpression {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for TemplateExpression {
    fn span(&self) -> Span {
        self.span
    }
}

// ---- Constructor helpers ----

impl Expression {
    /// Creates a literal expression.
    pub fn literal(kind: LiteralKind, span: Span) -> Self {
        Self::Literal(LiteralExpression { span, kind })
    }

    /// Creates an identifier expression.
    pub fn ident(name: impl Into<String>, span: Span) -> Self {
        Self::Ident(IdentifierExpression {
            span,
            name: name.into(),
        })
    }

    /// Creates a binary operator expression.
    pub fn binary(
        left: Expression,
        op: BinaryOp,
        right: Expression,
        span: Span,
    ) -> Self {
        Self::Binary(BinaryExpression {
            span,
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    /// Creates a block expression.
    pub fn block(
        statements: Vec<crate::statements::Statement>,
        tail: Option<Expression>,
        span: Span,
    ) -> Self {
        Self::Block(BlockExpression {
            span,
            statements,
            tail: tail.map(Box::new),
        })
    }

    /// Creates a list expression.
    pub fn list(
        elems: Vec<Option<Expression>>,
        span: Span,
    ) -> Self {
        Self::List(ListExpression { span, elems })
    }
}