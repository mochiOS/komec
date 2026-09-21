use std::collections::HashMap;
use std::fmt;

use kome_ast::Span;
use kome_ast::declarations::{
    Binding, ComponentDeclaration, ComponentMember, Declaration, FunctionDeclaration, Module,
};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, CallArg, Expression, ObjectProperty, TemplatePart,
};
use kome_ast::patterns::{IsPattern, Pattern};
use kome_ast::statements::{
    BlockStatement, ForInStatement, IfStatement, IsStatement, Statement, WhileStatement,
};

/// An error produced when a variable may be read before it has been initialized.
#[derive(Debug, Clone)]
pub struct InitializationError {
    pub name: String,
    pub span: Span,
}

impl fmt::Display for InitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "variable `{}` may be uninitialized at byte range {}..{}",
            self.name, self.span.start, self.span.end,
        )
    }
}

impl std::error::Error for InitializationError {}

/// The result of definite-initialization analysis.
#[derive(Debug, Clone)]
pub struct InitializationResult {
    pub errors: Vec<InitializationError>,
}

/// Walks a Kome AST and checks that variables are initialized before use.
///
/// A variable declared with an initializer is immediately considered initialized.
///
/// ```kome
/// var value = 1
/// print(value)
/// ```
///
/// A variable declared without an initializer must be assigned before it is read.
///
/// ```kome
/// var value: Number
/// value = 1
/// print(value)
/// ```
pub struct InitializationChecker {
    scopes: Vec<HashMap<String, bool>>,
    errors: Vec<InitializationError>,
}

impl InitializationChecker {
    /// Runs definite-initialization analysis on a parsed [`Module`].
    pub fn check(module: &Module) -> InitializationResult {
        let mut checker = Self {
            scopes: vec![HashMap::new()],
            errors: Vec::new(),
        };

        checker.visit_module(module);

        InitializationResult {
            errors: checker.errors,
        }
    }

    // -- module visitor --

    fn visit_module(&mut self, module: &Module) {
        /*
         * Constants are available to functions in the module, so register
         * them before visiting function bodies.
         */
        for declaration in &module.declarations {
            if let Declaration::Constant(binding) = declaration {
                self.register_binding(binding);
            }
        }

        let module_scopes = self.scopes.clone();

        for declaration in &module.declarations {
            self.scopes = module_scopes.clone();

            match declaration {
                Declaration::Function(function) => self.visit_function(function),

                Declaration::Component(component) => self.visit_component(component),

                _ => {}
            }
        }

        self.scopes = module_scopes;
    }

    // -- declaration visitors --

    fn visit_component(&mut self, component: &ComponentDeclaration) {
        self.enter_scope();

        for parameter in &component.params {
            self.declare(&parameter.name, true);
        }

        let Some(members) = &component.body else {
            self.exit_scope();
            return;
        };

        /*
         * Component bindings are registered before functions and recipes so
         * that they can reference state regardless of declaration order.
         */
        for member in members {
            match member {
                ComponentMember::State(binding) | ComponentMember::Let(binding) => {
                    self.register_binding(binding);
                }

                _ => {}
            }
        }

        let component_scopes = self.scopes.clone();

        for member in members {
            self.scopes = component_scopes.clone();

            match member {
                ComponentMember::Recipe(recipe) => {
                    self.visit_block_statement(&recipe.body);
                }

                ComponentMember::Function(function) => {
                    self.visit_function(function);
                }

                _ => {}
            }
        }

        self.scopes = component_scopes;
        self.exit_scope();
    }

    fn visit_function(&mut self, function: &FunctionDeclaration) {
        self.enter_scope();

        for parameter in &function.params {
            self.declare_pattern(parameter, true);
        }

        if let Some(body) = &function.body {
            self.visit_block_statement(body);
        }

        self.exit_scope();
    }

    // -- binding/pattern visitors --

    fn register_binding(&mut self, binding: &Binding) {
        /*
         * The binding is visible while its initializer is evaluated, but it
         * is not initialized until evaluation of the initializer completes.
         */
        self.declare_pattern(&binding.pattern, false);

        if let Some(initializer) = &binding.init {
            self.visit_expression(initializer);
            self.mark_pattern_initialized(&binding.pattern);
        }
    }

