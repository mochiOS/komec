use std::collections::HashMap;
use std::fmt;

use kome_ast::declarations::{
    Binding, ComponentDeclaration, ComponentMember, Declaration, FunctionDeclaration, Module,
};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, BinaryExpression, BinaryOp, CallArg, CallExpression,
    ComponentExpression, Expression, LiteralKind, UnaryOp,
};
use kome_ast::patterns::Pattern;
use kome_ast::statements::{
    BlockStatement, ForInStatement, IfStatement, ReturnStatement, Statement, WhileStatement,
};
use kome_ast::types::{PrimitiveTypeKind, Type};
use kome_ast::{AstNode, Span};

/// A type used during semantic type checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticType {
    Number,
    String,
    Bool,
    Null,
    Named(String),
    Optional(Box<SemanticType>),
    Void,

    /// A type that cannot be determined by the current type-checking pass.
    ///
    /// This is used for language features whose full type semantics have not
    /// been implemented yet. It must not be used for unresolved named types:
    /// external types such as ViewKit's `Color` are represented by [`Self::Named`].
    Unknown,
}

impl SemanticType {
    /// Returns the source-level name of this type.
    pub fn name(&self) -> String {
        match self {
            Self::Number => "Number".to_owned(),
            Self::String => "String".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Null => "Null".to_owned(),
            Self::Named(name) => name.clone(),
            Self::Optional(inner) => format!("{}?", inner.name()),
            Self::Void => "Void".to_owned(),
            Self::Unknown => "<unknown>".to_owned(),
        }
    }
}

/// An error produced while checking Kome types.
#[derive(Debug, Clone)]
pub struct TypeCheckError {
    pub message: String,
    pub span: Span,
}

impl fmt::Display for TypeCheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at byte range {}..{}",
            self.message, self.span.start, self.span.end,
        )
    }
}

impl std::error::Error for TypeCheckError {}

/// The result of semantic type checking.
#[derive(Debug, Clone)]
pub struct TypeCheckResult {
    pub errors: Vec<TypeCheckError>,
}

#[derive(Debug, Clone)]
struct ParameterType {
    name: String,
    type_: SemanticType,
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    params: Vec<ParameterType>,
    return_type: SemanticType,
}

#[derive(Debug, Clone)]
struct ComponentSignature {
    params: Vec<ParameterType>,
}

/// Walks a Kome AST and checks expression, binding, assignment, and function types.
///
/// Built-in Kome types are resolved directly. Named types are preserved without
/// requiring the defining package to be implemented by the compiler itself.
///
/// For example, ViewKit may define:
///
/// ```kome
/// component Text(
///     color: Color = .primary,
/// )
/// ```
///
/// The compiler represents `Color` as a named type. The `.primary` expression
/// can then be resolved from its expected `Color` type.
pub struct TypeChecker {
    scopes: Vec<HashMap<String, SemanticType>>,
    functions: HashMap<String, FunctionSignature>,
    components: HashMap<String, ComponentSignature>,
    return_type: SemanticType,
    errors: Vec<TypeCheckError>,
}

impl TypeChecker {
    /// Runs semantic type checking on a parsed [`Module`].
    pub fn check(module: &Module) -> TypeCheckResult {
        let mut checker = Self {
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            components: HashMap::new(),
            return_type: SemanticType::Void,
            errors: Vec::new(),
        };

        checker.collect_declarations(module);
        checker.visit_module(module);

        TypeCheckResult {
            errors: checker.errors,
        }
    }

    // -- declaration collection --

    fn collect_declarations(&mut self, module: &Module) {
        for declaration in &module.declarations {
            match declaration {
                Declaration::Function(function) => {
                    self.collect_function(function);
                }

                Declaration::Component(component) => {
                    self.collect_component(component);
                }

                _ => {}
            }
        }
    }

