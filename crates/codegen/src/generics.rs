//! Generic specialization discovery and AST monomorphization.

use crate::{CodegenError, CodegenResult};
use kome_ast::AstNode;
use kome_ast::declarations::{
    Declaration, ForDeclaration, FunctionDeclaration, GenericParameter, Module, StructDeclaration,
    StructField, TraitDeclaration, TypeMember, Visibility,
};
use kome_ast::expressions::{
    BlockExpression, CallArg, CallExpression, ClosureExpression, ClosureLowering, Expression,
    IdentifierExpression, LiteralKind, MemberExpression, PropertyKey,
};
use kome_ast::generics::TypeSubstitution;
use kome_ast::patterns::{IdentifierPattern, Pattern};
use kome_ast::statements::{BlockStatement, ExpressionStatement, ReturnStatement, Statement};
use kome_ast::types::{FunctionType, NamedType, Parameter, Type};
use std::collections::{HashMap, HashSet};

pub(crate) fn monomorphize(module: &Module) -> CodegenResult<Module> {
    Expander::new(module).run()
}

struct Expander<'a> {
    module: &'a Module,
    structs: HashMap<String, &'a StructDeclaration>,
    functions: HashMap<String, &'a FunctionDeclaration>,
    traits: HashMap<String, &'a TraitDeclaration>,
    components: HashSet<String>,
    enums: HashSet<String>,
    implementations: Vec<&'a ForDeclaration>,
    output: Vec<Declaration>,
    emitted_structs: HashMap<String, String>,
    struct_origins: HashMap<String, (String, Vec<Type>)>,
    emitted_functions: HashSet<String>,
    emitted_traits: HashSet<String>,
    emitted_impls: HashSet<(usize, String)>,
    layout_stack: Vec<String>,
    next_task_body: usize,
    next_closure: usize,
}

impl<'a> Expander<'a> {
    fn new(module: &'a Module) -> Self {
        let mut structs = HashMap::new();
        let mut functions = HashMap::new();
        let mut traits = HashMap::new();
        let mut implementations = Vec::new();
        let mut enums = HashSet::new();
        let mut components = HashSet::new();
        for declaration in &module.declarations {
            match declaration {
                Declaration::Struct(value) => {
                    structs.insert(value.name.clone(), value);
                }
                Declaration::Function(value) => {
                    functions.insert(value.name.clone(), value);
                }
                Declaration::Trait(value) => {
                    traits.insert(value.name.clone(), value);
                }
                Declaration::For(value) => implementations.push(value),
                Declaration::Enum(value) => {
                    enums.insert(value.name.clone());
                }
                Declaration::Component(value) => {
                    components.insert(value.name.clone());
                }
                _ => {}
            }
        }
        Self {
            module,
            structs,
            functions,
            traits,
            components,
            enums,
            implementations,
            output: Vec::new(),
            emitted_structs: HashMap::new(),
            struct_origins: HashMap::new(),
            emitted_functions: HashSet::new(),
            emitted_traits: HashSet::new(),
            emitted_impls: HashSet::new(),
            layout_stack: Vec::new(),
            next_task_body: 0,
            next_closure: 0,
        }
    }

    fn run(mut self) -> CodegenResult<Module> {
        for declaration in &self.module.declarations {
            let parameters = match declaration {
                Declaration::Struct(value) => &value.type_parameters,
                Declaration::Function(value) => &value.type_parameters,
                Declaration::Trait(value) => &value.type_parameters,
                _ => continue,
            };
            let mut names = HashSet::new();
            for parameter in parameters {
                if !names.insert(&parameter.name) {
                    return Err(CodegenError::at(
                        format!("duplicate generic parameter `{}`", parameter.name),
                        parameter.span,
                    ));
                }
            }
        }
        for declaration in &self.module.declarations {
            match declaration {
                Declaration::Struct(value) if value.type_parameters.is_empty() => {
                    self.instantiate_struct(&value.name, &[], value.span)?;
                }
                Declaration::Function(value) if value.type_parameters.is_empty() => {
                    self.instantiate_function(&value.name, &[], value.span)?;
                }
                Declaration::Trait(value) if value.type_parameters.is_empty() => {
                    if self.emitted_traits.insert(value.name.clone()) {
                        self.output.push(declaration.clone());
                    }
                }
                Declaration::For(value) if value.type_parameters.is_empty() => {
                    self.emit_concrete_impl(value.clone(), &TypeSubstitution::default())?
                }
                Declaration::Struct(_)
                | Declaration::Function(_)
                | Declaration::Trait(_)
                | Declaration::For(_) => {}
                _ => self.output.push(declaration.clone()),
            }
        }
        Ok(Module::new(self.output, self.module.span))
    }

    fn names(parameters: &[GenericParameter]) -> impl Iterator<Item = &str> {
        parameters.iter().map(|parameter| parameter.name.as_str())
    }
    fn encode_type(type_: &Type) -> String {
        match type_ {
            Type::Primitive(value) => {
                let name = format!("{:?}", value.kind);
                format!("P{}_{}", name.len(), name)
            }
            Type::Named(value) => {
                let arguments = value
                    .type_arguments
                    .iter()
                    .map(Self::encode_type)
                    .map(|value| format!("{}_{}", value.len(), value))
                    .collect::<String>();
                format!(
                    "N{}_{}_{}_{}",
                    value.name.len(),
                    value.name,
                    value.type_arguments.len(),
                    arguments
                )
            }
            Type::Optional(value) => {
                let inner = Self::encode_type(&value.inner);
                format!("O{}_{}", inner.len(), inner)
            }
            Type::List(value) => {
                let inner = Self::encode_type(&value.element);
                format!("L{}_{}", inner.len(), inner)
            }
            Type::Function(_) => "F".into(),
            Type::Object(_) => "R".into(),
            Type::Pointer(value) => format!(
                "P{}{}",
                match value.mutability {
                    kome_ast::types::PointerMutability::Const => "C",
                    kome_ast::types::PointerMutability::Mut => "M",
                },
                Self::encode_type(&value.pointee)
            ),
        }
    }
    fn specialized_name(name: &str, arguments: &[Type]) -> String {
        if arguments.is_empty() {
            name.into()
        } else {
            format!(
                "{}${}${}",
                name,
                arguments.len(),
                arguments
                    .iter()
                    .map(Self::encode_type)
                    .map(|value| format!("${}_{}", value.len(), value))
                    .collect::<String>()
            )
        }
    }