    fn declare_pattern(&mut self, pattern: &Pattern, initialized: bool) {
        if let Pattern::Ident(identifier) = pattern {
            self.declare(&identifier.name, initialized);
        }
    }

    fn mark_pattern_initialized(&mut self, pattern: &Pattern) {
        if let Pattern::Ident(identifier) = pattern {
            self.mark_initialized(&identifier.name);
        }
    }

    fn declare_is_pattern(&mut self, pattern: &IsPattern) {
        if let IsPattern::Ident(identifier) = pattern {
            self.declare(&identifier.name, true);
        }
    }

    // -- statement visitors --

    fn visit_block_statement(&mut self, block: &BlockStatement) {
        self.enter_scope();

        for statement in &block.statements {
            self.visit_statement(statement);
        }

        self.exit_scope();
    }

    fn visit_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Block(block) => self.visit_block_statement(block),

            Statement::Expression(statement) => {
                self.visit_expression(&statement.expression);
            }

            Statement::Let(binding) => self.register_binding(binding),

            Statement::If(if_statement) => self.visit_if_statement(if_statement),

            Statement::While(while_statement) => {
                self.visit_while_statement(while_statement);
            }

            Statement::ForIn(for_in) => self.visit_for_in_statement(for_in),

            Statement::Return(return_statement) => {
                if let Some(argument) = &return_statement.argument {
                    self.visit_expression(argument);
                }
            }

