use crate::error::ResolutionError;
use crate::scope::{Reference, Scope, ScopeId, ScopeKind, Symbol, SymbolId};

pub struct ScopeBuilder {
    scopes: Vec<Scope>,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    errors: Vec<ResolutionError>,
    scope_stack: Vec<ScopeId>,
    next_scope_id: usize,
    next_symbol_id: usize,
}

impl ScopeBuilder {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            symbols: Vec::new(),
            references: Vec::new(),
            errors: Vec::new(),
            scope_stack: Vec::new(),
            next_scope_id: 0,
            next_symbol_id: 0,
        }
    }
}
