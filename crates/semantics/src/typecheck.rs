use std::collections::HashMap;
use std::fmt;

use kome_ast::declarations::{
    Binding, ComponentDeclaration, ComponentMember, Declaration, ForDeclaration,
    FunctionDeclaration, Module, StructDeclaration, TraitDeclaration, TypeMember,
};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, BinaryExpression, BinaryOp, BlockExpression, CallArg,
    CallExpression, ClosureExpression, ComponentExpression, Expression, IdentifierExpression,
    LiteralKind, ObjectExpression, ObjectProperty, PropertyKey, StructExpression, UnaryOp,
};
use kome_ast::patterns::Pattern;
use kome_ast::statements::{
    BlockStatement, ExpressionStatement, ForInStatement, IfStatement, ReturnStatement, Statement,
    WhileStatement,
};
use kome_ast::types::{PrimitiveTypeKind, Type};
use kome_ast::{AstNode, Span};

fn collect_semantic_parameters(type_: &SemanticType, output: &mut Vec<String>) {
    match type_ {
        SemanticType::TypeParameter(name) => {
            if !output.contains(name) {
                output.push(name.clone());
            }
        }
        SemanticType::Applied(_, arguments) => {
            for argument in arguments {
                collect_semantic_parameters(argument, output);
            }
        }
        SemanticType::Optional(inner) => collect_semantic_parameters(inner, output),
        SemanticType::List(inner) => collect_semantic_parameters(inner, output),
        SemanticType::Function(parameters, return_type) => {
            for parameter in parameters {
                collect_semantic_parameters(parameter, output);
            }
            collect_semantic_parameters(return_type, output);
        }
        SemanticType::Pointer { pointee, .. } => collect_semantic_parameters(pointee, output),
        _ => {}
    }
}

fn substitute_semantic(
    type_: &SemanticType,
    substitutions: &HashMap<String, SemanticType>,
) -> SemanticType {
    match type_ {
        SemanticType::TypeParameter(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| type_.clone()),
        SemanticType::Applied(name, arguments) => SemanticType::Applied(
            name.clone(),
            arguments
                .iter()
                .map(|value| substitute_semantic(value, substitutions))
                .collect(),
        ),
        SemanticType::Optional(inner) => {
            SemanticType::Optional(Box::new(substitute_semantic(inner, substitutions)))
        }
        SemanticType::List(inner) => {
            SemanticType::List(Box::new(substitute_semantic(inner, substitutions)))
        }
        SemanticType::Function(parameters, return_type) => SemanticType::Function(
            parameters
                .iter()
                .map(|parameter| substitute_semantic(parameter, substitutions))
                .collect(),
            Box::new(substitute_semantic(return_type, substitutions)),
        ),
        SemanticType::Pointer {
            mutability,
            pointee,
        } => SemanticType::Pointer {
            mutability: *mutability,
            pointee: Box::new(substitute_semantic(pointee, substitutions)),
        },
        _ => type_.clone(),
    }
}

fn infer_type_parameters(
    formal: &SemanticType,
    actual: &SemanticType,
    substitutions: &mut HashMap<String, SemanticType>,
) -> Result<(), String> {
    match formal {
        SemanticType::TypeParameter(name) => {
            if let Some(previous) = substitutions.get(name) {
                if previous != actual {
                    return Err(name.clone());
                }
            } else {
                substitutions.insert(name.clone(), actual.clone());
            }
        }
        SemanticType::Applied(name, arguments) => {
            if let SemanticType::Applied(actual_name, actual_arguments) = actual
                && name == actual_name
                && arguments.len() == actual_arguments.len()
            {
                for (formal, actual) in arguments.iter().zip(actual_arguments) {
                    infer_type_parameters(formal, actual, substitutions)?;
                }
            } else {
                return Err(String::new());
            }
        }
        SemanticType::List(formal) => {
            if let SemanticType::List(actual) = actual {
                infer_type_parameters(formal, actual, substitutions)?;
            } else {
                return Err(String::new());
            }
        }
        SemanticType::Optional(formal) => {
            if let SemanticType::Optional(actual) = actual {
                infer_type_parameters(formal, actual, substitutions)?;
            } else {
                return Err(String::new());
            }
        }
        SemanticType::Function(formal_parameters, formal_return) => {
            if let SemanticType::Function(actual_parameters, actual_return) = actual
                && formal_parameters.len() == actual_parameters.len()
            {
                for (formal, actual) in formal_parameters.iter().zip(actual_parameters) {
                    infer_type_parameters(formal, actual, substitutions)?;
                }
                infer_type_parameters(formal_return, actual_return, substitutions)?;
            } else {
                return Err(String::new());
            }
        }
        SemanticType::Unknown => {}
        _ if formal != actual => return Err(String::new()),
        _ => {}
    }
    Ok(())
}

/// A type used during semantic type checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticType {
    Number,
    String,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Isize,
    Usize,
    F32,
    F64,
    Null,
    Named(String),
    Applied(String, Vec<SemanticType>),
    TypeParameter(String),
    Optional(Box<SemanticType>),
    List(Box<SemanticType>),
    Pointer {
        mutability: kome_ast::types::PointerMutability,
        pointee: Box<SemanticType>,
    },
    Function(Vec<SemanticType>, Box<SemanticType>),
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
            Self::I8 => "i8".to_owned(),
            Self::I16 => "i16".to_owned(),
            Self::I32 => "i32".to_owned(),
            Self::I64 => "i64".to_owned(),
            Self::U8 => "u8".to_owned(),
            Self::U16 => "u16".to_owned(),
            Self::U32 => "u32".to_owned(),
            Self::U64 => "u64".to_owned(),
            Self::Isize => "isize".to_owned(),
            Self::Usize => "usize".to_owned(),
            Self::F32 => "f32".to_owned(),
            Self::F64 => "f64".to_owned(),
            Self::Null => "Null".to_owned(),
            Self::Named(name) => name.clone(),
            Self::Applied(name, arguments) => format!(
                "{}<{}>",
                name,
                arguments
                    .iter()
                    .map(Self::name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::TypeParameter(name) => name.clone(),
            Self::Optional(inner) => format!("{}?", inner.name()),
            Self::List(inner) => format!("{}[]", inner.name()),
            Self::Pointer {
                mutability,
                pointee,
            } => {
                let qualifier = match mutability {
                    kome_ast::types::PointerMutability::Const => "const",
                    kome_ast::types::PointerMutability::Mut => "mut",
                };
                format!("*{qualifier} {}", pointee.name())
            }
            Self::Function(parameters, return_type) => format!(
                "({}) -> {}",
                parameters
                    .iter()
                    .map(Self::name)
                    .collect::<Vec<_>>()
                    .join(", "),
                return_type.name()
            ),
            Self::Void => "Void".to_owned(),
            Self::Unknown => "<unknown>".to_owned(),
        }
    }

    fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
        )
    }

    fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    fn is_numeric(&self) -> bool {
        matches!(self, Self::Number) || self.is_integer() || self.is_float()
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
    pub structs: HashMap<String, StructTypeInfo>,
    pub traits: HashMap<String, TraitTypeInfo>,
    pub implementations: Vec<TypeImplementationInfo>,
}

