use crate::error::ResolutionError;
use crate::scope::{Reference, Scope, ScopeId, ScopeKind, Symbol, SymbolId};
use kome_ast::Span;

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

    fn declare(&mut self, span: Span, symbol: Symbol) {
        let name = symbol.name().to_string();
        let scope_id = self.current_scope();
        let scope = &self.scopes[scope_id];

        let dup = scope.symbols.iter().any(|(n, _, _)| n == &name);
        if dup {
            let first = scope
                .symbols
                .iter()
                .find(|(n, _, _)| n == &name)
                .map(|(_, _, s)| *s)
                .unwrap();
            self.errors.push(ResolutionError::DuplicateDefinition {
                name,
                first,
                second: span,
            });
            return;
        }

        let sym_id = self.alloc_symbol();
        self.symbols.push(symbol);
        self.scopes[scope_id].symbols.push((name, sym_id, span));
    }

    fn resolve_name(&self, name: &str) -> Option<SymbolId> {
        for &scope_id in self.scope_stack.iter().rev() {
            let scope = &self.scopes[scope_id];
            if let Some((_, sym_id, _)) = scope.symbols.iter().find(|(n, _, _)| n == name) {
                return Some(*sym_id);
            }
        }
        None
    }

    fn record_reference(&mut self, name: &str, span: Span) {
        let resolved = self.resolve_name(name);
        if resolved.is_none() {
            self.errors.push(ResolutionError::UndefinedName {
                name: name.to_string(),
                span,
            });
        }
        self.references.push(Reference {
            span,
            name: name.to_string(),
            resolved_to: resolved,
        });
    }
}
