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