/// Semantic information retained for a declared struct.
#[derive(Debug, Clone)]
pub struct StructTypeInfo {
    pub fields: Option<HashMap<String, SemanticType>>,
    pub field_visibility: HashMap<String, kome_ast::declarations::Visibility>,
    pub type_parameters: Vec<String>,
    pub runtime: Option<String>,
}

/// Semantic information retained for a declared trait.
#[derive(Debug, Clone)]
pub struct TraitTypeInfo {
    pub functions: Vec<String>,
    pub type_parameters: Vec<String>,
    signatures: HashMap<String, FunctionSignature>,
}

/// Semantic target information for an inherent or trait implementation.
#[derive(Debug, Clone)]
pub struct TypeImplementationInfo {
    pub target: SemanticType,
    pub trait_: Option<SemanticType>,
    methods: HashMap<String, FunctionSignature>,
    method_visibility: HashMap<String, kome_ast::declarations::Visibility>,
    constants: HashMap<String, SemanticType>,
    constant_visibility: HashMap<String, kome_ast::declarations::Visibility>,
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
    type_parameters: Vec<String>,
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
    structs: HashMap<String, StructTypeInfo>,
    traits: HashMap<String, TraitTypeInfo>,
    implementations: Vec<TypeImplementationInfo>,
    current_module: String,
    current_type_parameters: Vec<String>,
    return_type: SemanticType,
    loop_depth: usize,
    in_drop_body: bool,
    allow_drop_self_access: bool,
    errors: Vec<TypeCheckError>,
}

impl TypeChecker {
    /// Runs semantic type checking on a parsed [`Module`].
    pub fn check(module: &Module) -> TypeCheckResult {
        let mut checker = Self {
            scopes: vec![HashMap::new()],
            functions: HashMap::new(),
            components: HashMap::new(),
            structs: HashMap::new(),
            traits: HashMap::new(),
            implementations: Vec::new(),
            current_module: "__app".into(),
            current_type_parameters: Vec::new(),
            return_type: SemanticType::Void,
            loop_depth: 0,
            in_drop_body: false,
            allow_drop_self_access: false,
            errors: Vec::new(),
        };

        checker.collect_declarations(module);
        checker.validate_implementations(module);
        checker.visit_module(module);

        TypeCheckResult {
            errors: checker.errors,
            structs: checker.structs,
            traits: checker.traits,
            implementations: checker.implementations,
        }
    }

    // -- declaration collection --

    fn collect_declarations(&mut self, module: &Module) {
        for declaration in &module.declarations {
            match declaration {
                Declaration::Struct(struct_decl) => {
                    self.check_generic_parameters(&struct_decl.type_parameters);
                    self.collect_struct(struct_decl);
                }

                Declaration::Trait(trait_decl) => {
                    self.check_generic_parameters(&trait_decl.type_parameters);
                    self.collect_trait(trait_decl);
                }

                Declaration::Function(function) => {
                    self.check_generic_parameters(&function.type_parameters);
                    self.collect_function(function);
                }

                Declaration::Component(component) => {
                    self.collect_component(component);
                }

                Declaration::For(declaration) => {
                    self.collect_implementation(declaration);
                }

                Declaration::Extern(extern_decl) => {
                    for item in &extern_decl.items {
                        match item {
                            kome_ast::declarations::ExternItem::Struct(declaration) => {
                                self.collect_struct(declaration);
                            }
                            kome_ast::declarations::ExternItem::Function(function) => {
                                self.collect_function(function);
                            }
                        }
                    }
                }

                _ => {}
            }
        }
    }

    fn check_generic_parameters(
        &mut self,
        parameters: &[kome_ast::declarations::GenericParameter],
    ) {
        let mut names = std::collections::HashSet::new();
        for parameter in parameters {
            if !names.insert(parameter.name.as_str()) {
                self.errors.push(TypeCheckError {
                    message: format!("duplicate generic parameter `{}`", parameter.name),
                    span: parameter.span,
                });
            }
        }
    }

    fn collect_implementation(&mut self, declaration: &ForDeclaration) {
        let generic_parameters = declaration
            .type_parameters
            .iter()
            .map(|value| value.name.clone())
            .collect::<Vec<_>>();
        let target = Self::type_from_annotation_with(&declaration.target, &generic_parameters);
        let mut methods = HashMap::new();
        let mut method_visibility = HashMap::new();
        let mut constants = HashMap::new();
        let mut constant_visibility = HashMap::new();
        for member in &declaration.members {
            match member {
                TypeMember::Function(function) => {
                    method_visibility.insert(
                        function.name.clone(),
                        if declaration.trait_.is_some() {
                            kome_ast::declarations::Visibility::Public
                        } else {
                            function.visibility
                        },
                    );
                    methods.insert(
                        function.name.clone(),
                        Self::signature_with(function, Some(&target), &generic_parameters),
                    );
                }
                TypeMember::Constant(binding) => {
                    if let Pattern::Ident(identifier) = &binding.pattern {
                        constant_visibility.insert(identifier.name.clone(), binding.visibility);
                        constants.insert(
                            identifier.name.clone(),
                            binding
                                .type_annotation
                                .as_ref()
                                .map(|value| {
                                    Self::type_from_annotation_with(value, &generic_parameters)
                                })
                                .unwrap_or(SemanticType::Unknown),
                        );
                    }
                }
            }
        }
        self.implementations.push(TypeImplementationInfo {
            target,
            trait_: declaration
                .trait_
                .as_ref()
                .map(|value| Self::type_from_annotation_with(value, &generic_parameters)),
            methods,
            method_visibility,
            constants,
            constant_visibility,
        });
    }

    fn collect_trait(&mut self, declaration: &TraitDeclaration) {
        let generic_parameters = declaration
            .type_parameters
            .iter()
            .map(|value| value.name.clone())
            .collect::<Vec<_>>();
        let functions = declaration
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect();
        let signatures = declaration
            .functions
            .iter()
            .map(|function| {
                (
                    function.name.clone(),
                    Self::signature_with(function, None, &generic_parameters),
                )
            })
            .collect();

        if self
            .traits
            .insert(
                declaration.name.clone(),
                TraitTypeInfo {
                    functions,
                    type_parameters: generic_parameters,
                    signatures,
                },
            )
            .is_some()
        {
            self.errors.push(TypeCheckError {
                message: format!("duplicate trait `{}`", declaration.name),
                span: declaration.span,
            });
        }
    }