    fn instantiate_struct(
        &mut self,
        name: &str,
        arguments: &[Type],
        span: kome_ast::Span,
    ) -> CodegenResult<String> {
        let template = self
            .structs
            .get(name)
            .copied()
            .ok_or_else(|| {
                CodegenError::at(format!("generic struct `{name}` was not found"), span)
            })?
            .clone();
        if template.type_parameters.len() != arguments.len() {
            return Err(CodegenError::at(
                format!(
                    "struct `{name}` expects {} type argument(s), but received {}",
                    template.type_parameters.len(),
                    arguments.len()
                ),
                span,
            ));
        }
        let key = Self::specialized_name(name, arguments);
        if self.emitted_structs.contains_key(&key) {
            return Ok(key);
        }
        if self.layout_stack.contains(&key) {
            return Err(CodegenError::at(
                format!("unsupported recursive generic layout `{key}`"),
                span,
            ));
        }
        self.layout_stack.push(key.clone());
        let substitution = TypeSubstitution::new(Self::names(&template.type_parameters), arguments);
        let mut concrete = template.clone();
        concrete.name = key.clone();
        concrete.type_parameters.clear();
        if let Some(fields) = &mut concrete.fields {
            for field in fields {
                field.type_ = self.concrete_type(&field.type_, &substitution, field.span)?;
            }
        }
        self.layout_stack.pop();
        self.emitted_structs.insert(key.clone(), key.clone());
        self.struct_origins
            .insert(key.clone(), (name.to_owned(), arguments.to_vec()));
        self.output.push(Declaration::Struct(concrete));
        let implementations = self
            .module
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::For(value) => Some(value.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for implementation in implementations {
            if self.impl_matches(&implementation.target, name) {
                self.emit_concrete_impl(implementation, &substitution)?;
            }
        }
        Ok(key)
    }

    fn impl_matches(&self, target: &Type, name: &str) -> bool {
        matches!(target, Type::Named(named) if named.name == name)
    }
    fn concrete_type(
        &mut self,
        type_: &Type,
        substitution: &TypeSubstitution,
        span: kome_ast::Span,
    ) -> CodegenResult<Type> {
        let applied = substitution.apply(type_);
        match applied {
            Type::Named(mut named) if named.name == "Task" => {
                if named.type_arguments.len() != 1 {
                    return Err(CodegenError::at(
                        format!(
                            "type `Task` expects 1 type argument, but received {}",
                            named.type_arguments.len()
                        ),
                        span,
                    ));
                }
                let argument = self.concrete_type(
                    &named.type_arguments[0],
                    substitution,
                    named.type_arguments[0].span(),
                )?;
                named.type_arguments = vec![argument];
                Ok(Type::Named(named))
            }
            Type::Named(mut named) if self.structs.contains_key(&named.name) => {
                let original = named.name.clone();
                let args = named.type_arguments.clone();
                named.name = self.instantiate_struct(&original, &args, span)?;
                named.type_arguments.clear();
                Ok(Type::Named(named))
            }
            Type::Named(named) if self.emitted_structs.contains_key(&named.name) => {
                Ok(Type::Named(named))
            }
            Type::Named(named) if named.name == "Void" && named.type_arguments.is_empty() => {
                Ok(Type::Named(named))
            }
            Type::Named(named)
                if substitution.get(&named.name).is_none()
                    && named.type_arguments.is_empty()
                    && !self.structs.contains_key(&named.name)
                    && !self.traits.contains_key(&named.name)
                    && !self.enums.contains(&named.name) =>
            {
                Err(CodegenError::at(
                    format!(
                        "unresolved generic type `{}` during code generation",
                        named.name
                    ),
                    span,
                ))
            }
            other => Ok(other),
        }
    }

    fn instantiate_function(
        &mut self,
        name: &str,
        arguments: &[Type],
        span: kome_ast::Span,
    ) -> CodegenResult<String> {
        let template = self
            .functions
            .get(name)
            .copied()
            .ok_or_else(|| CodegenError::at(format!("function `{name}` was not found"), span))?
            .clone();
        if template.type_parameters.len() != arguments.len() {
            return Err(CodegenError::at(
                format!(
                    "function `{name}` expects {} type argument(s), but received {}",
                    template.type_parameters.len(),
                    arguments.len()
                ),
                span,
            ));
        }
        let key = Self::specialized_name(name, arguments);
        if !self.emitted_functions.insert(key.clone()) {
            return Ok(key);
        }
        let substitution = TypeSubstitution::new(Self::names(&template.type_parameters), arguments);
        let mut concrete = template.clone();
        concrete.name = key.clone();
        concrete.type_parameters.clear();
        for pattern in &mut concrete.params {
            if let Pattern::Ident(identifier) = pattern
                && let Some(annotation) = &identifier.type_annotation
            {
                identifier.type_annotation =
                    Some(self.concrete_type(annotation, &substitution, identifier.span)?);
            }
        }
        if let Some(return_type) = &concrete.return_type {
            concrete.return_type =
                Some(self.concrete_type(return_type, &substitution, return_type.span())?);
        }
        let mut environment = HashMap::new();
        for pattern in &concrete.params {
            if let Pattern::Ident(identifier) = pattern
                && let Some(type_) = &identifier.type_annotation
            {
                environment.insert(identifier.name.clone(), type_.clone());
            }
        }
        if let Some(body) = &mut concrete.body {
            self.rewrite_block(body, &mut environment, &substitution)?;
        }
        self.output.push(Declaration::Function(concrete));
        Ok(key)
    }

    fn emit_concrete_impl(
        &mut self,
        mut implementation: ForDeclaration,
        substitution: &TypeSubstitution,
    ) -> CodegenResult<()> {
        implementation.type_parameters.clear();
        implementation.target =
            self.concrete_type(&implementation.target, substitution, implementation.span)?;
        if let Some(trait_) = &implementation.trait_.clone() {
            implementation.trait_ =
                Some(self.concrete_trait(trait_, substitution, implementation.span)?);
        }
        let implementation_key = (
            implementation.span.start,
            Self::encode_type(&implementation.target),
        );
        if !self.emitted_impls.insert(implementation_key) {
            return Ok(());
        }
        let self_type = implementation.target.clone();
        for member in &mut implementation.members {
            match member {
                TypeMember::Function(function) => {
                    function.type_parameters.clear();
                    for pattern in &mut function.params {
                        if let Pattern::Ident(identifier) = pattern {
                            if identifier.name == "self" && identifier.type_annotation.is_none() {
                                identifier.type_annotation = Some(self_type.clone());
                            } else if let Some(annotation) = &identifier.type_annotation {
                                identifier.type_annotation = Some(self.concrete_type(
                                    annotation,
                                    substitution,
                                    identifier.span,
                                )?);
                            }
                        }
                    }
                    if let Some(return_type) = &function.return_type {
                        function.return_type = Some(self.concrete_type(
                            return_type,
                            substitution,
                            return_type.span(),
                        )?);
                    }
                    let mut environment = HashMap::new();
                    for pattern in &function.params {
                        if let Pattern::Ident(identifier) = pattern {
                            environment.insert(
                                identifier.name.clone(),
                                identifier
                                    .type_annotation
                                    .clone()
                                    .unwrap_or_else(|| self_type.clone()),
                            );
                        }
                    }
                    if let Some(body) = &mut function.body {
                        self.rewrite_block(body, &mut environment, substitution)?;
                    }
                }
                TypeMember::Constant(binding) => {
                    if let Some(type_) = &binding.type_annotation {
                        binding.type_annotation =
                            Some(self.concrete_type(type_, substitution, binding.span)?);
                    }
                }
            }
        }
        self.output.push(Declaration::For(implementation));
        Ok(())
    }

    fn concrete_trait(
        &mut self,
        type_: &Type,
        substitution: &TypeSubstitution,
        span: kome_ast::Span,
    ) -> CodegenResult<Type> {
        let applied = substitution.apply(type_);
        let Type::Named(mut named) = applied else {
            return Ok(applied);
        };
        let Some(template) = self.traits.get(&named.name).copied().cloned() else {
            return Ok(Type::Named(named));
        };
        if template.type_parameters.len() != named.type_arguments.len() {
            return Err(CodegenError::at(
                format!(
                    "trait `{}` expects {} type argument(s), but received {}",
                    named.name,
                    template.type_parameters.len(),
                    named.type_arguments.len()
                ),
                span,
            ));
        }
        let key = Self::specialized_name(&named.name, &named.type_arguments);
        if self.emitted_traits.insert(key.clone()) {
            let trait_substitution = TypeSubstitution::new(
                Self::names(&template.type_parameters),
                &named.type_arguments,
            );
            let mut concrete = template;
            concrete.name = key.clone();
            concrete.type_parameters.clear();
            for function in &mut concrete.functions {
                for pattern in &mut function.params {
                    if let Pattern::Ident(identifier) = pattern
                        && let Some(annotation) = &identifier.type_annotation
                    {
                        identifier.type_annotation = Some(self.concrete_type(
                            annotation,
                            &trait_substitution,
                            identifier.span,
                        )?);
                    }
                }
                if let Some(ret) = &function.return_type {
                    function.return_type =
                        Some(self.concrete_type(ret, &trait_substitution, ret.span())?);
                }
            }
            self.output.push(Declaration::Trait(concrete));
        }
        named.name = key;
        named.type_arguments.clear();
        Ok(Type::Named(named))
    }

    fn rewrite_block(
        &mut self,
        block: &mut BlockStatement,
        environment: &mut HashMap<String, Type>,
        substitution: &TypeSubstitution,
    ) -> CodegenResult<()> {
        for statement in &mut block.statements {
            self.rewrite_statement(statement, environment, substitution)?;
        }
        Ok(())
    }

    fn rewrite_statement(
        &mut self,
        statement: &mut Statement,
        environment: &mut HashMap<String, Type>,
        substitution: &TypeSubstitution,
    ) -> CodegenResult<()> {
        match statement {
            Statement::Let(binding) => {
                let expected = binding
                    .type_annotation
                    .as_ref()
                    .map(|type_| substitution.apply(type_));
                let inferred = if let Some(init) = &mut binding.init {
                    Some(self.rewrite_expression(
                        init,
                        environment,
                        substitution,
                        expected.as_ref(),
                    )?)
                } else {
                    None
                };
                if let Pattern::Ident(identifier) = &binding.pattern {
                    environment.insert(
                        identifier.name.clone(),
                        expected
                            .or(inferred)
                            .unwrap_or_else(|| unknown_type(binding.span)),
                    );
                }
                if let Some(annotation) = &binding.type_annotation {
                    binding.type_annotation =
                        Some(self.concrete_type(annotation, substitution, binding.span)?);
                }
            }
            Statement::Expression(value) => {
                self.rewrite_expression(&mut value.expression, environment, substitution, None)?;
            }
            Statement::Return(value) => {
                if let Some(argument) = &mut value.argument {
                    self.rewrite_expression(argument, environment, substitution, None)?;
                }
            }
            Statement::Block(value) => {
                let mut nested = environment.clone();
                self.rewrite_block(value, &mut nested, substitution)?;
            }
            Statement::If(value) => {
                let boolean = primitive("bool", value.test.span());
                self.rewrite_expression(
                    &mut value.test,
                    environment,
                    substitution,
                    Some(&boolean),
                )?;
                let mut consequent_environment = environment.clone();
                self.rewrite_statement(
                    &mut value.consequent,
                    &mut consequent_environment,
                    substitution,
                )?;
                if let Some(alternative) = &mut value.alternative {
                    let mut alternative_environment = environment.clone();
                    self.rewrite_statement(
                        alternative,
                        &mut alternative_environment,
                        substitution,
                    )?;
                }
            }
            Statement::While(value) => {
                let boolean = primitive("bool", value.test.span());
                self.rewrite_expression(
                    &mut value.test,
                    environment,
                    substitution,
                    Some(&boolean),
                )?;
                let mut body_environment = environment.clone();
                self.rewrite_statement(&mut value.body, &mut body_environment, substitution)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn rewrite_expression(
        &mut self,
        expression: &mut Expression,
        environment: &mut HashMap<String, Type>,
        substitution: &TypeSubstitution,
        expected: Option<&Type>,
    ) -> CodegenResult<Type> {
        if let Expression::Component(component) = expression
            && !self.components.contains(&component.name)
        {
            *expression = trailing_closure_call(component.clone());
        }

        match expression {
            Expression::Literal(value) => Ok(match &value.kind {
                LiteralKind::String(_) => primitive("String", value.span),
                LiteralKind::Number(_) | LiteralKind::Percent(_) => primitive("Number", value.span),
                LiteralKind::Boolean(_) => primitive("bool", value.span),
                LiteralKind::Null => primitive("Null", value.span),
            }),
            Expression::Ident(value) => Ok(environment
                .get(&value.name)
                .cloned()
                .unwrap_or_else(|| unknown_type(value.span))),
            Expression::Group(value) => {
                self.rewrite_expression(&mut value.expression, environment, substitution, expected)
            }
            Expression::Block(value) => {
                let mut block_environment = environment.clone();
                for statement in &mut value.statements {
                    self.rewrite_statement(statement, &mut block_environment, substitution)?;
                }
                if let Some(tail) = &mut value.tail {
                    self.rewrite_expression(tail, &mut block_environment, substitution, expected)
                } else {
                    Ok(void_type(value.span))
                }
            }
            Expression::Task(value) => {
                let expected_result = expected.and_then(|expected| match expected {
                    Type::Named(named)
                        if named.name == "Task" && named.type_arguments.len() == 1 =>
                    {
                        named.type_arguments.first()
                    }
                    _ => None,
                });
                let inferred = self.rewrite_expression(
                    &mut value.argument,
                    environment,
                    substitution,
                    expected_result,
                )?;
                let inner = expected_result.cloned().unwrap_or(inferred);
                let inner = self.concrete_type(&inner, substitution, value.span)?;
                let mut captures = HashSet::new();
                collect_captures(&value.argument, environment, &mut captures);
                let mut captures = captures.into_iter().collect::<Vec<_>>();
                captures.sort();

                let name = format!(
                    "__kome_task_body_{}_{}",
                    value.span.start, self.next_task_body
                );
                self.next_task_body += 1;
                let body_expression = value.argument.as_ref().clone();
                let params = captures
                    .iter()
                    .map(|capture| {
                        Pattern::Ident(IdentifierPattern {
                            span: value.span,
                            name: capture.clone(),
                            type_annotation: environment.get(capture).cloned(),
                            default: None,
                        })
                    })
                    .collect();
                let statement = if matches!(
                    &inner,
                    Type::Named(named) if named.name == "Void" && named.type_arguments.is_empty()
                ) {
                    Statement::Expression(ExpressionStatement {
                        span: value.span,
                        expression: body_expression,
                    })
                } else {
                    Statement::Return(ReturnStatement {
                        span: value.span,
                        argument: Some(body_expression),
                    })
                };
                self.output.push(Declaration::Function(FunctionDeclaration {
                    span: value.span,
                    visibility: kome_ast::declarations::Visibility::Private,
                    attributes: Vec::new(),
                    name: name.clone(),
                    type_parameters: Vec::new(),
                    params,
                    body: Some(BlockStatement {
                        span: value.span,
                        statements: vec![statement],
                    }),
                    return_type: Some(inner.clone()),
                }));
                value.argument = Box::new(Expression::Call(CallExpression {
                    span: value.span,
                    callee: Box::new(Expression::Ident(IdentifierExpression {
                        span: value.span,
                        name,
                    })),
                    type_arguments: Vec::new(),
                    args: captures
                        .into_iter()
                        .map(|name| {
                            CallArg::Positional(Expression::Ident(IdentifierExpression {
                                span: value.span,
                                name,
                            }))
                        })
                        .collect(),
                }));
                Ok(Type::Named(NamedType {
                    span: value.span,
                    name: "Task".into(),
                    type_arguments: vec![inner],
                }))
            }
            Expression::Wait(value) => {
                let task =
                    self.rewrite_expression(&mut value.argument, environment, substitution, None)?;
                match task {
                    Type::Named(named)
                        if named.name == "Task" && named.type_arguments.len() == 1 =>
                    {
                        Ok(named.type_arguments.into_iter().next().unwrap())
                    }
                    _ => Err(CodegenError::at("`wait` expects Task<T>", value.span)),
                }
            }
            Expression::Unwrap(value) => {
                let optional =
                    self.rewrite_expression(&mut value.argument, environment, substitution, None)?;
                match optional {
                    Type::Optional(optional) => Ok(optional.inner.as_ref().clone()),
                    _ => Err(CodegenError::at(
                        "postfix `!` expects an optional value",
                        value.span,
                    )),
                }
            }
            Expression::Cancel(value) => {
                self.rewrite_expression(&mut value.argument, environment, substitution, None)?;
                Ok(unknown_type(value.span))
            }
            Expression::Closure(closure) => {
                let expected_function = expected.and_then(|expected| match expected {
                    Type::Function(function) => Some(function.clone()),
                    _ => None,
                });
                if let Some(expected) = &expected_function
                    && expected.params.len() != closure.params.len()
                {
                    return Err(CodegenError::at(
                        format!(
                            "closure expects {} parameter(s), but the target type requires {}",
                            closure.params.len(),
                            expected.params.len()
                        ),
                        closure.span,
                    ));
                }

                let mut closure_environment = environment.clone();
                let mut parameters = Vec::with_capacity(closure.params.len());
                for (index, pattern) in closure.params.iter_mut().enumerate() {
                    let Pattern::Ident(identifier) = pattern else {
                        return Err(CodegenError::at(
                            "closure parameters require identifier patterns",
                            pattern.span(),
                        ));
                    };
                    let type_ = match (&identifier.type_annotation, &expected_function) {
                        (Some(annotation), _) => {
                            self.concrete_type(annotation, substitution, identifier.span)?
                        }
                        (None, Some(function)) => function
                            .params
                            .get(index)
                            .map(|parameter| parameter.type_.clone())
                            .ok_or_else(|| {
                                CodegenError::at(
                                    "closure parameter type could not be inferred",
                                    identifier.span,
                                )
                            })?,
                        (None, None) => {
                            return Err(CodegenError::at(
                                "closure parameters require a type annotation when no function type is expected",
                                identifier.span,
                            ));
                        }
                    };
                    identifier.type_annotation = Some(type_.clone());
                    closure_environment.insert(identifier.name.clone(), type_.clone());
                    parameters.push(Parameter {
                        span: identifier.span,
                        name: identifier.name.clone(),
                        type_,
                        default: None,
                    });
                }

                let expected_return = expected_function
                    .as_ref()
                    .map(|function| function.return_type.as_ref());
                let return_type = self.rewrite_expression(
                    &mut closure.body,
                    &mut closure_environment,
                    substitution,
                    expected_return,
                )?;
                let return_type = expected_return.cloned().unwrap_or(return_type);
                let function_type = FunctionType {
                    span: closure.span,
                    params: parameters.clone(),
                    return_type: Box::new(return_type.clone()),
                };

                let parameter_names = parameters
                    .iter()
                    .map(|parameter| parameter.name.as_str())
                    .collect::<HashSet<_>>();
                let mut captures = HashSet::new();
                collect_captures(&closure.body, environment, &mut captures);
                captures.retain(|capture| !parameter_names.contains(capture.as_str()));
                let mut captures = captures.into_iter().collect::<Vec<_>>();
                captures.sort();

                let closure_id = self.next_closure;
                self.next_closure += 1;
                let environment_name = format!(
                    "__kome_closure_environment_{}_{}",
                    closure.span.start, closure_id
                );
                let function_name =
                    format!("__kome_closure_body_{}_{}", closure.span.start, closure_id);
                let fields = captures
                    .iter()
                    .map(|capture| {
                        let type_ = environment.get(capture).cloned().ok_or_else(|| {
                            CodegenError::at(
                                format!("closure capture `{capture}` has no concrete type"),
                                closure.span,
                            )
                        })?;
                        Ok(StructField {
                            span: closure.span,
                            visibility: Visibility::Private,
                            name: capture.clone(),
                            type_,
                        })
                    })
                    .collect::<CodegenResult<Vec<_>>>()?;
                self.output.push(Declaration::Struct(StructDeclaration {
                    span: closure.span,
                    visibility: Visibility::Private,
                    attributes: Vec::new(),
                    name: environment_name.clone(),
                    type_parameters: Vec::new(),
                    fields: Some(fields),
                }));

                replace_closure_captures(
                    &mut closure.body,
                    &captures.iter().cloned().collect(),
                    "__kome_closure_environment",
                );
                let environment_type = Type::Named(NamedType {
                    span: closure.span,
                    name: environment_name.clone(),
                    type_arguments: Vec::new(),
                });
                let mut lifted_parameters = vec![Pattern::Ident(IdentifierPattern {
                    span: closure.span,
                    name: "__kome_closure_environment".into(),
                    type_annotation: Some(environment_type),
                    default: None,
                })];
                lifted_parameters.extend(closure.params.clone());
                let body_expression = closure.body.as_ref().clone();
                let statement = if matches!(
                    &return_type,
                    Type::Named(named)
                        if named.name == "Void" && named.type_arguments.is_empty()
                ) {
                    Statement::Expression(ExpressionStatement {
                        span: closure.span,
                        expression: body_expression,
                    })
                } else {
                    Statement::Return(ReturnStatement {
                        span: closure.span,
                        argument: Some(body_expression),
                    })
                };
                self.output.push(Declaration::Function(FunctionDeclaration {
                    span: closure.span,
                    visibility: Visibility::Private,
                    attributes: Vec::new(),
                    name: function_name.clone(),
                    type_parameters: Vec::new(),
                    params: lifted_parameters,
                    body: Some(BlockStatement {
                        span: closure.span,
                        statements: vec![statement],
                    }),
                    return_type: Some(return_type),
                }));
                closure.lowering = Some(ClosureLowering {
                    function: function_name,
                    environment: environment_name,
                    capture_values: captures
                        .iter()
                        .map(|name| {
                            Expression::Ident(IdentifierExpression {
                                span: closure.span,
                                name: name.clone(),
                            })
                        })
                        .collect(),
                    captures,
                    function_type: function_type.clone(),
                });
                Ok(Type::Function(function_type))
            }
            Expression::Struct(value) => {
                let arguments = value
                    .type_arguments
                    .iter()
                    .map(|argument| substitution.apply(argument))
                    .collect::<Vec<_>>();
                let name = self.instantiate_struct(&value.name, &arguments, value.span)?;
                value.name = name.clone();
                value.type_arguments.clear();
                let declaration = self
                    .output
                    .iter()
                    .find_map(|declaration| match declaration {
                        Declaration::Struct(value) if value.name == name => Some(value.clone()),
                        _ => None,
                    })
                    .unwrap();
                if let Some(fields) = declaration.fields {
                    for property in &mut value.fields {
                        let field_name = match &property.key {
                            PropertyKey::Ident { name, .. }
                            | PropertyKey::String { value: name, .. } => name,
                            _ => continue,
                        };
                        let field_type = fields
                            .iter()
                            .find(|field| &field.name == field_name)
                            .map(|field| &field.type_);
                        self.rewrite_expression(
                            &mut property.value,
                            environment,
                            substitution,
                            field_type,
                        )?;
                    }
                }
                Ok(Type::Named(NamedType {
                    span: value.span,
                    name,
                    type_arguments: Vec::new(),
                }))
            }
            Expression::Call(call) => {
                if let Expression::Ident(identifier) = call.callee.as_ref()
                    && matches!(identifier.name.as_str(), "all" | "race" | "timeout")
                {
                    let name = identifier.name.clone();
                    let mut task_result = None;
                    for argument in &mut call.args {
                        let type_ = self.rewrite_expression(
                            match argument {
                                CallArg::Positional(value) => value,
                                CallArg::Named { value, .. } => value,
                            },
                            environment,
                            substitution,
                            None,
                        )?;
                        if task_result.is_none()
                            && let Type::Named(named) = type_
                            && named.name == "Task"
                            && named.type_arguments.len() == 1
                        {
                            task_result = named.type_arguments.into_iter().next();
                        }
                    }
                    let result = task_result.unwrap_or_else(|| unknown_type(call.span));
                    return Ok(if name == "all" {
                        Type::List(kome_ast::types::ListType {
                            span: call.span,
                            element: Box::new(result),
                        })
                    } else {
                        result
                    });
                }
                if let Expression::Member(member) = call.callee.as_mut()
                    && let Expression::Ident(identifier) = member.object.as_mut()
                    && self.structs.contains_key(&identifier.name)
                    && !call.type_arguments.is_empty()
                {
                    let arguments = call
                        .type_arguments
                        .iter()
                        .map(|argument| substitution.apply(argument))
                        .collect::<Vec<_>>();
                    identifier.name =
                        self.instantiate_struct(&identifier.name, &arguments, call.span)?;
                    call.type_arguments.clear();
                }
                if let Expression::Ident(identifier) = call.callee.as_mut()
                    && let Some(template) = self.functions.get(&identifier.name).copied().cloned()
                {
                    let mut argument_types = Vec::new();
                    for (index, argument) in call.args.iter_mut().enumerate() {
                        let value = match argument {
                            CallArg::Positional(value) => value,
                            CallArg::Named { value, .. } => value,
                        };
                        let expected_argument = if template.type_parameters.is_empty() {
                            template.params.get(index).and_then(|parameter| {
                                if let Pattern::Ident(parameter) = parameter {
                                    parameter
                                        .type_annotation
                                        .as_ref()
                                        .map(|type_| substitution.apply(type_))
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        };
                        argument_types.push(self.rewrite_expression(
                            value,
                            environment,
                            substitution,
                            expected_argument.as_ref(),
                        )?);
                    }
                    let mut type_arguments = call
                        .type_arguments
                        .iter()
                        .map(|argument| substitution.apply(argument))
                        .collect::<Vec<_>>();
                    if type_arguments.is_empty() && !template.type_parameters.is_empty() {
                        let mut inferred = HashMap::new();
                        for (pattern, actual) in template.params.iter().zip(&argument_types) {
                            if let Pattern::Ident(parameter) = pattern
                                && let Some(formal) = &parameter.type_annotation
                            {
                                let actual = self.generic_view(actual);
                                infer(
                                    formal,
                                    &actual,
                                    &template.type_parameters,
                                    &mut inferred,
                                    call.span,
                                )?;
                            }
                        }
                        for parameter in &template.type_parameters {
                            type_arguments.push(inferred.remove(&parameter.name).ok_or_else(
                                || {
                                    CodegenError::at(
                                        format!(
                                            "generic inference failed for `{}`",
                                            parameter.name
                                        ),
                                        call.span,
                                    )
                                },
                            )?);
                        }
                    }
                    let specialized =
                        self.instantiate_function(&identifier.name, &type_arguments, call.span)?;
                    identifier.name = specialized;
                    call.type_arguments.clear();
                    let map = TypeSubstitution::new(
                        Self::names(&template.type_parameters),
                        &type_arguments,
                    );
                    return Ok(template
                        .return_type
                        .as_ref()
                        .map(|value| map.apply(value))
                        .unwrap_or_else(|| void_type(call.span)));
                }
                let member_result = if let Expression::Member(member) = call.callee.as_mut() {
                    let target = if let Expression::Ident(identifier) = member.object.as_ref()
                        && self.structs.contains_key(&identifier.name)
                    {
                        Type::Named(NamedType {
                            span: identifier.span,
                            name: identifier.name.clone(),
                            type_arguments: Vec::new(),
                        })
                    } else {
                        self.rewrite_expression(
                            &mut member.object,
                            environment,
                            substitution,
                            None,
                        )?
                    };
                    self.method_return_type(&target, &member.property)
                } else {
                    None
                };
                for argument in &mut call.args {
                    self.rewrite_expression(
                        match argument {
                            CallArg::Positional(value) => value,
                            CallArg::Named { value, .. } => value,
                        },
                        environment,
                        substitution,
                        None,
                    )?;
                }
                Ok(member_result.unwrap_or_else(|| unknown_type(call.span)))
            }
            Expression::Member(member) => {
                let object =
                    self.rewrite_expression(&mut member.object, environment, substitution, None)?;
                if let Type::Named(named) = object {
                    for declaration in &self.output {
                        if let Declaration::Struct(struct_) = declaration
                            && struct_.name == named.name
                            && let Some(fields) = &struct_.fields
                            && let Some(field) =
                                fields.iter().find(|field| field.name == member.property)
                        {
                            return Ok(field.type_.clone());
                        }
                    }
                }
                Ok(unknown_type(member.span))
            }
            Expression::Binary(value) => {
                let left =
                    self.rewrite_expression(&mut value.left, environment, substitution, expected)?;
                self.rewrite_expression(&mut value.right, environment, substitution, Some(&left))?;
                Ok(left)
            }
            Expression::Assign(value) => {
                self.rewrite_expression(&mut value.value, environment, substitution, expected)
            }
            Expression::Component(value) => {
                for argument in &mut value.args {
                    self.rewrite_expression(
                        match argument {
                            CallArg::Positional(value) => value,
                            CallArg::Named { value, .. } => value,
                        },
                        environment,
                        substitution,
                        None,
                    )?;
                }
                for child in &mut value.children {
                    self.rewrite_expression(child, environment, substitution, None)?;
                }
                Ok(primitive("Null", value.span))
            }
            _ => Ok(expected
                .cloned()
                .unwrap_or_else(|| unknown_type(expression.span()))),
        }
    }

    fn generic_view(&self, type_: &Type) -> Type {
        if let Type::Named(named) = type_
            && let Some((name, arguments)) = self.struct_origins.get(&named.name)
        {
            return Type::Named(NamedType {
                span: named.span,
                name: name.clone(),
                type_arguments: arguments.clone(),
            });
        }
        type_.clone()
    }

    fn method_return_type(&self, target: &Type, method: &str) -> Option<Type> {
        let actual = self.generic_view(target);
        for implementation in &self.implementations {
            let mut inferred = HashMap::new();
            if infer(
                &implementation.target,
                &actual,
                &implementation.type_parameters,
                &mut inferred,
                implementation.span,
            )
            .is_err()
            {
                continue;
            }
            let formal = self.generic_view(&implementation.target);
            if implementation.type_parameters.is_empty()
                && Self::encode_type(&formal) != Self::encode_type(&actual)
            {
                continue;
            }
            let Some(function) = implementation
                .members
                .iter()
                .find_map(|member| match member {
                    TypeMember::Function(function) if function.name == method => Some(function),
                    _ => None,
                })
            else {
                continue;
            };
            let Some(return_type) = function.return_type.as_ref() else {
                continue;
            };
            let arguments = implementation
                .type_parameters
                .iter()
                .map(|parameter| inferred.get(&parameter.name).cloned())
                .collect::<Option<Vec<_>>>()?;
            let substitution =
                TypeSubstitution::new(Self::names(&implementation.type_parameters), &arguments);
            return Some(substitution.apply(return_type));
        }
        None
    }
}

fn replace_closure_captures(
    expression: &mut Expression,
    captures: &HashSet<String>,
    environment: &str,
) {
    if let Expression::Ident(identifier) = expression
        && captures.contains(&identifier.name)
    {
        *expression = Expression::Member(MemberExpression {
            span: identifier.span,
            object: Box::new(Expression::Ident(IdentifierExpression {
                span: identifier.span,
                name: environment.to_owned(),
            })),
            property: identifier.name.clone(),
        });
        return;
    }
    match expression {
        Expression::Unary(value) => {
            replace_closure_captures(&mut value.argument, captures, environment)
        }
        Expression::Unwrap(value) => {
            replace_closure_captures(&mut value.argument, captures, environment)
        }
        Expression::Task(value) => {
            replace_closure_captures(&mut value.argument, captures, environment)
        }
        Expression::Wait(value) => {
            replace_closure_captures(&mut value.argument, captures, environment)
        }
        Expression::Cancel(value) => {
            replace_closure_captures(&mut value.argument, captures, environment)
        }
        Expression::Binary(value) => {
            replace_closure_captures(&mut value.left, captures, environment);
            replace_closure_captures(&mut value.right, captures, environment);
        }
        Expression::Call(value) => {
            replace_closure_captures(&mut value.callee, captures, environment);
            for argument in &mut value.args {
                let expression = match argument {
                    CallArg::Positional(expression) => expression,
                    CallArg::Named { value, .. } => value,
                };
                replace_closure_captures(expression, captures, environment);
            }
        }
        Expression::Member(value) => {
            replace_closure_captures(&mut value.object, captures, environment)
        }
        Expression::Index(value) => {
            replace_closure_captures(&mut value.object, captures, environment);
            replace_closure_captures(&mut value.index, captures, environment);
        }
        Expression::Assign(value) => {
            replace_closure_captures(&mut value.target, captures, environment);
            replace_closure_captures(&mut value.value, captures, environment);
        }
        Expression::Group(value) => {
            replace_closure_captures(&mut value.expression, captures, environment)
        }
        Expression::Block(value) => {
            for statement in &mut value.statements {
                replace_statement_captures(statement, captures, environment);
            }
            if let Some(tail) = &mut value.tail {
                replace_closure_captures(tail, captures, environment);
            }
        }
        Expression::List(value) => {
            for element in value.elems.iter_mut().flatten() {
                replace_closure_captures(element, captures, environment);
            }
        }
        Expression::Object(value) => {
            for property in &mut value.props {
                let kome_ast::expressions::ObjectProperty::KeyValue(property) = property;
                replace_closure_captures(&mut property.value, captures, environment);
            }
        }
        Expression::Struct(value) => {
            for field in &mut value.fields {
                replace_closure_captures(&mut field.value, captures, environment);
            }
        }
        Expression::Template(value) => {
            for part in &mut value.parts {
                if let kome_ast::expressions::TemplatePart::Expression { expression, .. } = part {
                    replace_closure_captures(expression, captures, environment);
                }
            }
        }
        Expression::Is(value) => {
            replace_closure_captures(&mut value.value, captures, environment);
            replace_closure_captures(&mut value.body, captures, environment);
        }
        Expression::Component(value) => {
            for argument in &mut value.args {
                let expression = match argument {
                    CallArg::Positional(expression) => expression,
                    CallArg::Named { value, .. } => value,
                };
                replace_closure_captures(expression, captures, environment);
            }
            for child in &mut value.children {
                replace_closure_captures(child, captures, environment);
            }
        }
        Expression::Closure(value) => {
            if let Some(lowering) = &mut value.lowering {
                for capture in &mut lowering.capture_values {
                    replace_closure_captures(capture, captures, environment);
                }
            }
        }
        Expression::Literal(_) | Expression::Ident(_) | Expression::DotIdent(_) => {}
    }
}

fn trailing_closure_call(component: kome_ast::expressions::ComponentExpression) -> Expression {
    let mut children = component.children;
    let tail = children.pop().map(Box::new);
    let statements = children
        .into_iter()
        .map(|expression| {
            let span = expression.span();
            Statement::Expression(ExpressionStatement { span, expression })
        })
        .collect();
    let closure = Expression::Closure(ClosureExpression {
        span: component.span,
        params: Vec::new(),
        body: Box::new(Expression::Block(BlockExpression {
            span: component.span,
            statements,
            tail,
        })),
        lowering: None,
    });
    let mut args = component.args;
    args.push(CallArg::Positional(closure));
    Expression::Call(CallExpression {
        span: component.span,
        callee: Box::new(Expression::Ident(IdentifierExpression {
            span: component.span,
            name: component.name,
        })),
        type_arguments: Vec::new(),
        args,
    })
}

fn replace_statement_captures(
    statement: &mut Statement,
    captures: &HashSet<String>,
    environment: &str,
) {
    match statement {
        Statement::Let(binding) => {
            if let Some(initializer) = &mut binding.init {
                replace_closure_captures(initializer, captures, environment);
            }
        }
        Statement::Expression(value) => {
            replace_closure_captures(&mut value.expression, captures, environment)
        }
        Statement::Return(value) => {
            if let Some(argument) = &mut value.argument {
                replace_closure_captures(argument, captures, environment);
            }
        }
        Statement::Block(value) => {
            for statement in &mut value.statements {
                replace_statement_captures(statement, captures, environment);
            }
        }
        Statement::If(value) => {
            replace_closure_captures(&mut value.test, captures, environment);
            replace_statement_captures(&mut value.consequent, captures, environment);
            if let Some(alternative) = &mut value.alternative {
                replace_statement_captures(alternative, captures, environment);
            }
        }
        Statement::While(value) => {
            replace_closure_captures(&mut value.test, captures, environment);
            replace_statement_captures(&mut value.body, captures, environment);
        }
        Statement::ForIn(value) => {
            replace_closure_captures(&mut value.right, captures, environment);
            replace_statement_captures(&mut value.body, captures, environment);
        }
        Statement::Is(value) => {
            if let Some(expression) = &mut value.value {
                replace_closure_captures(expression, captures, environment);
            }
            replace_statement_captures(&mut value.body, captures, environment);
        }
        Statement::Declaration(Declaration::Let(binding))
        | Statement::Declaration(Declaration::Constant(binding)) => {
            if let Some(initializer) = &mut binding.init {
                replace_closure_captures(initializer, captures, environment);
            }
        }
        Statement::Declaration(_)
        | Statement::Break(_)
        | Statement::Continue(_)
        | Statement::Empty(_) => {}
    }
}

fn collect_captures(
    expression: &Expression,
    environment: &HashMap<String, Type>,
    captures: &mut HashSet<String>,
) {
    match expression {
        Expression::Ident(identifier) => {
            if environment.contains_key(&identifier.name) {
                captures.insert(identifier.name.clone());
            }
        }
        Expression::Unary(value) => collect_captures(&value.argument, environment, captures),
        Expression::Unwrap(value) => collect_captures(&value.argument, environment, captures),
        Expression::Task(value) => collect_captures(&value.argument, environment, captures),
        Expression::Wait(value) => collect_captures(&value.argument, environment, captures),
        Expression::Cancel(value) => collect_captures(&value.argument, environment, captures),
        Expression::Binary(value) => {
            collect_captures(&value.left, environment, captures);
            collect_captures(&value.right, environment, captures);
        }
        Expression::Call(value) => {
            collect_captures(&value.callee, environment, captures);
            for argument in &value.args {
                match argument {
                    CallArg::Positional(value) => {
                        collect_captures(value, environment, captures);
                    }
                    CallArg::Named { value, .. } => {
                        collect_captures(value, environment, captures);
                    }
                }
            }
        }
        Expression::Member(value) => collect_captures(&value.object, environment, captures),
        Expression::Index(value) => {
            collect_captures(&value.object, environment, captures);
            collect_captures(&value.index, environment, captures);
        }
        Expression::Struct(value) => {
            for field in &value.fields {
                collect_captures(&field.value, environment, captures);
            }
        }
        Expression::Group(value) => collect_captures(&value.expression, environment, captures),
        Expression::Assign(value) => {
            collect_captures(&value.target, environment, captures);
            collect_captures(&value.value, environment, captures);
        }
        Expression::Block(value) => {
            for statement in &value.statements {
                collect_statement_captures(statement, environment, captures);
            }
            if let Some(tail) = &value.tail {
                collect_captures(tail, environment, captures);
            }
        }
        Expression::List(value) => {
            for element in value.elems.iter().flatten() {
                collect_captures(element, environment, captures);
            }
        }
        Expression::Object(value) => {
            for property in &value.props {
                let kome_ast::expressions::ObjectProperty::KeyValue(property) = property;
                collect_captures(&property.value, environment, captures);
            }
        }
        Expression::Template(value) => {
            for part in &value.parts {
                if let kome_ast::expressions::TemplatePart::Expression { expression, .. } = part {
                    collect_captures(expression, environment, captures);
                }
            }
        }
        Expression::Closure(value) => {
            if let Some(lowering) = &value.lowering {
                for capture in &lowering.capture_values {
                    collect_captures(capture, environment, captures);
                }
            } else {
                collect_captures(&value.body, environment, captures);
            }
        }
        Expression::Is(value) => {
            collect_captures(&value.value, environment, captures);
            collect_captures(&value.body, environment, captures);
        }
        Expression::Component(value) => {
            for argument in &value.args {
                match argument {
                    CallArg::Positional(value) => collect_captures(value, environment, captures),
                    CallArg::Named { value, .. } => {
                        collect_captures(value, environment, captures);
                    }
                }
            }
            for child in &value.children {
                collect_captures(child, environment, captures);
            }
        }
        Expression::Literal(_) | Expression::DotIdent(_) => {}
    }
}

fn collect_statement_captures(
    statement: &Statement,
    environment: &HashMap<String, Type>,
    captures: &mut HashSet<String>,
) {
    match statement {
        Statement::Expression(value) => collect_captures(&value.expression, environment, captures),
        Statement::Let(value) => {
            if let Some(init) = &value.init {
                collect_captures(init, environment, captures);
            }
        }
        Statement::Return(value) => {
            if let Some(argument) = &value.argument {
                collect_captures(argument, environment, captures);
            }
        }
        Statement::Block(value) => {
            for statement in &value.statements {
                collect_statement_captures(statement, environment, captures);
            }
        }
        Statement::If(value) => {
            collect_captures(&value.test, environment, captures);
            collect_statement_captures(&value.consequent, environment, captures);
            if let Some(alternative) = &value.alternative {
                collect_statement_captures(alternative, environment, captures);
            }
        }
        Statement::While(value) => {
            collect_captures(&value.test, environment, captures);
            collect_statement_captures(&value.body, environment, captures);
        }
        Statement::ForIn(value) => {
            collect_captures(&value.right, environment, captures);
            collect_statement_captures(&value.body, environment, captures);
        }
        Statement::Is(value) => {
            if let Some(value) = &value.value {
                collect_captures(value, environment, captures);
            }
            collect_statement_captures(&value.body, environment, captures);
        }
        Statement::Declaration(_)
        | Statement::Break(_)
        | Statement::Continue(_)
        | Statement::Empty(_) => {}
    }
}

fn unknown_type(span: kome_ast::Span) -> Type {
    Type::Named(NamedType {
        span,
        name: "<unknown>".into(),
        type_arguments: Vec::new(),
    })
}

fn void_type(span: kome_ast::Span) -> Type {
    Type::Named(NamedType {
        span,
        name: "Void".into(),
        type_arguments: Vec::new(),
    })
}
fn primitive(name: &str, span: kome_ast::Span) -> Type {
    use kome_ast::types::{PrimitiveType, PrimitiveTypeKind::*};
    let kind = match name {
        "String" => String,
        "Number" => Number,
        "bool" => Bool,
        "Null" => Null,
        _ => unreachable!(),
    };
    Type::Primitive(PrimitiveType { span, kind })
}
fn infer(
    formal: &Type,
    actual: &Type,
    parameters: &[GenericParameter],
    inferred: &mut HashMap<String, Type>,
    span: kome_ast::Span,
) -> CodegenResult<()> {
    if let Type::Named(named) = formal
        && named.type_arguments.is_empty()
        && parameters
            .iter()
            .any(|parameter| parameter.name == named.name)
    {
        if let Some(previous) = inferred.get(&named.name) {
            if previous != actual {
                return Err(CodegenError::at(
                    format!("conflicting inferred types for `{}`", named.name),
                    span,
                ));
            }
        } else {
            inferred.insert(named.name.clone(), actual.clone());
        }
        return Ok(());
    }
    if let (Type::Named(formal), Type::Named(actual)) = (formal, actual) {
        if formal.name == actual.name {
            for (left, right) in formal.type_arguments.iter().zip(&actual.type_arguments) {
                infer(left, right, parameters, inferred, span)?;
            }
        }
    }
    Ok(())
}
