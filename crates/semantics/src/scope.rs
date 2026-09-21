use crate::error::ResolutionError;
use kome_ast::Span;

pub type ScopeId = usize;
pub type SymbolId = usize;
pub type SourceId = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeKind {
    Module,
    Component,
    Function,
    Block,
    Closure,
    ForIn,
    IsPattern,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    pub symbols: Vec<(String, SymbolId, Span)>,
    pub children: Vec<ScopeId>,
}

#[derive(Debug, Clone)]
pub enum Symbol {
    Component {
        name: String,
        span: Span,
    },
    Function {
        name: String,
        span: Span,
    },
    Parameter {
        name: String,
        span: Span,
    },
    Variable {
        name: String,
        span: Span,
        mutable: bool,
    },
    Recipe {
        name: String,
        span: Span,
    },
    EnumType {
        name: String,
        span: Span,
    },
    EnumCase {
        name: String,
        span: Span,
    },
    ImportedName {
        name: String,
        span: Span,
    },
    BuiltinFunction {
        name: String,
    },
}

impl Symbol {
    pub fn name(&self) -> &str {
        match self {
            Symbol::Component { name, .. }
            | Symbol::Function { name, .. }
            | Symbol::Parameter { name, .. }
            | Symbol::Variable { name, .. }
            | Symbol::Recipe { name, .. }
            | Symbol::EnumType { name, .. }
            | Symbol::EnumCase { name, .. }
            | Symbol::ImportedName { name, .. }
            | Symbol::BuiltinFunction { name, .. } => name,
        }
    }

    pub fn definition_span(&self) -> Option<Span> {
        match self {
            Self::Component { span, .. }
            | Self::Function { span, .. }
            | Self::Parameter { span, .. }
            | Self::Variable { span, .. }
            | Self::Recipe { span, .. }
            | Self::EnumType { span, .. }
            | Self::EnumCase { span, .. }
            | Self::ImportedName { span, .. } => Some(*span),

            Self::BuiltinFunction { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub span: Span,
    pub name: String,
    pub resolved_to: Option<SymbolId>,
    pub source: Option<SourceId>,
}

#[derive(Debug, Clone)]
pub struct NameResolution {
    pub scopes: Vec<Scope>,
    pub symbols: Vec<Symbol>,

    /// Source file containing each symbol definition.
    ///
    /// The index corresponds to `symbols`.
    /// Built-in symbols do not have a source file.
    pub symbol_sources: Vec<Option<SourceId>>,
    pub references: Vec<Reference>,
    pub errors: Vec<ResolutionError>,
    pub root: ScopeId,
}
