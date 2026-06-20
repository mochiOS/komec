use crate::error::ResolutionError;
use crate::scope::{Reference, Scope, ScopeId, ScopeKind, Symbol, SymbolId};
use kome_ast::Span;
use kome_ast::{
    declarations::{
        Binding, ComponentDeclaration, ComponentMember, Declaration, EnumCase, EnumDeclaration,
        ExtensionDeclaration, ExtensionMember, FunctionDeclaration, Module, RecipeDeclaration,
        UseSpecifier,
    },
    expressions::Expression,
    patterns::{IsPattern, Pattern},
    statements::BlockStatement,
    types::{NamedType, Type},
};

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

    // -- stubs (filled in later commits) --

    fn visit_type(&mut self, ty: &Type) {
        match ty {
            Type::Primitive(_) => {}
            Type::Function(func_type) => {
                for param in &func_type.params {
                    self.visit_type(&param.type_);
                }
                self.visit_type(&func_type.return_type);
            }
            Type::List(list_type) => {
                self.visit_type(&list_type.element);
            }
            Type::Object(obj_type) => {
                for member in &obj_type.members {
                    self.visit_type(&member.type_);
                }
            }
            Type::Named(named) => {
                self.visit_named_type(named);
            }
            Type::Optional(opt) => {
                self.visit_type(&opt.inner);
            }
        }
    }

    fn visit_named_type(&mut self, named: &NamedType) {
        self.record_reference(&named.name, named.span);

        for arg in &named.type_arguments {
            self.visit_type(arg);
        }
    }

    fn visit_block_statement(&mut self, _block: &BlockStatement) {}

    fn visit_expression(&mut self, _expr: &Expression) {}

    fn visit_is_pattern(&mut self, _pattern: &IsPattern) {}

    // -- declaration visitors --

    fn visit_module(&mut self, module: &Module) {
        self.enter_scope(ScopeKind::Module);

        for decl in &module.declarations {
            if let Declaration::Use(use_decl) = decl {
                for spec in &use_decl.specifiers {
                    if let UseSpecifier::Named { name, span } = spec {
                        self.declare(
                            *span,
                            Symbol::ImportedName {
                                name: name.clone(),
                                span: *span,
                            },
                        );
                    }
                }
            }
        }

        for decl in &module.declarations {
            self.visit_top_level_declaration(decl);
        }

        self.exit_scope();
    }

    fn visit_top_level_declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::Component(comp) => self.visit_component_declaration(comp),
            Declaration::Function(func) => self.visit_function_declaration(func),
            Declaration::Let(binding) => self.register_binding(binding),
            Declaration::Constant(binding) => self.register_binding(binding),
            Declaration::Use(_) => {}
            Declaration::Enum(enum_decl) => self.visit_enum_declaration(enum_decl),
            Declaration::Extension(ext) => self.visit_extension_declaration(ext),
        }
    }

    fn visit_component_declaration(&mut self, comp: &ComponentDeclaration) {
        self.declare(
            comp.span,
            Symbol::Component {
                name: comp.name.clone(),
                span: comp.span,
            },
        );
        self.enter_scope(ScopeKind::Component);

        for param in &comp.params {
            self.declare(
                param.span,
                Symbol::Parameter {
                    name: param.name.clone(),
                    span: param.span,
                },
            );
        }

        if let Some(members) = &comp.body {
            for member in members {
                match member {
                    ComponentMember::State(binding) => self.register_binding(binding),
                    ComponentMember::Let(binding) => self.register_binding(binding),
                    ComponentMember::Recipe(recipe) => self.visit_recipe(recipe),
                    ComponentMember::Function(func) => self.visit_function_declaration(func),
                }
            }
        }

        self.exit_scope();
    }

    fn visit_recipe(&mut self, recipe: &RecipeDeclaration) {
        self.declare(
            recipe.span,
            Symbol::Recipe {
                name: recipe.name.clone(),
                span: recipe.span,
            },
        );

        if let Some(ref source) = recipe.event_source {
            self.record_reference(source, recipe.span);
        }

        self.visit_block_statement(&recipe.body);
    }

    fn visit_function_declaration(&mut self, func: &FunctionDeclaration) {
        self.declare(
            func.span,
            Symbol::Function {
                name: func.name.clone(),
                span: func.span,
            },
        );
        self.enter_scope(ScopeKind::Function);

        for param in &func.params {
            self.visit_pattern_binding(param);
        }

        if let Some(ref return_type) = func.return_type {
            self.visit_type(return_type);
        }

        if let Some(ref body) = func.body {
            self.visit_block_statement(body);
        }

        self.exit_scope();
    }

    fn visit_enum_declaration(&mut self, enum_decl: &EnumDeclaration) {
        self.declare(
            enum_decl.span,
            Symbol::EnumType {
                name: enum_decl.name.clone(),
                span: enum_decl.span,
            },
        );
        self.enter_scope(ScopeKind::IsPattern);
        for case in &enum_decl.cases {
            self.register_enum_case(case);
        }
        self.exit_scope();
    }

    fn register_enum_case(&mut self, case: &EnumCase) {
        self.declare(
            case.span,
            Symbol::EnumCase {
                name: case.name.clone(),
                span: case.span,
            },
        );

        if let Some(ref value) = case.value {
            self.visit_expression(value);
        }
    }

    fn visit_extension_declaration(&mut self, ext: &ExtensionDeclaration) {
        self.visit_type(&ext.target);

        for member in &ext.members {
            match member {
                ExtensionMember::Function(func) => self.visit_function_declaration(func),
            }
        }
    }

    fn register_binding(&mut self, binding: &Binding) {
        self.visit_pattern_binding(&binding.pattern);

        if let Some(ref type_ann) = binding.type_annotation {
            self.visit_type(type_ann);
        }

        if let Some(ref init) = binding.init {
            self.visit_expression(init);
        }
    }

    fn visit_pattern_binding(&mut self, pattern: &Pattern) {
        if let Pattern::Ident(ident) = pattern {
            self.declare(
                ident.span,
                Symbol::Variable {
                    name: ident.name.clone(),
                    span: ident.span,
                },
            );

            if let Some(ref type_ann) = ident.type_annotation {
                self.visit_type(type_ann);
            }

            if let Some(ref default) = ident.default {
                self.visit_expression(default);
            }
        }
    }
}
