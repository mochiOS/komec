//! Top-level declarations: `component`, `function`, `let`, `const`, `use`.

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
/// component App() { state x = 1; recipe body { ... } }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDeclaration {
    pub span: Span,
    pub name: String,
    pub params: Vec<crate::types::Parameter>,
    pub attributes: Vec<Attribute>,
    pub body: Vec<ComponentMember>,
}

/// An item inside a component body.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentMember {
    State(Binding),
    Recipe(RecipeDeclaration),
    Attribute(Attribute),
}

// ---- Recipe ----

/// A `recipe` (event handler / lifecycle method) inside a component.
///
/// ```kome
/// recipe body { ... }
/// recipe load_article: id_input { ... }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeDeclaration {
    pub span: Span,
    pub name: String,
    pub event_source: Option<String>,
    pub body: crate::statements::BlockStatement,
}

// ---- Attribute ----

/// An attribute, e.g. `@application`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub span: Span,
    pub name: String,
    pub args: Vec<crate::expressions::Expression>,
}

// ---- Function ----

/// A standalone function declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub span: Span,
    pub name: String,
    pub params: Vec<crate::patterns::Pattern>,
    pub body: Option<crate::statements::BlockStatement>,
    pub return_type: Option<crate::types::Type>,
}

// ---- Binding (state / let / const) ----

/// A variable binding: `state`, `let`, or `const`.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub span: Span,
    pub pattern: crate::patterns::Pattern,
    pub init: Option<crate::expressions::Expression>,
    pub type_annotation: Option<crate::types::Type>,
}

// ---- Use ----

/// A `use` import.
///
/// ```kome
/// use *;
/// use viewkit;
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

/// A source file: a list of declarations.
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

impl AstNode for Module {
    fn span(&self) -> Span {
        self.span
    }
}

// ---- AstNode implementation ----

impl AstNode for Declaration {
    fn span(&self) -> Span {
        match self {
            Declaration::Component(d) => d.span,
            Declaration::Function(d) => d.span,
            Declaration::Let(d) | Declaration::Constant(d) => d.span,
            Declaration::Use(d) => d.span,
        }
    }
}
