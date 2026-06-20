use crate::error::ResolutionError;
use crate::scope::{NameResolution, Reference, Scope, ScopeId, ScopeKind, Symbol, SymbolId};
use kome_ast::Span;
use kome_ast::{
    declarations::{
        Binding, ComponentDeclaration, ComponentMember, Declaration, EnumCase, EnumDeclaration,
        ExtensionDeclaration, ExtensionMember, FunctionDeclaration, Module, RecipeDeclaration,
        UseSpecifier,
    },
    expressions::{
        AssignmentExpression, BinaryExpression, BlockExpression, CallArg, CallExpression,
        ClosureExpression, ComponentExpression, Expression, IsExpression, MemberExpression,
        ObjectExpression, ObjectProperty, TemplateExpression, TemplatePart,
    },
    patterns::{IsPattern, Pattern},
    statements::{
        BlockStatement, ForInStatement, IfStatement, IsStatement, ReturnStatement, Statement,
        WhileStatement,
    },
    types::{NamedType, Type},
};

/// Walks a Kome AST and builds a [`NameResolution`] side-table.
///
/// This is the first semantic pass: it records every scope, symbol declaration, and
/// name reference, reporting errors for undefined names, duplicate definitions (except
/// for variables), and invalid `let` placements.
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

    /// Runs name resolution on a parsed [`Module`] and returns the result.
    pub fn resolve(module: &Module) -> NameResolution {
        let mut builder = Self::new();
        builder.visit_module(module);
        NameResolution {
            scopes: builder.scopes,
            symbols: builder.symbols,
            references: builder.references,
            errors: builder.errors,
            root: 0,
        }
    }

    // -- module visitor --

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

    // -- declaration visitors --

    fn visit_top_level_declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::Component(comp) => self.visit_component_declaration(comp),
            Declaration::Function(func) => self.visit_function_declaration(func),
            Declaration::Let(binding) => {
                self.errors
                    .push(ResolutionError::InvalidLetLocation { span: binding.span });
                if let Some(ref ty) = binding.type_annotation {
                    self.visit_type(ty);
                }
                if let Some(ref init) = binding.init {
                    self.visit_expression(init);
                }
            }
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
                    ComponentMember::Let(binding) => {
                        self.errors
                            .push(ResolutionError::InvalidLetLocation { span: binding.span });
                        if let Some(ref ty) = binding.type_annotation {
                            self.visit_type(ty);
                        }
                        if let Some(ref init) = binding.init {
                            self.visit_expression(init);
                        }
                    }
                    ComponentMember::Recipe(recipe) => self.visit_recipe(recipe),
                    ComponentMember::Function(func) => self.visit_function_declaration(func),
                }
            }
        }

        self.exit_scope();
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

    // -- binding/pattern visitors --

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

    // -- statement visitors --

    fn visit_block_statement(&mut self, block: &BlockStatement) {
        self.enter_scope(ScopeKind::Block);
        for stmt in &block.statements {
            self.visit_statement(stmt);
        }
        self.exit_scope();
    }

    fn visit_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Block(block) => self.visit_block_statement(block),
            Statement::Expression(expr_stmt) => self.visit_expression(&expr_stmt.expression),
            Statement::Let(binding) => self.register_binding(binding),
            Statement::If(if_stmt) => self.visit_if_statement(if_stmt),
            Statement::While(while_stmt) => self.visit_while_statement(while_stmt),
            Statement::ForIn(for_in) => self.visit_for_in_statement(for_in),
            Statement::Return(ret) => self.visit_return_statement(ret),
            Statement::Break(_) | Statement::Continue(_) | Statement::Empty(_) => {}
            Statement::Is(is_stmt) => self.visit_is_statement(is_stmt),
            Statement::Declaration(decl) => self.visit_top_level_declaration(decl),
        }
    }

    fn visit_if_statement(&mut self, if_stmt: &IfStatement) {
        self.visit_expression(&if_stmt.test);
        self.enter_scope(ScopeKind::Block);
        self.visit_statement(&if_stmt.consequent);
        self.exit_scope();

        if let Some(ref alt) = if_stmt.alternative {
            self.enter_scope(ScopeKind::Block);
            self.visit_statement(alt);
            self.exit_scope();
        }
    }

    fn visit_while_statement(&mut self, while_stmt: &WhileStatement) {
        self.visit_expression(&while_stmt.test);
        self.enter_scope(ScopeKind::Block);
        self.visit_statement(&while_stmt.body);
        self.exit_scope();
    }

    fn visit_for_in_statement(&mut self, for_in: &ForInStatement) {
        self.visit_expression(&for_in.right);
        self.enter_scope(ScopeKind::ForIn);
        self.visit_pattern_binding(&for_in.pattern);
        self.visit_statement(&for_in.body);
        self.exit_scope();
    }

    fn visit_return_statement(&mut self, ret: &ReturnStatement) {
        if let Some(ref arg) = ret.argument {
            self.visit_expression(arg);
        }
    }

    fn visit_is_statement(&mut self, is_stmt: &IsStatement) {
        if let Some(ref value) = is_stmt.value {
            self.visit_expression(value);
        }
        self.enter_scope(ScopeKind::IsPattern);
        self.visit_is_pattern(&is_stmt.pattern);
        self.visit_statement(&is_stmt.body);
        self.exit_scope();
    }

    // -- expression visitors --

    fn visit_expression(&mut self, expr: &Expression) {
        match expr {
            Expression::Literal(_) => {}
            Expression::Ident(ident) => {
                self.record_reference(&ident.name, ident.span);
            }
            Expression::Unary(unary) => {
                self.visit_expression(&unary.argument);
            }
            Expression::Binary(binary) => self.visit_binary_expression(binary),
            Expression::Call(call) => self.visit_call_expression(call),
            Expression::Member(member) => self.visit_member_expression(member),
            Expression::Index(index) => {
                self.visit_expression(&index.object);
                self.visit_expression(&index.index);
            }
            Expression::Assign(assign) => self.visit_assignment_expression(assign),
            Expression::Group(group) => {
                self.visit_expression(&group.expression);
            }
            Expression::Block(block) => self.visit_block_expression(block),
            Expression::List(list) => {
                for elem in &list.elems {
                    if let Some(e) = elem {
                        self.visit_expression(e);
                    }
                }
            }
            Expression::Object(obj) => self.visit_object_expression(obj),
            Expression::Template(tmpl) => self.visit_template_expression(tmpl),
            Expression::Closure(closure) => self.visit_closure_expression(closure),
            Expression::DotIdent(dot) => {
                self.record_reference(&dot.name, dot.span);
            }
            Expression::Is(is_expr) => self.visit_is_expression(is_expr),
            Expression::Component(comp) => self.visit_component_expression(comp),
        }
    }

    fn visit_binary_expression(&mut self, binary: &BinaryExpression) {
        self.visit_expression(&binary.left);
        self.visit_expression(&binary.right);
    }

    fn visit_call_expression(&mut self, call: &CallExpression) {
        self.visit_expression(&call.callee);

        for arg in &call.args {
            match arg {
                CallArg::Positional(expr) => self.visit_expression(expr),
                CallArg::Named { value, .. } => self.visit_expression(value),
            }
        }
    }

    fn visit_member_expression(&mut self, member: &MemberExpression) {
        self.visit_expression(&member.object);
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression) {
        self.visit_expression(&assign.target);
        self.visit_expression(&assign.value);
    }

    fn visit_block_expression(&mut self, block: &BlockExpression) {
        self.enter_scope(ScopeKind::Block);
        for stmt in &block.statements {
            self.visit_statement(stmt);
        }
        if let Some(ref tail) = block.tail {
            self.visit_expression(tail);
        }
        self.exit_scope();
    }

    fn visit_object_expression(&mut self, obj: &ObjectExpression) {
        for prop in &obj.props {
            let ObjectProperty::KeyValue(kv) = prop;
            self.visit_expression(&kv.value);
        }
    }

    fn visit_template_expression(&mut self, tmpl: &TemplateExpression) {
        for part in &tmpl.parts {
            if let TemplatePart::Expression { expression, .. } = part {
                self.visit_expression(expression);
            }
        }
    }

    fn visit_closure_expression(&mut self, closure: &ClosureExpression) {
        self.enter_scope(ScopeKind::Closure);
        for param in &closure.params {
            self.visit_pattern_binding(param);
        }
        self.visit_expression(&closure.body);
        self.exit_scope();
    }

    fn visit_is_expression(&mut self, is_expr: &IsExpression) {
        self.visit_expression(&is_expr.value);
        self.enter_scope(ScopeKind::IsPattern);
        self.visit_is_pattern(&is_expr.pattern);
        self.visit_expression(&is_expr.body);
        self.exit_scope();
    }

    fn visit_component_expression(&mut self, comp: &ComponentExpression) {
        self.record_reference(&comp.name, comp.span);

        for arg in &comp.args {
            match arg {
                CallArg::Positional(expr) => self.visit_expression(expr),
                CallArg::Named { value, .. } => self.visit_expression(value),
            }
        }

        for child in &comp.children {
            self.visit_expression(child);
        }
    }

    // -- pattern visitors --

    fn visit_is_pattern(&mut self, pattern: &IsPattern) {
        match pattern {
            IsPattern::Literal(_) => {}
            IsPattern::Ident(ident) => {
                self.declare(
                    ident.span,
                    Symbol::Variable {
                        name: ident.name.clone(),
                        span: ident.span,
                    },
                );
            }
            IsPattern::DotIdent(dot) => {
                self.record_reference(&dot.name, dot.span);
            }
        }
    }

    // -- type visitors --

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

    // -- scope management --

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

    fn alloc_scope(&mut self) -> ScopeId {
        let id = self.next_scope_id;
        self.next_scope_id += 1;
        id
    }

    fn current_scope(&self) -> Option<ScopeId> {
        self.scope_stack.last().copied()
    }

    // -- symbol management --

    fn declare(&mut self, span: Span, symbol: Symbol) {
        let name = symbol.name().to_string();
        let scope_id = match self.current_scope() {
            Some(id) => id,
            None => {
                self.errors.push(ResolutionError::ScopeStackEmpty);
                return;
            }
        };

        let dup = !matches!(symbol, Symbol::Variable { .. })
            && self.scopes[scope_id]
                .symbols
                .iter()
                .any(|(n, _, _)| n == &name);
        if dup {
            let first = self.scopes[scope_id]
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

    fn alloc_symbol(&mut self) -> SymbolId {
        let id = self.next_symbol_id;
        self.next_symbol_id += 1;
        id
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

    fn resolve_name(&self, name: &str) -> Option<SymbolId> {
        for &scope_id in self.scope_stack.iter().rev() {
            let scope = &self.scopes[scope_id];
            if let Some((_, sym_id, _)) = scope.symbols.iter().find(|(n, _, _)| n == name) {
                return Some(*sym_id);
            }
        }
        None
    }
}