    fn collect_struct(&mut self, struct_decl: &StructDeclaration) {
        let type_parameters = struct_decl
            .type_parameters
            .iter()
            .map(|value| value.name.clone())
            .collect::<Vec<_>>();
        let fields = struct_decl.fields.as_ref().map(|fields| {
            fields
                .iter()
                .map(|field| {
                    (
                        field.name.clone(),
                        Self::type_from_annotation_with(&field.type_, &type_parameters),
                    )
                })
                .collect()
        });
        let field_visibility = struct_decl
            .fields
            .as_ref()
            .into_iter()
            .flatten()
            .map(|field| (field.name.clone(), field.visibility))
            .collect();
        let runtime = struct_decl.attributes.iter().find_map(|attribute| {
            if attribute.name != "runtime" || attribute.args.len() != 1 {
                return None;
            }

            match &attribute.args[0] {
                Expression::Literal(literal) => match &literal.kind {
                    LiteralKind::String(runtime) => Some(runtime.clone()),
                    _ => None,
                },
                _ => None,
            }
        });

        if self
            .structs
            .insert(
                struct_decl.name.clone(),
                StructTypeInfo {
                    fields,
                    field_visibility,
                    type_parameters,
                    runtime,
                },
            )
            .is_some()
        {
            self.errors.push(TypeCheckError {
                message: format!("duplicate struct `{}`", struct_decl.name),
                span: struct_decl.span,
            });
        }
    }

    fn collect_function(&mut self, function: &FunctionDeclaration) {
        self.functions
            .insert(function.name.clone(), Self::signature(function, None));
    }

    fn signature(
        function: &FunctionDeclaration,
        self_type: Option<&SemanticType>,
    ) -> FunctionSignature {
        let generic_parameters = function
            .type_parameters
            .iter()
            .map(|value| value.name.clone())
            .collect::<Vec<_>>();
        Self::signature_with(function, self_type, &generic_parameters)
    }

    fn signature_with(
        function: &FunctionDeclaration,
        self_type: Option<&SemanticType>,
        generic_parameters: &[String],
    ) -> FunctionSignature {
        let mut params = Vec::new();

        for pattern in &function.params {
            let Pattern::Ident(identifier) = pattern else {
                continue;
            };

            let type_ = if identifier.name == "self" && identifier.type_annotation.is_none() {
                self_type.cloned().unwrap_or(SemanticType::Unknown)
            } else {
                identifier
                    .type_annotation
                    .as_ref()
                    .map(|value| Self::type_from_annotation_with(value, generic_parameters))
                    .unwrap_or(SemanticType::Unknown)
            };

            params.push(ParameterType {
                name: identifier.name.clone(),
                type_,
            });
        }

        let return_type = function
            .return_type
            .as_ref()
            .map(|value| Self::type_from_annotation_with(value, generic_parameters))
            .unwrap_or(SemanticType::Void);

        FunctionSignature {
            params,
            return_type,
            type_parameters: generic_parameters.to_vec(),
        }
    }

