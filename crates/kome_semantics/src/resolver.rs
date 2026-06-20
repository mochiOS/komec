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

    fn alloc_scope(&mut self) -> ScopeId {
        let id = self.next_scope_id;
        self.next_scope_id += 1;
        id
    }

    fn alloc_symbol(&mut self) -> SymbolId {
        let id = self.next_symbol_id;
        self.next_symbol_id += 1;
        id
    }

    fn current_scope(&self) -> ScopeId {
        *self.scope_stack.last().unwrap()
    }

    fn enter_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let parent = self.scope_stack.last().copied();
        let id = self.alloc_scope();
        self.scopes.push(Scope {
            id,
            parent,
            kind,
            symbols: Vec::new(),
            children: Vec::new(),
        });
        if let Some(parent_id) = parent {
            self.scopes[parent_id].children.push(id);
        }
        self.scope_stack.push(id);
        id
    }

    fn exit_scope(&mut self) {
        self.scope_stack.pop();
    }
}
