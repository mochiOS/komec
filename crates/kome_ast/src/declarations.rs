//! Declarations: `component`, `function`, `recipe`, `state`, `let`, `const`, `use`.

use crate::{AstNode, Span};

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Component(ComponentDeclaration),
    Function(FunctionDeclaration),
    Let(Binding),
    Constant(Binding),
    Use(UseDeclaration),
}

// ---- Component ----

/// A `component` declaration.
///
/// ```kome
/// @application
/// component App() {
///     state counter = 0
///
///     @body
///     let body: View = {
///         Text("count: {counter}")
///     }
///
///     fn increment() {
///         counter = counter + 1
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDeclaration {
    pub span: Span,
    pub name: String,
    pub params: Vec<crate::types::Parameter>,
    pub attributes: Vec<Attribute>,
    pub body: Vec<ComponentMember>,
}

/// An item declared inside a component body.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentMember {
    State(Box<Binding>),
    Let(Box<Binding>),
    Recipe(RecipeDeclaration),
    Function(FunctionDeclaration),
}

// ---- Recipe ----

/// A `recipe` event handler or lifecycle declaration inside a component.
///
/// ```kome
/// recipe load_article: id_input {
///     print(id_input)
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub name: String,
    pub event_source: Option<String>,
    pub body: crate::statements::BlockStatement,
}

// ---- Attribute ----

/// An attribute such as `@application` or `@body`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub span: Span,
    pub name: String,
    pub args: Vec<crate::expressions::Expression>,
}

// ---- Function ----

/// A function declaration.
///
/// Functions may be declared at the top level or inside a component.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub name: String,
    pub params: Vec<crate::patterns::Pattern>,
    pub body: Option<crate::statements::BlockStatement>,
    pub return_type: Option<crate::types::Type>,
}

// ---- Binding (state / let / const) ----

/// A variable binding declared with `state`, `let`, or `const`.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub pattern: crate::patterns::Pattern,
    pub init: Option<crate::expressions::Expression>,
    pub type_annotation: Option<crate::types::Type>,
}

// ---- Use ----

/// A `use` import.
///
/// ```kome
/// use *
/// use viewKit
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct UseDeclaration {
    pub span: Span,
    pub specifiers: Vec<UseSpecifier>,
    pub source: Option<String>,
}

/// One specifier inside a `use` declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum UseSpecifier {
    Wildcard { span: Span },
    Named { name: String, span: Span },
}

// ---- Module ----

/// A source file containing a list of declarations.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub span: Span,
    pub declarations: Vec<Declaration>,
}

impl Module {
    pub fn new(declarations: Vec<Declaration>, span: Span) -> Self {
        Self { span, declarations }
    }
}

// ---- AstNode implementations ----

impl AstNode for Module {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for Declaration {
    fn span(&self) -> Span {
        match self {
            Declaration::Component(declaration) => declaration.span,
            Declaration::Function(declaration) => declaration.span,
            Declaration::Let(binding) | Declaration::Constant(binding) => binding.span,
            Declaration::Use(declaration) => declaration.span,
        }
    }
}

impl AstNode for ComponentDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ComponentMember {
    fn span(&self) -> Span {
        match self {
            ComponentMember::State(binding) => binding.span,
            ComponentMember::Let(binding) => binding.span,
            ComponentMember::Recipe(declaration) => declaration.span,
            ComponentMember::Function(declaration) => declaration.span,
        }
    }
}

impl AstNode for RecipeDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for Attribute {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for FunctionDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for Binding {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for UseDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for UseSpecifier {
    fn span(&self) -> Span {
        match self {
            UseSpecifier::Wildcard { span } | UseSpecifier::Named { span, .. } => *span,
        }
    }
}