    fn collect_function(&mut self, function: &FunctionDeclaration) {
        let mut params = Vec::new();

        for pattern in &function.params {
            let Pattern::Ident(identifier) = pattern else {
                continue;
            };

            let type_ = identifier
                .type_annotation
                .as_ref()
                .map(Self::type_from_annotation)
                .unwrap_or(SemanticType::Unknown);

            params.push(ParameterType {
                name: identifier.name.clone(),
                type_,
            });
        }

        let return_type = function
            .return_type
            .as_ref()
            .map(Self::type_from_annotation)
            .unwrap_or(SemanticType::Void);

        self.functions.insert(
            function.name.clone(),
            FunctionSignature {
                params,
                return_type,
            },
        );
    }

    fn collect_component(&mut self, component: &ComponentDeclaration) {
        let params = component
            .params
            .iter()
            .map(|parameter| ParameterType {
                name: parameter.name.clone(),
                type_: Self::type_from_annotation(&parameter.type_),
            })
            .collect();

        self.components
            .insert(component.name.clone(), ComponentSignature { params });
    }

    // -- module visitor --

    fn visit_module(&mut self, module: &Module) {
        for declaration in &module.declarations {
            match declaration {
                Declaration::Function(function) => {
                    self.visit_function(function);
                }

                Declaration::Component(component) => {
                    self.visit_component(component);
                }

                Declaration::Constant(binding) => {
                    self.register_binding(binding);
                }

                _ => {}
            }
        }
    }

    // -- declaration visitors --

    fn visit_component(&mut self, component: &ComponentDeclaration) {
        self.enter_scope();

        for parameter in &component.params {
            let type_ = Self::type_from_annotation(&parameter.type_);

            self.declare(&parameter.name, type_.clone());

            if let Some(default) = &parameter.default {
                let actual = self.infer_expression(default, Some(&type_));

                self.check_compatible(&type_, &actual, default.span());
            }
        }

        if let Some(members) = &component.body {
            for member in members {
                match member {
                    ComponentMember::State(binding) | ComponentMember::Let(binding) => {
                        self.register_binding(binding);
                    }

                    ComponentMember::Recipe(recipe) => {
                        self.visit_block_statement(&recipe.body);
                    }

                    ComponentMember::Function(function) => {
                        self.visit_function(function);
                    }
                }
            }
        }

        self.exit_scope();
    }

    fn visit_function(&mut self, function: &FunctionDeclaration) {
        self.enter_scope();

        for pattern in &function.params {
            let Pattern::Ident(identifier) = pattern else {
                continue;
            };

            let type_ = identifier
                .type_annotation
                .as_ref()
                .map(Self::type_from_annotation)
                .unwrap_or(SemanticType::Unknown);

            self.declare(&identifier.name, type_);
        }

        let previous_return_type = self.return_type.clone();

        self.return_type = function
            .return_type
            .as_ref()
            .map(Self::type_from_annotation)
            .unwrap_or(SemanticType::Void);

        if let Some(body) = &function.body {
            self.visit_block_statement(body);
        }

        self.return_type = previous_return_type;

        self.exit_scope();
    }

    // -- binding visitors --

