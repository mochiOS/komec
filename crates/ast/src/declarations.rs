//! Declarations for `component`, `function`, `recipe`, `state`,
//! `let`, `const`, and `use`.

use crate::{AstNode, Span};

/// A declaration placed at the top level of a source file.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    Component(ComponentDeclaration),
    Function(FunctionDeclaration),
    Struct(StructDeclaration),
    Let(Binding),
    Constant(Binding),
    Use(UseDeclaration),
    Enum(EnumDeclaration),
    Extension(ExtensionDeclaration),
}

// ---- Struct ----

/// A named product type. `None` fields represent an opaque struct declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub name: String,
    pub fields: Option<Vec<StructField>>,
}

/// One named field in a struct declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub span: Span,
    pub name: String,
    pub type_: crate::types::Type,
}

// ---- Component ----

/// A `component` declaration.
///
/// A component may either contain a Kome implementation:
///
/// ```kome
/// component App() {
///     state counter = 0
/// }
/// ```
///
/// Or declare an externally implemented component:
///
/// ```kome
/// @nativeComponent("Text")
/// component Text(
///     content: String,
/// )
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDeclaration {
    pub span: Span,
    pub name: String,
    pub params: Vec<crate::types::Parameter>,
    pub attributes: Vec<Attribute>,

    /// `None` when the declaration has no Kome body.
    ///
    /// `Some(Vec::new())` represents an explicitly empty body: `{}`.
    pub body: Option<Vec<ComponentMember>>,
}

/// A declaration placed inside a component body.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentMember {
    State(Box<Binding>),
    Let(Box<Binding>),
    Recipe(RecipeDeclaration),
    Function(FunctionDeclaration),
}

// ---- Recipe ----

/// A `recipe` declaration inside a component.
///
/// A recipe may represent an event handler, a reactive operation,
/// or a lifecycle operation.
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

/// An attribute attached to a declaration.
///
/// ```kome
/// @application
/// @body
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub span: Span,
    pub name: String,
    pub args: Vec<crate::expressions::Expression>,
}

// ---- Function ----

/// A function declaration.
///
/// Functions may be placed at the top level or inside a component.
///
/// ```kome
/// fn greet(name: String) {
///     return "Hello, " + name
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub name: String,
    pub params: Vec<crate::patterns::Pattern>,
    pub body: Option<crate::statements::BlockStatement>,
    pub return_type: Option<crate::types::Type>,
}

// ---- Binding ----

/// A variable binding declared with `state`, `let`, or `const`.
///
/// The kind of binding is determined by the containing enum variant:
///
/// - [`ComponentMember::State`]
/// - [`ComponentMember::Let`]
/// - [`Declaration::Let`]
/// - [`Declaration::Constant`]
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub mutable: bool,
    pub pattern: crate::patterns::Pattern,
    pub init: Option<crate::expressions::Expression>,
    pub type_annotation: Option<crate::types::Type>,
}

// ---- Use ----

/// One or more module imports.
///
/// ```kome
/// use std.io
/// use viewKit
/// use collections, io
/// use *
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct UseDeclaration {
    pub span: Span,
    pub imports: Vec<UseImport>,
}

/// One import entry inside a `use` declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum UseImport {
    Module(Path),

    Wildcard { span: Span },
}

/// A path like `std::io`, `self::super::thing`.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub span: Span,
    pub segments: Vec<PathSegment>,
    pub separators: Vec<PathSeparator>,
}

/// One segment inside a path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathSegment {
    pub span: Span,
    pub kind: PathSegmentKind,
}

/// The kind of a path segment.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegmentKind {
    /// A regular identifier like `std`, `io`, `foo`.
    Ident(String),
    /// The `self` keyword.
    Self_,
    /// The `super` keyword.
    Super,
}

/// Separator between path segments.
#[derive(Debug, Clone, PartialEq)]
pub enum PathSeparator {
    /// `::`
    ColonColon,
}

// ---- Enum ----

/// An enum declaration.
///
/// ```kome
/// enum Color {
///     blue,
///     red,
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub name: String,
    pub cases: Vec<EnumCase>,
}

/// One case declared inside an enum.
///
/// ```kome
/// enum HttpStatus {
///     ok = 200,
///     notFound = 404,
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EnumCase {
    pub span: Span,
    pub name: String,

    /// The optional raw value assigned to this case.
    ///
    /// `None` for `blue`.
    /// `Some(...)` for `blue = "#007aff"`.
    pub value: Option<crate::expressions::Expression>,
}

// ---- Extension ---

/// A decalaration that adds members to an existing type
///
/// Extensions allow functions to be declared for a type without modifying
/// the types original declaration.
///
/// ```kome
/// extension View {
///     fn padding(value: Int) {
///         // ...
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionDeclaration {
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub target: crate::types::Type,
    pub members: Vec<ExtensionMember>,
}

// TODO: enumも生やす
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionMember {
    Function(FunctionDeclaration),
}

impl AstNode for ExtensionDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for ExtensionMember {
    fn span(&self) -> Span {
        match self {
            ExtensionMember::Function(declaration) => declaration.span,
        }
    }
}

// ---- Module ----

/// A Kome source file containing a list of declarations.
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

// ---- AstNode implementations ----

impl AstNode for UseDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for UseImport {
    fn span(&self) -> Span {
        match self {
            UseImport::Module(path) => path.span,
            UseImport::Wildcard { span } => *span,
        }
    }
}

impl AstNode for Path {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for PathSegment {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for PathSeparator {
    fn span(&self) -> Span {
        Span::new(0, 0)
    }
}

impl AstNode for Declaration {
    fn span(&self) -> Span {
        match self {
            Declaration::Component(declaration) => declaration.span,
            Declaration::Function(declaration) => declaration.span,
            Declaration::Struct(declaration) => declaration.span,
            Declaration::Let(binding) | Declaration::Constant(binding) => binding.span,
            Declaration::Use(declaration) => declaration.span,
            Declaration::Enum(declaration) => declaration.span,
            Declaration::Extension(declaration) => declaration.span,
        }
    }
}

impl AstNode for ComponentDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for StructDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for StructField {
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

impl AstNode for EnumDeclaration {
    fn span(&self) -> Span {
        self.span
    }
}

impl AstNode for EnumCase {
    fn span(&self) -> Span {
        self.span
    }
}