    fn validate_implementations(&mut self, module: &Module) {
        let mut drop_targets = Vec::new();
        for (index, declaration) in module
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::For(value) => Some(value),
                _ => None,
            })
            .enumerate()
        {
            let (trait_name, trait_arguments) = match self.implementations[index].trait_.as_ref() {
                Some(SemanticType::Named(name)) => (name, Vec::new()),
                Some(SemanticType::Applied(name, arguments)) => (name, arguments.clone()),
                _ => continue,
            };
            let Some(trait_info) = self.traits.get(trait_name).cloned() else {
                self.errors.push(TypeCheckError {
                    message: format!("trait `{trait_name}` was not found"),
                    span: declaration.span,
                });
                continue;
            };
            let target = self.implementations[index].target.clone();
            if trait_name == "Drop" {
                let target_name = match &target {
                    SemanticType::Named(name) | SemanticType::Applied(name, _) => Some(name),
                    _ => None,
                };
                let is_user_struct = target_name
                    .and_then(|name| self.structs.get(name))
                    .is_some_and(|info| info.runtime.is_none() && info.fields.is_some());
                if !is_user_struct {
                    self.errors.push(TypeCheckError {
                        message: "`Drop` can only be implemented by a user-defined struct"
                            .to_owned(),
                        span: declaration.span,
                    });
                }
                if drop_targets.iter().any(|existing| existing == &target) {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "type `{}` has more than one `Drop` implementation",
                            target.name()
                        ),
                        span: declaration.span,
                    });
                } else {
                    drop_targets.push(target.clone());
                }
            }
            if trait_info.type_parameters.len() != trait_arguments.len() {
                self.errors.push(TypeCheckError {
                    message: format!(
                        "trait `{trait_name}` expects {} type argument(s), but received {}",
                        trait_info.type_parameters.len(),
                        trait_arguments.len()
                    ),
                    span: declaration.span,
                });
                continue;
            }
            let trait_substitutions = trait_info
                .type_parameters
                .iter()
                .cloned()
                .zip(trait_arguments)
                .collect::<HashMap<_, _>>();
            for name in &trait_info.functions {
                let Some(actual) = self.implementations[index].methods.get(name) else {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "implementation of `{trait_name}` is missing required method `{name}`"
                        ),
                        span: declaration.span,
                    });
                    continue;
                };
                let mut expected = trait_info.signatures[name].clone();
                for parameter in &mut expected.params {
                    parameter.type_ = substitute_semantic(&parameter.type_, &trait_substitutions);
                }
                expected.return_type =
                    substitute_semantic(&expected.return_type, &trait_substitutions);
                for parameter in &mut expected.params {
                    if parameter.name == "self" && matches!(parameter.type_, SemanticType::Unknown)
                    {
                        parameter.type_ = target.clone();
                    }
                }
                if actual.params.len() != expected.params.len()
                    || actual
                        .params
                        .iter()
                        .zip(&expected.params)
                        .any(|(a, e)| a.type_ != e.type_)
                    || actual.return_type != expected.return_type
                {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "method `{name}` has an incompatible signature for trait `{trait_name}`"
                        ),
                        span: declaration.span,
                    });
                }
            }
        }
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
                    self.current_module = module_name_for_declaration(&function.name);
                    self.visit_function(function);
                }

                Declaration::Component(component) => {
                    self.current_module = module_name_for_declaration(&component.name);
                    self.visit_component(component);
                }

                Declaration::Constant(binding) => {
                    if let Pattern::Ident(identifier) = &binding.pattern {
                        self.current_module = module_name_for_declaration(&identifier.name);
                    }
                    self.register_binding(binding);
                }

                Declaration::For(declaration) => {
                    self.current_module = module_name_for_type(&declaration.target);
                    self.visit_implementation(declaration);
                }

                Declaration::Trait(declaration) => {
                    for function in &declaration.functions {
                        self.visit_function(function);
                    }
                }

                Declaration::Struct(declaration) => {
                    if let Some(fields) = &declaration.fields {
                        for field in fields {
                            self.validate_type_arity(&field.type_);
                        }
                    }
                }

                Declaration::Extern(extern_decl) => {
                    for item in &extern_decl.items {
                        match item {
                            kome_ast::declarations::ExternItem::Struct(declaration) => {
                                if let Some(fields) = &declaration.fields {
                                    for field in fields {
                                        self.validate_type_arity(&field.type_);
                                    }
                                }
                            }
                            kome_ast::declarations::ExternItem::Function(function) => {
                                self.current_module = module_name_for_declaration(&function.name);
                                self.visit_function(function);
                            }
                        }
                    }
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
        self.visit_function_with_self(function, None);
    }

    fn visit_function_with_self(
        &mut self,
        function: &FunctionDeclaration,
        self_type: Option<&SemanticType>,
    ) {
        self.enter_scope();
        let mut generic_parameters = function
            .type_parameters
            .iter()
            .map(|value| value.name.clone())
            .collect::<Vec<_>>();
        if let Some(self_type) = self_type {
            collect_semantic_parameters(self_type, &mut generic_parameters);
        }
        let previous_type_parameters = std::mem::replace(
            &mut self.current_type_parameters,
            generic_parameters.clone(),
        );

        for pattern in &function.params {
            let Pattern::Ident(identifier) = pattern else {
                continue;
            };
            if let Some(annotation) = &identifier.type_annotation {
                self.validate_type_arity(annotation);
            }

            let type_ = if identifier.name == "self" && identifier.type_annotation.is_none() {
                self_type.cloned().unwrap_or(SemanticType::Unknown)
            } else {
                identifier
                    .type_annotation
                    .as_ref()
                    .map(|value| Self::type_from_annotation_with(value, &generic_parameters))
                    .unwrap_or(SemanticType::Unknown)
            };

            self.declare(&identifier.name, type_);
        }

        let previous_return_type = self.return_type.clone();

        self.return_type = function
            .return_type
            .as_ref()
            .map(|value| Self::type_from_annotation_with(value, &generic_parameters))
            .unwrap_or(SemanticType::Void);
        if let Some(return_type) = &function.return_type {
            self.validate_type_arity(return_type);
        }

        if let Some(body) = &function.body {
            self.visit_block_statement(body);
        }

        self.return_type = previous_return_type;
        self.current_type_parameters = previous_type_parameters;

        self.exit_scope();
    }

    fn visit_implementation(&mut self, declaration: &ForDeclaration) {
        self.validate_type_arity(&declaration.target);
        if let Some(trait_) = &declaration.trait_ {
            self.validate_type_arity(trait_);
        }
        let target = Self::type_from_annotation(&declaration.target);
        self.enter_scope();
        for member in &declaration.members {
            if let TypeMember::Constant(binding) = member {
                self.register_binding(binding);
            }
        }
        for member in &declaration.members {
            if let TypeMember::Function(function) = member {
                let previous_drop_body = self.in_drop_body;
                self.in_drop_body = matches!(
                    declaration.trait_.as_ref(),
                    Some(Type::Named(named)) if named.name == "Drop"
                ) && function.name == "drop";
                self.visit_function_with_self(function, Some(&target));
                self.in_drop_body = previous_drop_body;
            }
        }
        self.exit_scope();
    }

    // -- binding visitors --

    fn register_binding(&mut self, binding: &Binding) {
        let Pattern::Ident(identifier) = &binding.pattern else {
            return;
        };
        if let Some(annotation) = &binding.type_annotation {
            self.validate_type_arity(annotation);
        }

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

            Statement::Break(statement) => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeCheckError {
                        message: "`break` can only be used inside a loop".into(),
                        span: statement.span,
                    });
                }
            }

            Statement::Continue(statement) => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeCheckError {
                        message: "`continue` can only be used inside a loop".into(),
                        span: statement.span,
                    });
                }
            }

            Statement::Empty(_) | Statement::Declaration(_) => {}
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

        self.loop_depth += 1;
        self.visit_statement(&while_statement.body);
        self.loop_depth -= 1;
    }

    fn visit_for_in_statement(&mut self, for_in: &ForInStatement) {
        let element_type = match self.infer_expression(&for_in.right, None) {
            SemanticType::List(element) => *element,
            SemanticType::Unknown => SemanticType::Unknown,
            actual => {
                self.errors.push(TypeCheckError {
                    message: format!("`for in` expects a List, but found {}", actual.name()),
                    span: for_in.right.span(),
                });
                SemanticType::Unknown
            }
        };

        self.enter_scope();

        if let Pattern::Ident(identifier) = &for_in.pattern {
            if let Some(annotation) = &identifier.type_annotation {
                let annotated = Self::type_from_annotation(annotation);
                self.check_compatible(&annotated, &element_type, identifier.span);
                self.declare(&identifier.name, annotated);
            } else {
                self.declare(&identifier.name, element_type);
            }
        }

        self.loop_depth += 1;
        self.visit_statement(&for_in.body);
        self.loop_depth -= 1;

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
                LiteralKind::Number(number) => {
                    let has_fraction = number.0.contains('.');
                    match expected {
                        Some(type_) if type_.is_integer() && !has_fraction => type_.clone(),
                        Some(type_) if type_.is_float() => type_.clone(),

                        _ => SemanticType::Number,
                    }
                }
                LiteralKind::Percent(_) => SemanticType::Number,
                LiteralKind::Boolean(_) => SemanticType::Bool,
                LiteralKind::Null => SemanticType::Null,
            },

            Expression::Ident(identifier) => {
                if self.in_drop_body && identifier.name == "self" && !self.allow_drop_self_access {
                    self.errors.push(TypeCheckError {
                        message: "destructor `self` may only be used to access fields".to_owned(),
                        span: identifier.span,
                    });
                }
                self.resolve(&identifier.name)
                    .unwrap_or(SemanticType::Unknown)
            }

            Expression::Unary(unary) => match unary.op {
                UnaryOp::Not => {
                    let argument =
                        self.infer_expression(&unary.argument, Some(&SemanticType::Bool));

                    self.check_compatible(&SemanticType::Bool, &argument, unary.argument.span());

                    SemanticType::Bool
                }
            },

            Expression::Unwrap(unwrap) => match self.infer_expression(&unwrap.argument, None) {
                SemanticType::Optional(inner) => *inner,
                SemanticType::Unknown => SemanticType::Unknown,
                actual => {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "postfix `!` expects an optional value, but found {}",
                            actual.name()
                        ),
                        span: unwrap.argument.span(),
                    });
                    SemanticType::Unknown
                }
            },

            Expression::Task(task) => {
                let result_expected = match expected {
                    Some(SemanticType::Applied(name, arguments))
                        if name == "Task" && arguments.len() == 1 =>
                    {
                        arguments.first()
                    }
                    _ => None,
                };
                SemanticType::Applied(
                    "Task".into(),
                    vec![self.infer_expression(&task.argument, result_expected)],
                )
            }

            Expression::Wait(wait) => {
                let task_expected = expected
                    .cloned()
                    .map(|result| SemanticType::Applied("Task".into(), vec![result]));
                match self.infer_expression(&wait.argument, task_expected.as_ref()) {
                    SemanticType::Applied(name, mut arguments)
                        if name == "Task" && arguments.len() == 1 =>
                    {
                        arguments.remove(0)
                    }
                    SemanticType::Unknown => SemanticType::Unknown,
                    actual => {
                        self.errors.push(TypeCheckError {
                            message: format!("`wait` expects Task<T>, but found {}", actual.name()),
                            span: wait.argument.span(),
                        });
                        SemanticType::Unknown
                    }
                }
            }

            Expression::Cancel(cancel) => {
                let actual = self.infer_expression(&cancel.argument, None);
                if !matches!(actual, SemanticType::Applied(ref name, ref arguments) if name == "Task" && arguments.len() == 1)
                    && actual != SemanticType::Unknown
                {
                    self.errors.push(TypeCheckError {
                        message: format!("`cancel` expects Task<T>, but found {}", actual.name()),
                        span: cancel.argument.span(),
                    });
                }
                SemanticType::Void
            }

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
                if let Expression::Ident(identifier) = member.object.as_ref()
                    && self.resolve(&identifier.name).is_none()
                    && self.structs.contains_key(&identifier.name)
                {
                    let target = SemanticType::Named(identifier.name.clone());
                    let found = self
                        .implementations
                        .iter()
                        .find(|implementation| implementation.target == target)
                        .and_then(|implementation| {
                            Some((
                                implementation.constants.get(&member.property)?.clone(),
                                implementation
                                    .constant_visibility
                                    .get(&member.property)
                                    .copied()
                                    .unwrap_or(kome_ast::declarations::Visibility::Private),
                            ))
                        });
                    if let Some((type_, visibility)) = found {
                        self.check_visibility(
                            &identifier.name,
                            &member.property,
                            visibility,
                            member.span,
                        );
                        return type_;
                    }
                    return SemanticType::Unknown;
                }
                let previous_access = self.allow_drop_self_access;
                if self.in_drop_body
                    && matches!(member.object.as_ref(), Expression::Ident(identifier) if identifier.name == "self")
                {
                    self.allow_drop_self_access = true;
                }
                let object = self.infer_expression(&member.object, None);
                self.allow_drop_self_access = previous_access;

                match object {
                    SemanticType::Named(name) => {
                        let found = self.structs.get(&name).and_then(|struct_| {
                            Some((
                                struct_.fields.as_ref()?.get(&member.property)?.clone(),
                                struct_
                                    .field_visibility
                                    .get(&member.property)
                                    .copied()
                                    .unwrap_or(kome_ast::declarations::Visibility::Private),
                            ))
                        });
                        if let Some((type_, visibility)) = found {
                            self.check_visibility(&name, &member.property, visibility, member.span);
                            type_
                        } else {
                            SemanticType::Unknown
                        }
                    }
                    SemanticType::Applied(name, arguments) => self
                        .structs
                        .get(&name)
                        .and_then(|info| {
                            let fields = info.fields.as_ref()?;
                            let field = fields.get(&member.property)?;
                            let substitutions = info
                                .type_parameters
                                .iter()
                                .cloned()
                                .zip(arguments)
                                .collect::<HashMap<_, _>>();
                            Some(substitute_semantic(field, &substitutions))
                        })
                        .map(|type_| {
                            let visibility = self.structs[&name]
                                .field_visibility
                                .get(&member.property)
                                .copied()
                                .unwrap_or(kome_ast::declarations::Visibility::Private);
                            self.check_visibility(&name, &member.property, visibility, member.span);
                            type_
                        })
                        .unwrap_or(SemanticType::Unknown),
                    _ => SemanticType::Unknown,
                }
            }

            Expression::Index(index) => {
                let object = self.infer_expression(&index.object, None);
                self.infer_expression(&index.index, None);
                match object {
                    SemanticType::List(element) => *element,
                    _ => SemanticType::Unknown,
                }
            }

            Expression::List(list) => {
                let mut inferred = None;
                for element in &list.elems {
                    if let Some(element) = element {
                        let actual = self.infer_expression(element, inferred.as_ref());
                        if let Some(expected) = &inferred {
                            self.check_compatible(expected, &actual, element.span());
                        } else {
                            inferred = Some(actual);
                        }
                    }
                }
                SemanticType::List(Box::new(inferred.unwrap_or(SemanticType::Unknown)))
            }

            Expression::Object(object) => self.infer_object_expression(object, expected),

            Expression::Struct(struct_) => self.infer_struct_expression(struct_),

            Expression::Closure(closure) => {
                let expected_function = match expected {
                    Some(SemanticType::Function(parameters, return_type)) => {
                        Some((parameters.as_slice(), return_type.as_ref()))
                    }
                    _ => None,
                };
                if let Some((parameters, _)) = expected_function
                    && parameters.len() != closure.params.len()
                {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "closure expects {} parameter(s), but the target type requires {}",
                            closure.params.len(),
                            parameters.len()
                        ),
                        span: closure.span,
                    });
                }
                self.enter_scope();
                let mut parameter_types = Vec::new();
                for (index, parameter) in closure.params.iter().enumerate() {
                    if let Pattern::Ident(identifier) = parameter {
                        let annotated = identifier
                            .type_annotation
                            .as_ref()
                            .map(Self::type_from_annotation);
                        let contextual = expected_function
                            .and_then(|(parameters, _)| parameters.get(index))
                            .cloned();
                        if let (Some(annotated), Some(contextual)) = (&annotated, &contextual) {
                            self.check_compatible(contextual, annotated, identifier.span);
                        }
                        let type_ = annotated.or(contextual).unwrap_or(SemanticType::Unknown);

                        self.declare(&identifier.name, type_.clone());
                        parameter_types.push(type_);
                    }
                }
                let expected_return = expected_function.map(|(_, return_type)| return_type);
                let return_type = self.infer_expression(&closure.body, expected_return);
                if let Some(expected_return) = expected_return {
                    self.check_compatible(expected_return, &return_type, closure.body.span());
                }

                self.exit_scope();
                SemanticType::Function(parameter_types, Box::new(return_type))
            }

            Expression::Is(is_expression) => {
                self.infer_expression(&is_expression.value, None);
                self.infer_expression(&is_expression.body, expected)
            }
        }
    }

    fn infer_object_expression(
        &mut self,
        object: &ObjectExpression,
        expected: Option<&SemanticType>,
    ) -> SemanticType {
        let fields = match expected {
            Some(SemanticType::Named(name)) => self
                .structs
                .get(name)
                .and_then(|struct_| struct_.fields.clone()),
            _ => None,
        };

        for property in &object.props {
            let ObjectProperty::KeyValue(property) = property;
            let name = match &property.key {
                PropertyKey::Ident { name, .. } | PropertyKey::String { value: name, .. } => {
                    Some(name.as_str())
                }
                _ => None,
            };
            let field_type = name.and_then(|name| fields.as_ref()?.get(name));
            let actual = self.infer_expression(&property.value, field_type);

            if let Some(field_type) = field_type {
                self.check_compatible(field_type, &actual, property.value.span());
            }
        }

        if fields.is_some() {
            expected.cloned().unwrap_or(SemanticType::Unknown)
        } else {
            SemanticType::Unknown
        }
    }

    fn infer_struct_expression(&mut self, struct_: &StructExpression) -> SemanticType {
        let info = self.structs.get(&struct_.name).cloned();
        let arguments = struct_
            .type_arguments
            .iter()
            .map(|type_| Self::type_from_annotation_with(type_, &self.current_type_parameters))
            .collect::<Vec<_>>();
        if let Some(info) = &info
            && info.type_parameters.len() != arguments.len()
        {
            self.errors.push(TypeCheckError {
                message: format!(
                    "struct `{}` expects {} type argument(s), but received {}",
                    struct_.name,
                    info.type_parameters.len(),
                    arguments.len()
                ),
                span: struct_.span,
            });
        }
        let substitutions = info
            .as_ref()
            .map(|info| {
                info.type_parameters
                    .iter()
                    .cloned()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let fields = info.and_then(|info| info.fields).map(|fields| {
            fields
                .into_iter()
                .map(|(name, type_)| (name, substitute_semantic(&type_, &substitutions)))
                .collect::<HashMap<_, _>>()
        });

        for field in &struct_.fields {
            let name = match &field.key {
                PropertyKey::Ident { name, .. } | PropertyKey::String { value: name, .. } => {
                    Some(name.as_str())
                }
                _ => None,
            };
            let expected = name.and_then(|name| fields.as_ref()?.get(name));
            let actual = self.infer_expression(&field.value, expected);

            if let Some(expected) = expected {
                self.check_compatible(expected, &actual, field.value.span());
            }
            if let Some(name) = name
                && let Some(info) = self.structs.get(&struct_.name)
            {
                let visibility = info
                    .field_visibility
                    .get(name)
                    .copied()
                    .unwrap_or(kome_ast::declarations::Visibility::Private);
                self.check_visibility(&struct_.name, name, visibility, field.span);
            }
        }

        if arguments.is_empty() {
            SemanticType::Named(struct_.name.clone())
        } else {
            SemanticType::Applied(struct_.name.clone(), arguments)
        }
    }

    fn infer_binary_expression(&mut self, binary: &BinaryExpression) -> SemanticType {
        match binary.op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let left = self.infer_expression(&binary.left, None);

                if binary.op == BinaryOp::Add && left == SemanticType::String {
                    let right = self.infer_expression(&binary.right, Some(&SemanticType::String));
                    self.check_compatible(&SemanticType::String, &right, binary.right.span());
                    return SemanticType::String;
                }

                if !left.is_numeric() && !matches!(left, SemanticType::Unknown) {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "operator requires a numeric value, but found {}",
                            left.name(),
                        ),
                        span: binary.left.span(),
                    });

                    self.infer_expression(&binary.right, None);

                    return SemanticType::Unknown;
                }

                let right = self.infer_expression(&binary.right, Some(&left));

                self.check_compatible(&left, &right, binary.right.span());

                left
            }

            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                let left = self.infer_expression(&binary.left, None);

                if !left.is_numeric() && !matches!(left, SemanticType::Unknown) {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "comparison requires a numeric value, but found {}",
                            left.name(),
                        ),
                        span: binary.left.span(),
                    });

                    self.infer_expression(&binary.right, None);

                    return SemanticType::Bool;
                }

                let right = self.infer_expression(&binary.right, Some(&left));

                self.check_compatible(&left, &right, binary.right.span());

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
                if !target_type.is_numeric() && !matches!(target_type, SemanticType::Unknown) {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "compound assignment requires a numeric variable, but found {}",
                            target_type.name(),
                        ),
                        span: identifier.span,
                    });
                }

                let value = self.infer_expression(&assignment.value, Some(&target_type));

                self.check_compatible(&target_type, &value, assignment.value.span());
            }
        }

        target_type
    }

    fn infer_call_expression(&mut self, call: &CallExpression) -> SemanticType {
        if let Expression::Ident(identifier) = call.callee.as_ref()
            && matches!(identifier.name.as_str(), "all" | "race" | "timeout")
        {
            return self.infer_task_builtin(&identifier.name, call);
        }
        if let Expression::Ident(identifier) = call.callee.as_ref()
            && self.components.contains_key(&identifier.name)
        {
            return self.infer_component_expression(&ComponentExpression {
                span: call.span,
                name: identifier.name.clone(),
                args: call.args.clone(),
                children: Vec::new(),
            });
        }
        if let Expression::Member(member) = call.callee.as_ref() {
            let static_target = if let Expression::Ident(identifier) = member.object.as_ref()
                && self.resolve(&identifier.name).is_none()
                && self.structs.contains_key(&identifier.name)
            {
                if call.type_arguments.is_empty() {
                    Some(SemanticType::Named(identifier.name.clone()))
                } else {
                    let expected = self.structs[&identifier.name].type_parameters.len();
                    if expected != call.type_arguments.len() {
                        self.errors.push(TypeCheckError {
                            message: format!(
                                "type `{}` expects {expected} type argument(s), but received {}",
                                identifier.name,
                                call.type_arguments.len()
                            ),
                            span: call.span,
                        });
                    }
                    Some(SemanticType::Applied(
                        identifier.name.clone(),
                        call.type_arguments
                            .iter()
                            .map(Self::type_from_annotation)
                            .collect(),
                    ))
                }
            } else {
                None
            };
            let target = match &static_target {
                Some(target) => target.clone(),
                None => self.infer_expression(&member.object, None),
            };
            let resolved = self.implementations.iter().find_map(|implementation| {
                let mut substitutions = HashMap::new();
                if implementation.target != target
                    && infer_type_parameters(&implementation.target, &target, &mut substitutions)
                        .is_err()
                {
                    return None;
                }
                implementation
                    .methods
                    .get(&member.property)
                    .map(|signature| {
                        (
                            FunctionSignature {
                                params: signature
                                    .params
                                    .iter()
                                    .map(|parameter| ParameterType {
                                        name: parameter.name.clone(),
                                        type_: substitute_semantic(
                                            &parameter.type_,
                                            &substitutions,
                                        ),
                                    })
                                    .collect(),
                                return_type: substitute_semantic(
                                    &signature.return_type,
                                    &substitutions,
                                ),
                                type_parameters: Vec::new(),
                            },
                            matches!(
                                implementation.trait_.as_ref(),
                                Some(SemanticType::Named(name)) if name == "Drop"
                            ),
                            implementation
                                .method_visibility
                                .get(&member.property)
                                .copied()
                                .unwrap_or(kome_ast::declarations::Visibility::Private),
                        )
                    })
            });
            let Some((signature, is_destructor, visibility)) = resolved else {
                for argument in &call.args {
                    self.infer_expression(
                        match argument {
                            CallArg::Positional(value) => value,
                            CallArg::Named { value, .. } => value,
                        },
                        None,
                    );
                }
                return SemanticType::Unknown;
            };
            self.check_visibility(&target.name(), &member.property, visibility, member.span);
            if is_destructor {
                self.errors.push(TypeCheckError {
                    message: "destructor `drop` cannot be called directly".to_owned(),
                    span: call.span,
                });
            }
            let has_self = signature
                .params
                .first()
                .is_some_and(|parameter| parameter.name == "self");
            if has_self == static_target.is_some() {
                self.errors.push(TypeCheckError {
                    message: format!(
                        "method `{}` is {}",
                        member.property,
                        if has_self {
                            "an instance method"
                        } else {
                            "a static method"
                        }
                    ),
                    span: member.span,
                });
            }
            let parameters = signature.params.iter().skip(usize::from(has_self));
            if call.args.len() != signature.params.len() - usize::from(has_self) {
                self.errors.push(TypeCheckError {
                    message: format!(
                        "method `{}` expects {} argument(s), but received {}",
                        member.property,
                        signature.params.len() - usize::from(has_self),
                        call.args.len()
                    ),
                    span: call.span,
                });
            }
            for (argument, parameter) in call.args.iter().zip(parameters) {
                let expression = match argument {
                    CallArg::Positional(value) => value,
                    CallArg::Named { value, .. } => value,
                };
                let actual = self.infer_expression(expression, Some(&parameter.type_));
                self.check_compatible(&parameter.type_, &actual, expression.span());
            }
            return signature.return_type;
        }
        if let Expression::Ident(identifier) = call.callee.as_ref()
            && let Some(SemanticType::Function(parameters, return_type)) =
                self.resolve(&identifier.name)
        {
            return self.check_callable_arguments(
                &parameters,
                return_type.as_ref(),
                &call.args,
                call.span,
            );
        }
        let Expression::Ident(identifier) = call.callee.as_ref() else {
            let callee = self.infer_expression(&call.callee, None);
            if let SemanticType::Function(parameters, return_type) = callee {
                return self.check_callable_arguments(
                    &parameters,
                    return_type.as_ref(),
                    &call.args,
                    call.span,
                );
            }
            for argument in &call.args {
                self.infer_expression(
                    match argument {
                        CallArg::Positional(expression) => expression,
                        CallArg::Named { value, .. } => value,
                    },
                    None,
                );
            }
            return SemanticType::Unknown;
        };

        let Some(signature) = self.functions.get(&identifier.name).cloned() else {
            return SemanticType::Unknown;
        };

        if !signature.type_parameters.is_empty() {
            let mut substitutions = HashMap::new();
            if !call.type_arguments.is_empty() {
                if call.type_arguments.len() != signature.type_parameters.len() {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "function `{}` expects {} type argument(s), but received {}",
                            identifier.name,
                            signature.type_parameters.len(),
                            call.type_arguments.len()
                        ),
                        span: call.span,
                    });
                }
                for (name, argument) in signature.type_parameters.iter().zip(&call.type_arguments) {
                    substitutions.insert(name.clone(), Self::type_from_annotation(argument));
                }
            }
            let mut actuals = Vec::new();
            for argument in &call.args {
                let expression = match argument {
                    CallArg::Positional(value) => value,
                    CallArg::Named { value, .. } => value,
                };
                actuals.push((expression, self.infer_expression(expression, None)));
            }
            if call.type_arguments.is_empty() {
                for ((_, actual), parameter) in actuals.iter().zip(&signature.params) {
                    if let Err(name) =
                        infer_type_parameters(&parameter.type_, actual, &mut substitutions)
                        && !name.is_empty()
                    {
                        self.errors.push(TypeCheckError {
                            message: format!("conflicting inferred types for `{name}`"),
                            span: call.span,
                        });
                    }
                }
                for name in &signature.type_parameters {
                    if !substitutions.contains_key(name) {
                        self.errors.push(TypeCheckError {
                            message: format!("generic inference failed for `{name}`"),
                            span: call.span,
                        });
                    }
                }
            }
            for ((expression, actual), parameter) in actuals.iter().zip(&signature.params) {
                let expected = substitute_semantic(&parameter.type_, &substitutions);
                self.check_compatible(&expected, actual, expression.span());
            }
            return substitute_semantic(&signature.return_type, &substitutions);
        }

        let mut assigned = vec![false; signature.params.len()];
        let mut next_positional = 0;
        for argument in &call.args {
            let parameter = match argument {
                CallArg::Positional(_) => {
                    while assigned.get(next_positional).copied() == Some(true) {
                        next_positional += 1;
                    }
                    let parameter = signature.params.get(next_positional);
                    if parameter.is_some() {
                        assigned[next_positional] = true;
                        next_positional += 1;
                    }
                    parameter
                }

                CallArg::Named { name, .. } => {
                    signature
                        .params
                        .iter()
                        .enumerate()
                        .find_map(|(index, parameter)| {
                            if parameter.name == *name {
                                assigned[index] = true;
                                Some(parameter)
                            } else {
                                None
                            }
                        })
                }
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

    fn check_callable_arguments(
        &mut self,
        parameters: &[SemanticType],
        return_type: &SemanticType,
        arguments: &[CallArg],
        span: Span,
    ) -> SemanticType {
        if parameters.len() != arguments.len() {
            self.errors.push(TypeCheckError {
                message: format!(
                    "closure expects {} argument(s), but received {}",
                    parameters.len(),
                    arguments.len()
                ),
                span,
            });
        }
        for (argument, expected) in arguments.iter().zip(parameters) {
            let expression = match argument {
                CallArg::Positional(expression) => expression,
                CallArg::Named { value, .. } => value,
            };
            let actual = self.infer_expression(expression, Some(expected));
            self.check_compatible(expected, &actual, expression.span());
        }
        return_type.clone()
    }

    fn infer_task_builtin(&mut self, name: &str, call: &CallExpression) -> SemanticType {
        let task_count = if name == "timeout" {
            if call.args.len() != 2 {
                self.errors.push(TypeCheckError {
                    message: "`timeout` expects a Task and a millisecond duration".into(),
                    span: call.span,
                });
            }
            1
        } else {
            if call.args.is_empty() {
                self.errors.push(TypeCheckError {
                    message: format!("`{name}` requires at least one Task"),
                    span: call.span,
                });
            }
            call.args.len()
        };
        let mut result = None;
        for argument in call.args.iter().take(task_count) {
            let expression = match argument {
                CallArg::Positional(value) => value,
                CallArg::Named { value, .. } => value,
            };
            let actual = self.infer_expression(expression, None);
            let SemanticType::Applied(task, arguments) = actual else {
                self.errors.push(TypeCheckError {
                    message: format!("`{name}` expects Task<T> arguments"),
                    span: expression.span(),
                });
                continue;
            };
            if task != "Task" || arguments.len() != 1 {
                self.errors.push(TypeCheckError {
                    message: format!("`{name}` expects Task<T> arguments"),
                    span: expression.span(),
                });
                continue;
            }
            let actual = arguments[0].clone();
            if let Some(expected) = &result {
                if !self.types_compatible(expected, &actual) {
                    self.errors.push(TypeCheckError {
                        message: format!("`{name}` Task result types must match"),
                        span: expression.span(),
                    });
                }
            } else {
                result = Some(actual);
            }
        }
        if name == "timeout"
            && let Some(argument) = call.args.get(1)
        {
            let expression = match argument {
                CallArg::Positional(value) => value,
                CallArg::Named { value, .. } => value,
            };
            let duration = self.infer_expression(expression, Some(&SemanticType::Number));
            self.check_compatible(&SemanticType::Number, &duration, expression.span());
        }
        let result = result.unwrap_or(SemanticType::Unknown);
        if name == "all" {
            SemanticType::List(Box::new(result))
        } else {
            result
        }
    }

    fn infer_component_expression(&mut self, component: &ComponentExpression) -> SemanticType {
        let Some(signature) = self.components.get(&component.name).cloned() else {
            if self.functions.contains_key(&component.name) {
                let call = trailing_closure_call(component.clone());
                return self.infer_call_expression(&call);
            }
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

        SemanticType::Null
    }

    // -- type conversion --

    fn validate_type_arity(&mut self, type_: &Type) {
        match type_ {
            Type::Named(named) => {
                let expected = self
                    .structs
                    .get(&named.name)
                    .map(|info| info.type_parameters.len())
                    .or_else(|| {
                        self.traits
                            .get(&named.name)
                            .map(|info| info.type_parameters.len())
                    })
                    .or_else(|| (named.name == "Task").then_some(1));
                if let Some(expected) = expected
                    && expected != named.type_arguments.len()
                {
                    self.errors.push(TypeCheckError {
                        message: format!(
                            "type `{}` expects {expected} type argument(s), but received {}",
                            named.name,
                            named.type_arguments.len()
                        ),
                        span: named.span,
                    });
                }
                for argument in &named.type_arguments {
                    self.validate_type_arity(argument);
                }
            }
            Type::Optional(value) => self.validate_type_arity(&value.inner),
            Type::List(value) => self.validate_type_arity(&value.element),
            Type::Pointer(value) => self.validate_type_arity(&value.pointee),
            Type::Function(value) => {
                for parameter in &value.params {
                    self.validate_type_arity(&parameter.type_);
                }
                self.validate_type_arity(&value.return_type);
            }
            Type::Object(value) => {
                for member in &value.members {
                    self.validate_type_arity(&member.type_);
                }
            }
            Type::Primitive(_) => {}
        }
    }

    fn type_from_annotation(type_: &Type) -> SemanticType {
        Self::type_from_annotation_with(type_, &[])
    }

    fn type_from_annotation_with(type_: &Type, parameters: &[String]) -> SemanticType {
        match type_ {
            Type::Primitive(primitive) => match &primitive.kind {
                PrimitiveTypeKind::String => SemanticType::String,
                PrimitiveTypeKind::Number => SemanticType::Number,
                PrimitiveTypeKind::Bool => SemanticType::Bool,
                PrimitiveTypeKind::I8 => SemanticType::I8,
                PrimitiveTypeKind::I16 => SemanticType::I16,
                PrimitiveTypeKind::I32 => SemanticType::I32,
                PrimitiveTypeKind::I64 => SemanticType::I64,
                PrimitiveTypeKind::U8 => SemanticType::U8,
                PrimitiveTypeKind::U16 => SemanticType::U16,
                PrimitiveTypeKind::U32 => SemanticType::U32,
                PrimitiveTypeKind::U64 => SemanticType::U64,
                PrimitiveTypeKind::Isize => SemanticType::Isize,
                PrimitiveTypeKind::Usize => SemanticType::Usize,
                PrimitiveTypeKind::F32 => SemanticType::F32,
                PrimitiveTypeKind::F64 => SemanticType::F64,
                PrimitiveTypeKind::Null => SemanticType::Null,
            },

            Type::Named(named)
                if parameters.contains(&named.name) && named.type_arguments.is_empty() =>
            {
                SemanticType::TypeParameter(named.name.clone())
            }
            Type::Named(named) if named.name == "Void" && named.type_arguments.is_empty() => {
                SemanticType::Void
            }
            Type::Named(named) if !named.type_arguments.is_empty() => SemanticType::Applied(
                named.name.clone(),
                named
                    .type_arguments
                    .iter()
                    .map(|value| Self::type_from_annotation_with(value, parameters))
                    .collect(),
            ),
            Type::Named(named) => SemanticType::Named(named.name.clone()),

            Type::Optional(optional) => SemanticType::Optional(Box::new(
                Self::type_from_annotation_with(&optional.inner, parameters),
            )),

            Type::List(list) => SemanticType::List(Box::new(Self::type_from_annotation_with(
                &list.element,
                parameters,
            ))),
            Type::Pointer(pointer) => SemanticType::Pointer {
                mutability: pointer.mutability,
                pointee: Box::new(Self::type_from_annotation_with(
                    &pointer.pointee,
                    parameters,
                )),
            },

            Type::Function(function) => SemanticType::Function(
                function
                    .params
                    .iter()
                    .map(|parameter| Self::type_from_annotation_with(&parameter.type_, parameters))
                    .collect(),
                Box::new(Self::type_from_annotation_with(
                    &function.return_type,
                    parameters,
                )),
            ),

            Type::Object(_) => SemanticType::Unknown,
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

        if let Some(runtime_type) = self.runtime_type(expected) {
            return self.types_compatible(&runtime_type, actual);
        }

        if let Some(runtime_type) = self.runtime_type(actual) {
            return self.types_compatible(expected, &runtime_type);
        }

        match expected {
            SemanticType::Optional(inner) => {
                actual == &SemanticType::Null || self.types_compatible(inner, actual)
            }

            _ => false,
        }
    }

    fn runtime_type(&self, type_: &SemanticType) -> Option<SemanticType> {
        let SemanticType::Named(name) = type_ else {
            return None;
        };
        let runtime = self.structs.get(name)?.runtime.as_deref()?;

        match runtime {
            "string" => Some(SemanticType::String),
            "number" => Some(SemanticType::Number),
            _ => None,
        }
    }

    fn push_mismatch(&mut self, expected: &SemanticType, actual: &SemanticType, span: Span) {
        self.errors.push(TypeCheckError {
            message: format!("expected {}, but found {}", expected.name(), actual.name(),),
            span,
        });
    }

    fn check_visibility(
        &mut self,
        owner: &str,
        member: &str,
        visibility: kome_ast::declarations::Visibility,
        span: Span,
    ) {
        let owner = owner.split('<').next().unwrap_or(owner);
        let owner_module = module_name_for_declaration(owner);
        let visible = match visibility {
            kome_ast::declarations::Visibility::Public => true,
            kome_ast::declarations::Visibility::Package => {
                package_name(&owner_module) == package_name(&self.current_module)
            }
            kome_ast::declarations::Visibility::Private => owner_module == self.current_module,
        };
        if !visible {
            self.errors.push(TypeCheckError {
                message: format!("member `{member}` of `{owner}` is not visible here"),
                span,
            });
        }
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

fn trailing_closure_call(component: ComponentExpression) -> CallExpression {
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
    CallExpression {
        span: component.span,
        callee: Box::new(Expression::Ident(IdentifierExpression {
            span: component.span,
            name: component.name,
        })),
        type_arguments: Vec::new(),
        args,
    }
}

fn module_name_for_declaration(name: &str) -> String {
    name.rsplit_once("::")
        .map_or_else(|| "__app".to_owned(), |(module, _)| module.to_owned())
}

fn module_name_for_type(type_: &Type) -> String {
    match type_ {
        Type::Named(named) => module_name_for_declaration(&named.name),
        _ => "__app".to_owned(),
    }
}

fn package_name(module: &str) -> &str {
    module.split("::").next().unwrap_or(module)
}