    fn register_binding(&mut self, binding: &Binding) {
        let Pattern::Ident(identifier) = &binding.pattern else {
            return;
        };

        let annotated = binding
            .type_annotation
            .as_ref()
            .map(Self::type_from_annotation);

        /*
         * Declare the binding before checking its initializer so other
         * semantic passes can retain the same lexical binding behavior.
         */
        self.declare(
            &identifier.name,
            annotated.clone().unwrap_or(SemanticType::Unknown),
        );

        let inferred = binding
            .init
            .as_ref()
            .map(|initializer| self.infer_expression(initializer, annotated.as_ref()));

        let final_type = match (annotated, inferred) {
            (Some(expected), Some(actual)) => {
                self.check_compatible(&expected, &actual, binding.span);

                expected
            }

            (Some(expected), None) => expected,

            (None, Some(actual)) => actual,

            (None, None) => SemanticType::Unknown,
        };

        self.update(&identifier.name, final_type);
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
            Statement::Block(block) => {
                self.visit_block_statement(block);
            }

            Statement::Expression(statement) => {
                self.infer_expression(&statement.expression, None);
            }

            Statement::Let(binding) => {
                self.register_binding(binding);
            }

            Statement::If(if_statement) => {
                self.visit_if_statement(if_statement);
            }

            Statement::While(while_statement) => {
                self.visit_while_statement(while_statement);
            }

            Statement::ForIn(for_in) => {
                self.visit_for_in_statement(for_in);
            }

            Statement::Return(return_statement) => {
                self.visit_return_statement(return_statement);
            }

            Statement::Is(is_statement) => {
                if let Some(value) = &is_statement.value {
                    self.infer_expression(value, None);
                }

                self.visit_statement(&is_statement.body);
            }

            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Empty(_)
            | Statement::Declaration(_) => {}
        }
    }

    fn visit_if_statement(&mut self, if_statement: &IfStatement) {
        let condition = self.infer_expression(&if_statement.test, Some(&SemanticType::Bool));

        self.check_compatible(&SemanticType::Bool, &condition, if_statement.test.span());

        self.visit_statement(&if_statement.consequent);

        if let Some(alternative) = &if_statement.alternative {
            self.visit_statement(alternative);
        }
    }

    fn visit_while_statement(&mut self, while_statement: &WhileStatement) {
        let condition = self.infer_expression(&while_statement.test, Some(&SemanticType::Bool));

        self.check_compatible(&SemanticType::Bool, &condition, while_statement.test.span());

        self.visit_statement(&while_statement.body);
    }

    fn visit_for_in_statement(&mut self, for_in: &ForInStatement) {
        self.infer_expression(&for_in.right, None);

        self.enter_scope();

        if let Pattern::Ident(identifier) = &for_in.pattern {
            self.declare(&identifier.name, SemanticType::Unknown);
        }

        self.visit_statement(&for_in.body);

        self.exit_scope();
    }

    fn visit_return_statement(&mut self, return_statement: &ReturnStatement) {
        match &return_statement.argument {
            Some(argument) => {
                let expected = self.return_type.clone();
                let actual = self.infer_expression(argument, Some(&expected));

                self.check_compatible(&expected, &actual, argument.span());
            }

            None => {
                let expected = self.return_type.clone();

                self.check_compatible(&expected, &SemanticType::Void, return_statement.span);
            }
        }
    }

    // -- expression inference --

    fn infer_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&SemanticType>,
    ) -> SemanticType {
        match expression {
            Expression::Literal(literal) => match &literal.kind {
                LiteralKind::String(_) => SemanticType::String,
                LiteralKind::Number(_) | LiteralKind::Percent(_) => SemanticType::Number,
                LiteralKind::Boolean(_) => SemanticType::Bool,
                LiteralKind::Null => SemanticType::Null,
            },

            Expression::Ident(identifier) => self
                .resolve(&identifier.name)
                .unwrap_or(SemanticType::Unknown),

            Expression::Unary(unary) => match unary.op {
                UnaryOp::Not => {
                    let argument =
                        self.infer_expression(&unary.argument, Some(&SemanticType::Bool));

                    self.check_compatible(&SemanticType::Bool, &argument, unary.argument.span());

                    SemanticType::Bool
                }
            },

            Expression::Binary(binary) => self.infer_binary_expression(binary),

            Expression::Call(call) => self.infer_call_expression(call),

            Expression::Assign(assignment) => self.infer_assignment_expression(assignment),

            Expression::Group(group) => self.infer_expression(&group.expression, expected),

            Expression::Block(block) => {
                self.enter_scope();

                for statement in &block.statements {
                    self.visit_statement(statement);
                }

                let type_ = block
                    .tail
                    .as_ref()
                    .map(|tail| self.infer_expression(tail, expected))
                    .unwrap_or(SemanticType::Void);

                self.exit_scope();

                type_
            }

            Expression::Template(template) => {
                for part in &template.parts {
                    if let kome_ast::expressions::TemplatePart::Expression { expression, .. } = part
                    {
                        self.infer_expression(expression, None);
                    }
                }

                SemanticType::String
            }

            Expression::DotIdent(_) => expected.cloned().unwrap_or(SemanticType::Unknown),

            Expression::Component(component) => self.infer_component_expression(component),

            /*
             * Full typing for these expression kinds depends on later
             * collection, object, closure, and member-resolution work.
             */
            Expression::Member(member) => {
                self.infer_expression(&member.object, None);

                SemanticType::Unknown
            }

            Expression::Index(index) => {
                self.infer_expression(&index.object, None);
                self.infer_expression(&index.index, None);

                SemanticType::Unknown
            }

            Expression::List(list) => {
                for element in &list.elems {
                    if let Some(element) = element {
                        self.infer_expression(element, None);
                    }
                }

                SemanticType::Unknown
            }

            Expression::Object(object) => {
                for property in &object.props {
                    let kome_ast::expressions::ObjectProperty::KeyValue(property) = property;

                    self.infer_expression(&property.value, None);
                }

                SemanticType::Unknown
            }

            Expression::Closure(closure) => {
                self.enter_scope();

                for parameter in &closure.params {
                    if let Pattern::Ident(identifier) = parameter {
                        let type_ = identifier
                            .type_annotation
                            .as_ref()
                            .map(Self::type_from_annotation)
                            .unwrap_or(SemanticType::Unknown);

                        self.declare(&identifier.name, type_);
                    }
                }

                self.infer_expression(&closure.body, None);

                self.exit_scope();

                SemanticType::Unknown
            }

            Expression::Is(is_expression) => {
                self.infer_expression(&is_expression.value, None);
                self.infer_expression(&is_expression.body, expected)
            }
        }
    }

    fn infer_binary_expression(&mut self, binary: &BinaryExpression) -> SemanticType {
        match binary.op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let left = self.infer_expression(&binary.left, Some(&SemanticType::Number));
                let right = self.infer_expression(&binary.right, Some(&SemanticType::Number));

                self.check_compatible(&SemanticType::Number, &left, binary.left.span());

                self.check_compatible(&SemanticType::Number, &right, binary.right.span());

                SemanticType::Number
            }

            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                let left = self.infer_expression(&binary.left, Some(&SemanticType::Number));
                let right = self.infer_expression(&binary.right, Some(&SemanticType::Number));

                self.check_compatible(&SemanticType::Number, &left, binary.left.span());

                self.check_compatible(&SemanticType::Number, &right, binary.right.span());

                SemanticType::Bool
            }

            BinaryOp::And | BinaryOp::Or => {
                let left = self.infer_expression(&binary.left, Some(&SemanticType::Bool));
                let right = self.infer_expression(&binary.right, Some(&SemanticType::Bool));

                self.check_compatible(&SemanticType::Bool, &left, binary.left.span());

                self.check_compatible(&SemanticType::Bool, &right, binary.right.span());

                SemanticType::Bool
            }

            BinaryOp::Eq | BinaryOp::NotEq => {
                let left = self.infer_expression(&binary.left, None);
                let right = self.infer_expression(&binary.right, Some(&left));

                if !self.types_compatible(&left, &right) && !self.types_compatible(&right, &left) {
                    self.push_mismatch(&left, &right, binary.span);
                }

                SemanticType::Bool
            }
        }
    }

    fn infer_assignment_expression(&mut self, assignment: &AssignmentExpression) -> SemanticType {
        let Expression::Ident(identifier) = assignment.target.as_ref() else {
            self.infer_expression(&assignment.target, None);
            self.infer_expression(&assignment.value, None);

            return SemanticType::Unknown;
        };

        let target_type = self
            .resolve(&identifier.name)
            .unwrap_or(SemanticType::Unknown);

        match assignment.op {
            AssignOp::Assign => {
                let value = self.infer_expression(&assignment.value, Some(&target_type));

                self.check_compatible(&target_type, &value, assignment.value.span());
            }

            AssignOp::AddAssign => {
                self.check_compatible(&SemanticType::Number, &target_type, identifier.span);

                let value = self.infer_expression(&assignment.value, Some(&SemanticType::Number));

                self.check_compatible(&SemanticType::Number, &value, assignment.value.span());
            }
        }

        target_type
    }

    fn infer_call_expression(&mut self, call: &CallExpression) -> SemanticType {
        let Expression::Ident(identifier) = call.callee.as_ref() else {
            self.infer_expression(&call.callee, None);

            for argument in &call.args {
                match argument {
                    CallArg::Positional(expression) => {
                        self.infer_expression(expression, None);
                    }

                    CallArg::Named { value, .. } => {
                        self.infer_expression(value, None);
                    }
                }
            }

            return SemanticType::Unknown;
        };

        let Some(signature) = self.functions.get(&identifier.name).cloned() else {
            return SemanticType::Unknown;
        };

        for (index, argument) in call.args.iter().enumerate() {
            let parameter = match argument {
                CallArg::Positional(_) => signature.params.get(index),

                CallArg::Named { name, .. } => signature
                    .params
                    .iter()
                    .find(|parameter| parameter.name == *name),
            };

            let expression = match argument {
                CallArg::Positional(expression) => expression,
                CallArg::Named { value, .. } => value,
            };

            let Some(parameter) = parameter else {
                self.infer_expression(expression, None);
                continue;
            };

            let actual = self.infer_expression(expression, Some(&parameter.type_));

            self.check_compatible(&parameter.type_, &actual, expression.span());
        }

        signature.return_type
    }

    fn infer_component_expression(&mut self, component: &ComponentExpression) -> SemanticType {
        let Some(signature) = self.components.get(&component.name).cloned() else {
            for argument in &component.args {
                match argument {
                    CallArg::Positional(expression) => {
                        self.infer_expression(expression, None);
                    }

                    CallArg::Named { value, .. } => {
                        self.infer_expression(value, None);
                    }
                }
            }

            for child in &component.children {
                self.infer_expression(child, None);
            }

            return SemanticType::Unknown;
        };

        for (index, argument) in component.args.iter().enumerate() {
            let parameter = match argument {
                CallArg::Positional(_) => signature.params.get(index),

                CallArg::Named { name, .. } => signature
                    .params
                    .iter()
                    .find(|parameter| parameter.name == *name),
            };

            let expression = match argument {
                CallArg::Positional(expression) => expression,
                CallArg::Named { value, .. } => value,
            };

            let Some(parameter) = parameter else {
                self.infer_expression(expression, None);
                continue;
            };

            let actual = self.infer_expression(expression, Some(&parameter.type_));

            self.check_compatible(&parameter.type_, &actual, expression.span());
        }

        for child in &component.children {
            self.infer_expression(child, None);
        }

        SemanticType::Unknown
    }

    // -- type conversion --

    fn type_from_annotation(type_: &Type) -> SemanticType {
        match type_ {
            Type::Primitive(primitive) => match primitive.kind {
                PrimitiveTypeKind::String => SemanticType::String,
                PrimitiveTypeKind::Number => SemanticType::Number,
                PrimitiveTypeKind::Bool => SemanticType::Bool,
                PrimitiveTypeKind::Null => SemanticType::Null,
            },

            Type::Named(named) => SemanticType::Named(named.name.clone()),

            Type::Optional(optional) => {
                SemanticType::Optional(Box::new(Self::type_from_annotation(&optional.inner)))
            }

            _ => SemanticType::Unknown,
        }
    }

    // -- type compatibility --

    fn check_compatible(&mut self, expected: &SemanticType, actual: &SemanticType, span: Span) {
        if !self.types_compatible(expected, actual) {
            self.push_mismatch(expected, actual, span);
        }
    }

    fn types_compatible(&self, expected: &SemanticType, actual: &SemanticType) -> bool {
        if matches!(expected, SemanticType::Unknown) || matches!(actual, SemanticType::Unknown) {
            return true;
        }

        if expected == actual {
            return true;
        }

        match expected {
            SemanticType::Optional(inner) => {
                actual == &SemanticType::Null || self.types_compatible(inner, actual)
            }

            _ => false,
        }
    }

    fn push_mismatch(&mut self, expected: &SemanticType, actual: &SemanticType, span: Span) {
        self.errors.push(TypeCheckError {
            message: format!("expected {}, but found {}", expected.name(), actual.name(),),
            span,
        });
    }

    // -- scope management --

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, type_: SemanticType) {
        let scope = self.scopes.last_mut().expect("scope stack is never empty");

        scope.insert(name.to_owned(), type_);
    }

    fn update(&mut self, name: &str, type_: SemanticType) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(existing) = scope.get_mut(name) {
                *existing = type_;
                return;
            }
        }
    }

    fn resolve(&self, name: &str) -> Option<SemanticType> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .cloned()
    }
}
