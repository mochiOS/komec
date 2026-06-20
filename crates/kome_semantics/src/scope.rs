use crate::error::ResolutionError;
use kome_ast::Span;

pub type ScopeId = usize;
pub type SymbolId = usize;

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
    Component { name: String, span: Span },
    Function { name: String, span: Span },
    Parameter { name: String, span: Span },
    Variable { name: String, span: Span },
    Recipe { name: String, span: Span },
    EnumType { name: String, span: Span },
    EnumCase { name: String, span: Span },
    ImportedName { name: String, span: Span },
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
            | Symbol::ImportedName { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub span: Span,
    pub name: String,
    pub resolved_to: Option<SymbolId>,
}

#[derive(Debug, Clone)]
pub struct NameResolution {
    pub scopes: Vec<Scope>,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub errors: Vec<ResolutionError>,
    pub root: ScopeId,
}