            Statement::Is(is_statement) => self.visit_is_statement(is_statement),

            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Empty(_)
            | Statement::Declaration(_) => {}
        }
    }

    fn visit_if_statement(&mut self, if_statement: &IfStatement) {
        self.visit_expression(&if_statement.test);

        let before = self.scopes.clone();

        self.scopes = before.clone();
        self.visit_statement(&if_statement.consequent);
        let consequent = self.scopes.clone();

        let alternative = if let Some(alternative) = &if_statement.alternative {
            self.scopes = before.clone();
            self.visit_statement(alternative);
            self.scopes.clone()
        } else {
            before.clone()
        };

        self.scopes = before;

        self.merge_branches(&consequent, &alternative);
    }

    fn visit_while_statement(&mut self, while_statement: &WhileStatement) {
        self.visit_expression(&while_statement.test);

        /*
         * A while loop may execute zero times, so assignments in its body
         * cannot definitely initialize variables after the loop.
         */
        let before = self.scopes.clone();

        self.visit_statement(&while_statement.body);

        self.scopes = before;
    }

    fn visit_for_in_statement(&mut self, for_in: &ForInStatement) {
        self.visit_expression(&for_in.right);

        /*
         * A for loop may execute zero times, so assignments in its body
         * cannot definitely initialize variables after the loop.
         */
        let before = self.scopes.clone();

        self.enter_scope();
        self.declare_pattern(&for_in.pattern, true);
        self.visit_statement(&for_in.body);
        self.exit_scope();

        self.scopes = before;
    }

    fn visit_is_statement(&mut self, is_statement: &IsStatement) {
        if let Some(value) = &is_statement.value {
            self.visit_expression(value);
        }

        /*
         * The body only executes when the pattern matches, so assignments
         * performed inside it cannot definitely initialize outer variables.
         */
        let before = self.scopes.clone();

        self.enter_scope();
        self.declare_is_pattern(&is_statement.pattern);
        self.visit_statement(&is_statement.body);
        self.exit_scope();

        self.scopes = before;
    }

    // -- expression visitors --

    fn visit_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::Literal(_) | Expression::DotIdent(_) => {}

            Expression::Ident(identifier) => {
                self.check_read(&identifier.name, identifier.span);
            }

            Expression::Unary(unary) => {
                self.visit_expression(&unary.argument);
            }

            Expression::Binary(binary) => {
                self.visit_expression(&binary.left);
                self.visit_expression(&binary.right);
            }

            Expression::Call(call) => {
                self.visit_expression(&call.callee);

                for argument in &call.args {
                    match argument {
                        CallArg::Positional(expression) => {
                            self.visit_expression(expression);
                        }

                        CallArg::Named { value, .. } => {
                            self.visit_expression(value);
                        }
                    }
                }
            }

            Expression::Member(member) => {
                self.visit_expression(&member.object);
            }

            Expression::Index(index) => {
                self.visit_expression(&index.object);
                self.visit_expression(&index.index);
            }

            Expression::Assign(assignment) => {
                self.visit_assignment_expression(assignment);
            }

            Expression::Group(group) => {
                self.visit_expression(&group.expression);
            }

            Expression::Block(block) => {
                self.enter_scope();

                for statement in &block.statements {
                    self.visit_statement(statement);
                }

                if let Some(tail) = &block.tail {
                    self.visit_expression(tail);
                }

                self.exit_scope();
            }

            Expression::List(list) => {
                for element in &list.elems {
                    if let Some(element) = element {
                        self.visit_expression(element);
                    }
                }
            }

            Expression::Object(object) => {
                for property in &object.props {
                    let ObjectProperty::KeyValue(property) = property;
                    self.visit_expression(&property.value);
                }
            }

            Expression::Template(template) => {
                for part in &template.parts {
                    if let TemplatePart::Expression { expression, .. } = part {
                        self.visit_expression(expression);
                    }
                }
            }

            Expression::Closure(closure) => {
                /*
                 * A closure body is not executed when the closure is created,
                 * so assignments inside it do not initialize outer variables.
                 */
                let before = self.scopes.clone();

                self.enter_scope();

                for parameter in &closure.params {
                    self.declare_pattern(parameter, true);
                }

                self.visit_expression(&closure.body);

                self.exit_scope();

                self.scopes = before;
            }

            Expression::Is(is_expression) => {
                self.visit_expression(&is_expression.value);

                let before = self.scopes.clone();

                self.enter_scope();
                self.declare_is_pattern(&is_expression.pattern);
                self.visit_expression(&is_expression.body);
                self.exit_scope();

                self.scopes = before;
            }

            Expression::Component(component) => {
                for argument in &component.args {
                    match argument {
                        CallArg::Positional(expression) => {
                            self.visit_expression(expression);
                        }

                        CallArg::Named { value, .. } => {
                            self.visit_expression(value);
                        }
                    }
                }

                for child in &component.children {
                    self.visit_expression(child);
                }
            }
        }
    }

    fn visit_assignment_expression(&mut self, assignment: &AssignmentExpression) {
        if let Expression::Ident(identifier) = assignment.target.as_ref() {
            match assignment.op {
                AssignOp::Assign => {
                    /*
                     * The assigned value is evaluated before the target
                     * variable becomes initialized.
                     */
                    self.visit_expression(&assignment.value);
                    self.mark_initialized(&identifier.name);
                }

                AssignOp::AddAssign => {
                    /*
                     * Compound assignment reads the previous value before
                     * storing the result.
                     */
                    self.check_read(&identifier.name, identifier.span);
                    self.visit_expression(&assignment.value);
                    self.mark_initialized(&identifier.name);
                }
            }

            return;
        }

        self.visit_expression(&assignment.target);
        self.visit_expression(&assignment.value);
    }

    // -- initialization state --

    fn declare(&mut self, name: &str, initialized: bool) {
        let scope = self.scopes.last_mut().expect("scope stack is never empty");

        scope.insert(name.to_owned(), initialized);
    }

    fn check_read(&mut self, name: &str, span: Span) {
        let initialized = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .copied();

        if initialized == Some(false) {
            self.errors.push(InitializationError {
                name: name.to_owned(),
                span,
            });
        }
    }

    fn mark_initialized(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(initialized) = scope.get_mut(name) {
                *initialized = true;
                return;
            }
        }
    }

    fn merge_branches(
        &mut self,
        consequent: &[HashMap<String, bool>],
        alternative: &[HashMap<String, bool>],
    ) {
        for (index, scope) in self.scopes.iter_mut().enumerate() {
            for (name, initialized) in scope.iter_mut() {
                if *initialized {
                    continue;
                }

                let consequent_initialized = consequent
                    .get(index)
                    .and_then(|scope| scope.get(name))
                    .copied()
                    .unwrap_or(false);

                let alternative_initialized = alternative
                    .get(index)
                    .and_then(|scope| scope.get(name))
                    .copied()
                    .unwrap_or(false);

                *initialized = consequent_initialized && alternative_initialized;
            }
        }
    }

    // -- scope management --

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }
}
