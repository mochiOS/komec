//! Shared compilation pipeline: AST analysis and Cranelift IR generation.
//!
//! Runtime backends drive [`compile_module`] over their own
//! [`cranelift_module::Module`] implementation.

use crate::error::{CodegenError, CodegenResult};
use crate::types::KomeType;
use cranelift::codegen::ir::MachMemFlags;
use cranelift::codegen::ir::stackslot::StackSize;
use cranelift::codegen::ir::{self, UserFuncName};
use cranelift::prelude::*;
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use kome_ast::AstNode;
use kome_ast::Span;
use kome_ast::declarations::{
    Binding, ComponentMember, Declaration, ForDeclaration, FunctionDeclaration,
    Module as KomeModule, StructDeclaration, TraitDeclaration, TypeMember,
};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, BinaryExpression, BinaryOp, BlockExpression, CallArg,
    CallExpression, CancelExpression, ClosureExpression, Expression, GroupExpression,
    IdentifierExpression, KeyValueProperty, ListExpression, LiteralExpression, LiteralKind,
    MemberExpression, NumberLiteral, ObjectExpression, ObjectProperty, PropertyKey,
    StructExpression, TaskExpression, TemplateExpression, TemplatePart, UnaryExpression, UnaryOp,
    UnwrapExpression, WaitExpression,
};
use kome_ast::patterns::IsPattern;
use kome_ast::statements::{BlockStatement, Statement};
use kome_ast::types::Type;
use std::cell::RefCell;
use std::collections::HashMap;

/// The compiled signature of a Kome function.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    /// Parameter names in ABI order.
    pub param_names: Vec<String>,
    /// Parameter types in ABI order.
    pub params: Vec<KomeType>,
    /// Default expressions aligned with parameters.
    pub defaults: Vec<Option<Expression>>,
    /// Function return type.
    pub ret: KomeType,
}

/// A top-level Kome function, either compiled from Kome source or backed by
/// a registered native.
#[derive(Debug)]
pub enum FunctionKind {
    Native {
        signature: FunctionSignature,

        /// The symbol registered in the native runtime's [`NativeRegistry`](kome_native_rt::NativeRegistry).
        symbol: String,
    },

    /// A function imported directly through the platform C ABI.
    External {
        signature: FunctionSignature,
        symbol: String,
        library: Option<String>,
    },

    User {
        declaration: FunctionDeclaration,
        signature: FunctionSignature,
    },
}

impl FunctionKind {
    /// Returns the native signature shared by this function kind.
    pub fn signature(&self) -> &FunctionSignature {
        match self {
            Self::Native { signature, .. } | Self::External { signature, .. } => signature,
            Self::User { signature, .. } => signature,
        }
    }
}

/// Static information about a module, gathered before code generation.
#[derive(Debug)]
pub struct ModuleInfo {
    functions: HashMap<String, FunctionKind>,
    runtime_types: HashMap<String, KomeType>,
    structs: RefCell<Vec<StructInfo>>,
    struct_ids: HashMap<String, usize>,
    implementations: Vec<TypeImplementation>,
    task_types: RefCell<Vec<KomeType>>,
    list_types: RefCell<Vec<KomeType>>,
    optional_types: RefCell<Vec<KomeType>>,
    closure_types: RefCell<Vec<FunctionSignature>>,
    enums: Vec<EnumInfo>,
    globals: HashMap<String, GlobalInfo>,
    components: HashMap<String, ComponentInfo>,
}

#[derive(Debug, Clone)]
struct ComponentInfo {
    param_names: Vec<String>,
    param_types: Vec<KomeType>,
    defaults: Vec<Option<Expression>>,
    body: Option<Vec<ComponentMember>>,
}

#[derive(Debug, Clone)]
struct GlobalInfo {
    binding: Binding,
    kome_type: Option<KomeType>,
}

#[derive(Debug, Clone, Copy)]
struct GlobalStorage {
    value: DataId,
    initialized: DataId,
}

#[derive(Debug, Clone)]
struct EnumInfo {
    name: String,
    cases: Vec<String>,
    raw_values: Vec<Option<Expression>>,
}

#[derive(Debug, Clone)]
struct StructFieldInfo {
    name: String,
    kome_type: KomeType,
    offset: i32,
}

#[derive(Debug, Clone)]
struct StructInfo {
    name: String,
    fields: Vec<StructFieldInfo>,
    size: u32,
    anonymous: bool,
}

#[derive(Debug, Clone)]
struct ImplementationMethod {
    function_key: String,
    signature: FunctionSignature,
    has_self: bool,
}

#[derive(Debug)]
struct AssociatedConstant {
    binding: Binding,
    kome_type: KomeType,
}

#[derive(Debug)]
struct TypeImplementation {
    target: KomeType,
    trait_name: Option<String>,
    methods: HashMap<String, ImplementationMethod>,
    constants: HashMap<String, AssociatedConstant>,
}

impl ModuleInfo {
    /// Looks up an analyzed function by its deterministic codegen key.
    pub fn get(&self, name: &str) -> Option<&FunctionKind> {
        self.functions.get(name)
    }

    /// Looks up the concrete representation of a runtime-backed named type.
    pub fn runtime_type(&self, name: &str) -> Option<KomeType> {
        self.runtime_types.get(name).copied()
    }

    fn struct_info(&self, id: usize) -> StructInfo {
        self.structs.borrow()[id].clone()
    }

    fn type_name(&self, ty: KomeType) -> String {
        match ty {
            KomeType::Struct(id) => self.structs.borrow()[id].name.clone(),
            KomeType::Task(id) => format!("Task<{}>", self.type_name(self.task_result(id))),
            KomeType::List(id) => format!("{}[]", self.type_name(self.list_element(id))),
            KomeType::Optional(id) => format!("{}?", self.type_name(self.optional_inner(id))),
            KomeType::Closure(id) => {
                let signature = self.closure_signature(id);
                format!(
                    "({}) -> {}",
                    signature
                        .params
                        .iter()
                        .map(|type_| self.type_name(*type_))
                        .collect::<Vec<_>>()
                        .join(", "),
                    self.type_name(signature.ret)
                )
            }
            KomeType::Enum(id) => self.enums[id].name.clone(),
            _ => ty.name(),
        }
    }

    fn task_type(&self, result: KomeType) -> KomeType {
        let mut task_types = self.task_types.borrow_mut();
        let id = task_types
            .iter()
            .position(|existing| *existing == result)
            .unwrap_or_else(|| {
                task_types.push(result);
                task_types.len() - 1
            });
        KomeType::Task(id)
    }

    fn task_result(&self, id: usize) -> KomeType {
        self.task_types.borrow()[id]
    }

    fn list_type(&self, element: KomeType) -> KomeType {
        let mut list_types = self.list_types.borrow_mut();
        let id = list_types
            .iter()
            .position(|existing| *existing == element)
            .unwrap_or_else(|| {
                list_types.push(element);
                list_types.len() - 1
            });
        KomeType::List(id)
    }

    fn list_element(&self, id: usize) -> KomeType {
        self.list_types.borrow()[id]
    }

    fn optional_inner(&self, id: usize) -> KomeType {
        self.optional_types.borrow()[id]
    }

    fn closure_signature(&self, id: usize) -> FunctionSignature {
        self.closure_types.borrow()[id].clone()
    }

    fn implementation_member(
        &self,
        target: KomeType,
        name: &str,
    ) -> Option<(
        &TypeImplementation,
        Option<&ImplementationMethod>,
        Option<&AssociatedConstant>,
    )> {
        self.implementations
            .iter()
            .filter_map(|implementation| {
                if implementation.target != target {
                    return None;
                }
                let method = implementation.methods.get(name);
                let constant = implementation.constants.get(name);
                (method.is_some() || constant.is_some()).then_some((
                    implementation,
                    method,
                    constant,
                ))
            })
            .min_by_key(|(implementation, _, _)| implementation.trait_name.is_some())
    }

    fn drop_method(&self, target: KomeType) -> Option<&ImplementationMethod> {
        self.implementations
            .iter()
            .find(|implementation| {
                implementation.target == target
                    && implementation.trait_name.as_deref() == Some("Drop")
            })
            .and_then(|implementation| implementation.methods.get("drop"))
    }

    fn is_drop_function(&self, function_key: &str) -> bool {
        self.implementations.iter().any(|implementation| {
            implementation.trait_name.as_deref() == Some("Drop")
                && implementation
                    .methods
                    .get("drop")
                    .is_some_and(|method| method.function_key == function_key)
        })
    }

    /// The entry point's signature, validated for direct invocation.
    pub fn entry_signature(&self, entry: &str) -> CodegenResult<&FunctionSignature> {
        match self.get(entry) {
            None => Err(CodegenError::new(
                format!("entry function `{entry}` was not found"),
                None,
            )),

            Some(FunctionKind::Native { .. } | FunctionKind::External { .. }) => {
                Err(CodegenError::new(
                    format!("entry function `{entry}` must be defined in Kome"),
                    None,
                ))
            }

            Some(FunctionKind::User { signature, .. }) => {
                if !signature.params.is_empty() {
                    return Err(CodegenError::new(
                        format!("entry function `{entry}` must not take parameters"),
                        None,
                    ));
                }

                Ok(signature)
            }
        }
    }

    /// Iterates over Kome-defined functions and their analyzed signatures.
    pub fn user_functions(
        &self,
    ) -> impl Iterator<Item = (&str, &FunctionDeclaration, &FunctionSignature)> {
        self.functions.iter().filter_map(|(name, kind)| match kind {
            FunctionKind::User {
                declaration,
                signature,
            } => Some((name.as_str(), declaration, signature)),
            FunctionKind::Native { .. } | FunctionKind::External { .. } => None,
        })
    }

    /// Iterates over functions imported directly from C libraries.
    pub fn external_functions(
        &self,
    ) -> impl Iterator<Item = (&str, &str, Option<&str>, &FunctionSignature)> {
        self.functions.iter().filter_map(|(name, kind)| match kind {
            FunctionKind::External {
                signature,
                symbol,
                library,
            } => Some((
                name.as_str(),
                symbol.as_str(),
                library.as_deref(),
                signature,
            )),
            _ => None,
        })
    }

    /// Returns each explicitly requested C library once.
    pub fn external_libraries(&self) -> Vec<&str> {
        let mut libraries = Vec::new();
        for (_, _, library, _) in self.external_functions() {
            if let Some(library) = library
                && !libraries.contains(&library)
            {
                libraries.push(library);
            }
        }
        libraries
    }
}

/// Collects function declarations and computes their signatures.
pub fn analyze_module(module: &KomeModule) -> CodegenResult<ModuleInfo> {
    let module = crate::generics::monomorphize(module)?;
    let module = &module;
    let mut functions = HashMap::new();
    let mut runtime_types = HashMap::new();
    let mut struct_ids = HashMap::new();
    let task_types = RefCell::new(Vec::new());
    let list_types = RefCell::new(Vec::new());
    let optional_types = RefCell::new(Vec::new());
    let closure_types = RefCell::new(Vec::new());
    let mut enums = Vec::new();
    let mut globals = HashMap::new();

    for declaration in &module.declarations {
        let binding = match declaration {
            Declaration::Let(binding) | Declaration::Constant(binding) => binding,
            _ => continue,
        };
        let kome_ast::patterns::Pattern::Ident(identifier) = &binding.pattern else {
            return Err(CodegenError::at(
                "top-level bindings require an identifier pattern",
                binding.pattern.span(),
            ));
        };
        if globals
            .insert(
                identifier.name.clone(),
                GlobalInfo {
                    binding: binding.clone(),
                    kome_type: None,
                },
            )
            .is_some()
        {
            return Err(CodegenError::at(
                format!("duplicate global binding `{}`", identifier.name),
                identifier.span,
            ));
        }
    }

    for declaration in &module.declarations {
        let Declaration::Struct(struct_decl) = declaration else {
            continue;
        };

        if let Some(runtime_type) = runtime_type(struct_decl)? {
            runtime_types.insert(struct_decl.name.clone(), runtime_type);
        } else {
            let next = struct_ids.len();
            if struct_ids.insert(struct_decl.name.clone(), next).is_some() {
                return Err(CodegenError::at(
                    format!("duplicate struct `{}`", struct_decl.name),
                    struct_decl.span,
                ));
            }
        }
    }

    for declaration in &module.declarations {
        let Declaration::Enum(enum_) = declaration else {
            continue;
        };
        let id = enums.len();
        if runtime_types
            .insert(enum_.name.clone(), KomeType::Enum(id))
            .is_some()
            || struct_ids.contains_key(&enum_.name)
        {
            return Err(CodegenError::at(
                format!("duplicate type `{}`", enum_.name),
                enum_.span,
            ));
        }
        enums.push(EnumInfo {
            name: enum_.name.clone(),
            cases: enum_.cases.iter().map(|case| case.name.clone()).collect(),
            raw_values: enum_.cases.iter().map(|case| case.value.clone()).collect(),
        });
    }

    let mut structs = vec![None; struct_ids.len()];
    for declaration in &module.declarations {
        let Declaration::Struct(struct_decl) = declaration else {
            continue;
        };
        let Some(&id) = struct_ids.get(&struct_decl.name) else {
            continue;
        };
        let declared = struct_decl.fields.as_ref().ok_or_else(|| {
            CodegenError::at(
                format!(
                    "opaque struct `{}` has no code generation layout",
                    struct_decl.name
                ),
                struct_decl.span,
            )
        })?;
        let mut fields = Vec::with_capacity(declared.len());
        for (index, field) in declared.iter().enumerate() {
            let kome_type = type_from_annotation(
                &field.type_,
                &runtime_types,
                &struct_ids,
                &task_types,
                &list_types,
                &optional_types,
                &closure_types,
            )?;
            if kome_type == KomeType::Void {
                return Err(CodegenError::at(
                    "struct fields cannot have type Void",
                    field.span,
                ));
            }
            if fields
                .iter()
                .any(|existing: &StructFieldInfo| existing.name == field.name)
            {
                return Err(CodegenError::at(
                    format!("duplicate field `{}`", field.name),
                    field.span,
                ));
            }
            fields.push(StructFieldInfo {
                name: field.name.clone(),
                kome_type,
                offset: (index * 8) as i32,
            });
        }
        structs[id] = Some(StructInfo {
            name: struct_decl.name.clone(),
            size: (fields.len() * 8) as u32,
            fields,
            anonymous: false,
        });
    }
    let mut structs = structs.into_iter().map(Option::unwrap).collect::<Vec<_>>();
    let mut components = HashMap::new();
    for declaration in &module.declarations {
        let Declaration::Component(component) = declaration else {
            continue;
        };
        let mut param_names = Vec::with_capacity(component.params.len());
        let mut param_types = Vec::with_capacity(component.params.len());
        let mut defaults = Vec::with_capacity(component.params.len());
        for parameter in &component.params {
            param_names.push(parameter.name.clone());
            param_types.push(type_from_annotation(
                &parameter.type_,
                &runtime_types,
                &struct_ids,
                &task_types,
                &list_types,
                &optional_types,
                &closure_types,
            )?);
            defaults.push(parameter.default.clone());
        }
        if components
            .insert(
                component.name.clone(),
                ComponentInfo {
                    param_names,
                    param_types,
                    defaults,
                    body: component.body.clone(),
                },
            )
            .is_some()
        {
            return Err(CodegenError::at(
                format!("duplicate component `{}`", component.name),
                component.span,
            ));
        }
    }

    for declaration in &module.declarations {
        let Declaration::Function(function) = declaration else {
            continue;
        };

        let signature = analyze_signature(
            function,
            &runtime_types,
            &struct_ids,
            &task_types,
            &list_types,
            &optional_types,
            &closure_types,
            None,
        )?;

        let kind = match native_symbol(function)? {
            Some(symbol) => FunctionKind::Native {
                signature,
                symbol: symbol.to_owned(),
            },

            None => FunctionKind::User {
                declaration: function.clone(),
                signature,
            },
        };

        if functions.insert(function.name.clone(), kind).is_some() {
            return Err(CodegenError::at(
                format!("duplicate function `{}`", function.name),
                function.span,
            ));
        }
    }

    for declaration in &module.declarations {
        let Declaration::Extern(external) = declaration else {
            continue;
        };
        if external.abi != "C" {
            return Err(CodegenError::at(
                format!("unsupported external ABI `{}`; expected `C`", external.abi),
                external.span,
            ));
        }
        for item in &external.items {
            let kome_ast::declarations::ExternItem::Function(function) = item else {
                continue;
            };
            if !function.type_parameters.is_empty() {
                return Err(CodegenError::at(
                    "external C functions cannot be generic",
                    function.span,
                ));
            }
            if function.params.iter().any(|parameter| {
                matches!(parameter, kome_ast::patterns::Pattern::Ident(identifier) if identifier.default.is_some())
            }) {
                return Err(CodegenError::at(
                    "external C functions cannot have default arguments",
                    function.span,
                ));
            }
            let signature = analyze_signature(
                function,
                &runtime_types,
                &struct_ids,
                &task_types,
                &list_types,
                &optional_types,
                &closure_types,
                None,
            )?;
            let kind = FunctionKind::External {
                signature,
                symbol: function.name.clone(),
                library: external.library.clone(),
            };
            if functions.insert(function.name.clone(), kind).is_some() {
                return Err(CodegenError::at(
                    format!("duplicate function `{}`", function.name),
                    function.span,
                ));
            }
        }
    }

    let applications = module
        .declarations
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::Component(component)
                if component
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "application") =>
            {
                Some(component)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if applications.len() > 1 {
        return Err(CodegenError::at(
            "only one component may have the @application attribute",
            applications[1].span,
        ));
    }
    if let Some(application) = applications.first()
        && !functions.contains_key("main")
    {
        if application
            .params
            .iter()
            .any(|parameter| parameter.default.is_none())
        {
            return Err(CodegenError::at(
                "an @application component cannot require arguments",
                application.span,
            ));
        }
        let expression = Expression::Component(kome_ast::expressions::ComponentExpression {
            span: application.span,
            name: application.name.clone(),
            args: Vec::new(),
            children: Vec::new(),
        });
        let declaration = FunctionDeclaration {
            span: application.span,
            visibility: kome_ast::declarations::Visibility::Private,
            attributes: Vec::new(),
            name: "main".into(),
            type_parameters: Vec::new(),
            params: Vec::new(),
            body: Some(BlockStatement {
                span: application.span,
                statements: vec![Statement::Expression(
                    kome_ast::statements::ExpressionStatement {
                        span: application.span,
                        expression,
                    },
                )],
            }),
            return_type: None,
        };
        functions.insert(
            "main".into(),
            FunctionKind::User {
                declaration,
                signature: FunctionSignature {
                    param_names: Vec::new(),
                    params: Vec::new(),
                    defaults: Vec::new(),
                    ret: KomeType::Void,
                },
            },
        );
    }

    let traits = module
        .declarations
        .iter()
        .filter_map(|declaration| match declaration {
            Declaration::Trait(declaration) => Some((declaration.name.clone(), declaration)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut implementations = Vec::new();
    for declaration in &module.declarations {
        let Declaration::For(implementation) = declaration else {
            continue;
        };
        let target = type_from_annotation(
            &implementation.target,
            &runtime_types,
            &struct_ids,
            &task_types,
            &list_types,
            &optional_types,
            &closure_types,
        )?;
        let trait_name = implementation
            .trait_
            .as_ref()
            .map(|annotation| match annotation {
                kome_ast::types::Type::Named(named) => Ok(named.name.clone()),
                _ => Err(CodegenError::at(
                    "implementation trait must be a named type",
                    annotation.span(),
                )),
            })
            .transpose()?;
        let mut methods = HashMap::new();
        let mut constants = HashMap::new();
        for member in &implementation.members {
            match member {
                TypeMember::Function(function) => {
                    let signature = analyze_signature(
                        function,
                        &runtime_types,
                        &struct_ids,
                        &task_types,
                        &list_types,
                        &optional_types,
                        &closure_types,
                        Some(target),
                    )?;
                    let has_self = matches!(function.params.first(), Some(kome_ast::patterns::Pattern::Ident(parameter)) if parameter.name == "self");
                    let key =
                        implementation_function_key(target, trait_name.as_deref(), &function.name);
                    if methods
                        .insert(
                            function.name.clone(),
                            ImplementationMethod {
                                function_key: key.clone(),
                                signature: signature.clone(),
                                has_self,
                            },
                        )
                        .is_some()
                    {
                        return Err(CodegenError::at(
                            format!("duplicate implementation method `{}`", function.name),
                            function.span,
                        ));
                    }
                    if functions
                        .insert(
                            key,
                            FunctionKind::User {
                                declaration: function.clone(),
                                signature,
                            },
                        )
                        .is_some()
                    {
                        return Err(CodegenError::at(
                            format!("duplicate implementation method `{}`", function.name),
                            function.span,
                        ));
                    }
                }
                TypeMember::Constant(binding) => {
                    let kome_ast::patterns::Pattern::Ident(identifier) = &binding.pattern else {
                        return Err(CodegenError::at(
                            "associated constants require an identifier",
                            binding.span,
                        ));
                    };
                    let annotation = binding.type_annotation.as_ref().ok_or_else(|| {
                        CodegenError::at(
                            format!(
                                "associated constant `{}` requires a type annotation",
                                identifier.name
                            ),
                            binding.span,
                        )
                    })?;
                    let kome_type = type_from_annotation(
                        annotation,
                        &runtime_types,
                        &struct_ids,
                        &task_types,
                        &list_types,
                        &optional_types,
                        &closure_types,
                    )?;
                    constants.insert(
                        identifier.name.clone(),
                        AssociatedConstant {
                            binding: binding.clone(),
                            kome_type,
                        },
                    );
                }
            }
        }
        if let Some(trait_name) = &trait_name {
            let trait_decl = traits.get(trait_name).ok_or_else(|| {
                CodegenError::at(
                    format!("trait `{trait_name}` was not found"),
                    implementation.span,
                )
            })?;
            validate_trait_implementation(
                implementation,
                trait_decl,
                target,
                &methods,
                &runtime_types,
                &struct_ids,
                &task_types,
                &list_types,
                &optional_types,
                &closure_types,
            )?;
            if trait_name == "Drop" {
                if !matches!(target, KomeType::Struct(_)) {
                    return Err(CodegenError::at(
                        "`Drop` can only be implemented by a user-defined struct",
                        implementation.span,
                    ));
                }
                if implementations.iter().any(|existing: &TypeImplementation| {
                    existing.target == target && existing.trait_name.as_deref() == Some("Drop")
                }) {
                    return Err(CodegenError::at(
                        format!(
                            "type `{}` has more than one `Drop` implementation",
                            type_code(target)
                        ),
                        implementation.span,
                    ));
                }
            }
        }
        implementations.push(TypeImplementation {
            target,
            trait_name,
            methods,
            constants,
        });
    }

    let global_names = globals.keys().cloned().collect::<Vec<_>>();
    let mut global_type_cache = HashMap::new();
    for name in global_names {
        let binding = &globals[&name].binding;
        let is_closure = binding
            .init
            .as_ref()
            .is_some_and(|initializer| match initializer {
                Expression::Closure(_) => true,
                Expression::Group(group) => {
                    matches!(group.expression.as_ref(), Expression::Closure(_))
                }
                _ => false,
            });
        if is_closure {
            continue;
        }
        let kome_type = infer_global_type(
            &name,
            &globals,
            &functions,
            &runtime_types,
            &struct_ids,
            &mut structs,
            &implementations,
            &task_types,
            &list_types,
            &optional_types,
            &closure_types,
            &mut global_type_cache,
            &mut Vec::new(),
        )?;
        globals.get_mut(&name).expect("global exists").kome_type = Some(kome_type);
    }

    Ok(ModuleInfo {
        functions,
        runtime_types,
        structs: RefCell::new(structs),
        struct_ids,
        implementations,
        task_types,
        list_types,
        optional_types,
        closure_types,
        enums,
        globals,
        components,
    })
}

fn type_code(ty: KomeType) -> String {
    match ty {
        KomeType::Struct(id) => format!("S{id}"),
        _ => ty.name(),
    }
}

fn encode_symbol_part(value: &str) -> String {
    format!("{}_{}", value.len(), value)
}

fn implementation_function_key(target: KomeType, trait_name: Option<&str>, method: &str) -> String {
    format!(
        "impl${}${}${}",
        encode_symbol_part(&type_code(target)),
        encode_symbol_part(trait_name.unwrap_or("")),
        encode_symbol_part(method)
    )
}

fn validate_trait_implementation(
    implementation: &ForDeclaration,
    trait_decl: &TraitDeclaration,
    target: KomeType,
    methods: &HashMap<String, ImplementationMethod>,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    task_types: &RefCell<Vec<KomeType>>,
    list_types: &RefCell<Vec<KomeType>>,
    optional_types: &RefCell<Vec<KomeType>>,
    closure_types: &RefCell<Vec<FunctionSignature>>,
) -> CodegenResult<()> {
    for required in &trait_decl.functions {
        let expected = analyze_signature(
            required,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
            Some(target),
        )?;
        let actual = methods.get(&required.name).ok_or_else(|| {
            CodegenError::at(
                format!(
                    "implementation of `{}` is missing required method `{}`",
                    trait_decl.name, required.name
                ),
                implementation.span,
            )
        })?;
        if actual.signature.params != expected.params || actual.signature.ret != expected.ret {
            return Err(CodegenError::at(
                format!(
                    "method `{}` has an incompatible signature for trait `{}`",
                    required.name, trait_decl.name
                ),
                implementation.span,
            ));
        }
    }
    Ok(())
}

fn runtime_type(struct_decl: &StructDeclaration) -> CodegenResult<Option<KomeType>> {
    let mut attributes = struct_decl
        .attributes
        .iter()
        .filter(|attribute| attribute.name == "runtime");
    let Some(attribute) = attributes.next() else {
        return Ok(None);
    };

    let invalid = |message: &'static str| {
        CodegenError::at(
            format!(
                "invalid @runtime attribute on `{}`: {message}",
                struct_decl.name
            ),
            attribute.span,
        )
    };

    if attributes.next().is_some() {
        return Err(invalid("attribute appears more than once"));
    }

    if attribute.args.len() != 1 {
        return Err(invalid("expected exactly one string argument"));
    }

    let Expression::Literal(LiteralExpression {
        kind: LiteralKind::String(name),
        ..
    }) = &attribute.args[0]
    else {
        return Err(invalid("argument must be a string literal"));
    };

    KomeType::from_runtime_name(name, attribute.span).map(Some)
}

/// Extracts the symbol name from an `@native("symbol")` attribute.
pub fn native_symbol(function: &FunctionDeclaration) -> CodegenResult<Option<&str>> {
    let mut attributes = function
        .attributes
        .iter()
        .filter(|attribute| attribute.name == "native");

    let Some(attribute) = attributes.next() else {
        return Ok(None);
    };

    let invalid = |message: &'static str| {
        CodegenError::at(
            format!(
                "invalid @native attribute on `{}`: {message}",
                function.name
            ),
            attribute.span,
        )
    };

    if attributes.next().is_some() {
        return Err(invalid("attribute appears more than once"));
    }

    if attribute.args.len() != 1 {
        return Err(invalid("expected exactly one string argument"));
    }

    let Expression::Literal(LiteralExpression {
        kind: LiteralKind::String(symbol),
        ..
    }) = &attribute.args[0]
    else {
        return Err(invalid("argument must be a string literal"));
    };

    Ok(Some(symbol.as_str()))
}

/// Returns the deterministic native symbol for a Kome function name.
pub fn mangled_name(name: &str) -> String {
    if name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return format!("kome_{name}");
    }

    let encoded = name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("kome_q{encoded}")
}

/// Compiles every user function in the module.
///
/// Functions are pre-declared so that arbitrary forward references work.
/// Returns the mapping from Kome function names to module function ids.
pub fn compile_module<M: Module>(
    info: &ModuleInfo,
    module: &mut M,
) -> CodegenResult<HashMap<String, FuncId>> {
    let mut func_ids = HashMap::new();
    let mut task_entry_ids = HashMap::new();
    let mut closure_drop_ids = HashMap::new();
    let mut global_storage = HashMap::new();

    for (name, global) in &info.globals {
        if global.kome_type.is_none() {
            continue;
        }
        let encoded = encode_symbol_part(name);
        let value = module
            .declare_data(
                &format!("kome_global_value_{encoded}"),
                Linkage::Local,
                true,
                false,
            )
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        let mut value_description = DataDescription::new();
        value_description.define_zeroinit(8);
        value_description.set_align(8);
        module
            .define_data(value, &value_description)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        let initialized = module
            .declare_data(
                &format!("kome_global_initialized_{encoded}"),
                Linkage::Local,
                true,
                false,
            )
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        let mut flag_description = DataDescription::new();
        flag_description.define_zeroinit(1);
        module
            .define_data(initialized, &flag_description)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        global_storage.insert(name.clone(), GlobalStorage { value, initialized });
    }

    for (name, kind) in &info.functions {
        let (link_name, linkage, signature) = match kind {
            FunctionKind::User { signature, .. } => {
                (mangled_name(name), Linkage::Export, signature)
            }
            FunctionKind::External {
                signature, symbol, ..
            } => (symbol.clone(), Linkage::Import, signature),
            FunctionKind::Native { .. } => continue,
        };
        let cranelift_signature = build_signature(module, signature)?;
        let func_id = module
            .declare_function(&link_name, linkage, &cranelift_signature)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        func_ids.insert(name.to_owned(), func_id);

        if matches!(kind, FunctionKind::User { .. }) && name.starts_with("__kome_task_body_") {
            let mut entry_signature = module.make_signature();
            entry_signature.params.push(AbiParam::new(types::I64));
            entry_signature
                .params
                .push(AbiParam::new(module.target_config().pointer_type()));
            entry_signature.params.push(AbiParam::new(types::I8));
            entry_signature.returns.push(AbiParam::new(types::I64));
            let entry_id = module
                .declare_function(
                    &format!("{}_entry", mangled_name(name)),
                    Linkage::Local,
                    &entry_signature,
                )
                .map_err(|error| CodegenError::new(error.to_string(), None))?;
            task_entry_ids.insert(name.to_owned(), entry_id);
        }
    }

    for (id, layout) in info.structs.borrow().iter().enumerate() {
        if !layout.name.starts_with("__kome_closure_environment_") {
            continue;
        }
        let mut signature = module.make_signature();
        signature.params.push(AbiParam::new(types::I64));
        let func_id = module
            .declare_function(
                &format!(
                    "kome_closure_drop_{}_{}",
                    encode_symbol_part(&layout.name),
                    id
                ),
                Linkage::Local,
                &signature,
            )
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        closure_drop_ids.insert(id, func_id);
    }

    let foreign = ForeignFunctions::declare(module)?;
    let mut native_symbols = NativeSymbolPool::default();

    for (name, declaration, signature) in info.user_functions() {
        let func_id = func_ids[name];

        let mut context = module.make_context();
        context.func.signature = build_signature(module, signature)?;
        context.func.name = UserFuncName::user(0, func_id.as_u32());

        let mut function_builder_context = FunctionBuilderContext::new();

        {
            let builder = FunctionBuilder::new(&mut context.func, &mut function_builder_context);

            let translator = FunctionTranslator {
                builder,
                module,
                info,
                func_ids: &func_ids,
                task_entry_ids: &task_entry_ids,
                closure_drop_ids: &closure_drop_ids,
                global_storage: &global_storage,
                foreign: &foreign,
                native_symbols: &mut native_symbols,
                scopes: vec![HashMap::new()],
                remaining_reads: HashMap::new(),
                next_binding_id: 0,
                return_type: signature.ret,
                terminated: false,
                loops: Vec::new(),
                allow_managed_moves: true,
                evaluating_globals: Vec::new(),
                closures: Vec::new(),
                local_functions: Vec::new(),
                local_call_stack: Vec::new(),
                inline_returns: Vec::new(),
            };

            translator.translate_function(declaration, signature, info.is_drop_function(name))?;
        }

        module
            .define_function(func_id, &mut context)
            .map_err(|error| {
                CodegenError::at(
                    format!("failed to compile function `{name}`: {error:?}"),
                    declaration.span,
                )
            })?;
    }

    for (&environment_id, &drop_id) in &closure_drop_ids {
        let mut context = module.make_context();
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I64));
        context.func.name = UserFuncName::user(0, drop_id.as_u32());
        let mut function_builder_context = FunctionBuilderContext::new();
        {
            let mut builder =
                FunctionBuilder::new(&mut context.func, &mut function_builder_context);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);
            let environment = builder.block_params(entry)[0];
            let mut translator = FunctionTranslator {
                builder,
                module,
                info,
                func_ids: &func_ids,
                task_entry_ids: &task_entry_ids,
                closure_drop_ids: &closure_drop_ids,
                global_storage: &global_storage,
                foreign: &foreign,
                native_symbols: &mut native_symbols,
                scopes: vec![HashMap::new()],
                remaining_reads: HashMap::new(),
                next_binding_id: 0,
                return_type: KomeType::Void,
                terminated: false,
                loops: Vec::new(),
                allow_managed_moves: true,
                evaluating_globals: Vec::new(),
                closures: Vec::new(),
                local_functions: Vec::new(),
                local_call_stack: Vec::new(),
                inline_returns: Vec::new(),
            };
            translator.release_struct(environment, environment_id);
            translator.builder.ins().return_(&[]);
            translator
                .builder
                .finalize(translator.module.target_config());
        }
        module
            .define_function(drop_id, &mut context)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
    }

    for (name, entry_id) in &task_entry_ids {
        let signature = info
            .get(name)
            .expect("task body is a user function")
            .signature();
        let mut context = module.make_context();
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I64));
        context
            .func
            .signature
            .params
            .push(AbiParam::new(module.target_config().pointer_type()));
        context.func.signature.params.push(AbiParam::new(types::I8));
        context
            .func
            .signature
            .returns
            .push(AbiParam::new(types::I64));
        context.func.name = UserFuncName::user(0, entry_id.as_u32());
        let mut function_builder_context = FunctionBuilderContext::new();
        {
            let builder = FunctionBuilder::new(&mut context.func, &mut function_builder_context);
            let translator = FunctionTranslator {
                builder,
                module,
                info,
                func_ids: &func_ids,
                task_entry_ids: &task_entry_ids,
                closure_drop_ids: &closure_drop_ids,
                global_storage: &global_storage,
                foreign: &foreign,
                native_symbols: &mut native_symbols,
                scopes: vec![HashMap::new()],
                remaining_reads: HashMap::new(),
                next_binding_id: 0,
                return_type: signature.ret,
                terminated: false,
                loops: Vec::new(),
                allow_managed_moves: true,
                evaluating_globals: Vec::new(),
                closures: Vec::new(),
                local_functions: Vec::new(),
                local_call_stack: Vec::new(),
                inline_returns: Vec::new(),
            };
            translator.translate_task_entry(name, signature)?;
        }
        module
            .define_function(*entry_id, &mut context)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
    }

    Ok(func_ids)
}

/// Foreign runtime symbols referenced by generated code.
struct ForeignFunctions {
    native_call: FuncId,
    number_parse: FuncId,
    number_retain: FuncId,
    number_release: FuncId,
    number_add: FuncId,
    number_sub: FuncId,
    number_mul: FuncId,
    number_div: FuncId,
    number_compare: FuncId,
    number_to_i64: FuncId,
    number_to_string: FuncId,
    string_create: FuncId,
    string_retain: FuncId,
    string_release: FuncId,
    string_concat: FuncId,
    string_compare: FuncId,
    boolean_to_string: FuncId,
    signed_integer_to_string: FuncId,
    unsigned_integer_to_string: FuncId,
    f32_to_string: FuncId,
    f64_to_string: FuncId,
    socket_retain: FuncId,
    socket_release: FuncId,
    struct_alloc: FuncId,
    struct_retain: FuncId,
    struct_release: FuncId,
    struct_dealloc: FuncId,
    closure_alloc: FuncId,
    closure_retain: FuncId,
    closure_release: FuncId,
    closure_code: FuncId,
    closure_environment: FuncId,
    optional_require: FuncId,
    task_spawn: FuncId,
    task_cancel: FuncId,
    task_is_cancelled: FuncId,
    task_wait: FuncId,
    task_result: FuncId,
    task_retain: FuncId,
    task_release: FuncId,
    task_dealloc: FuncId,
    task_race: FuncId,
    task_wait_timeout: FuncId,
    task_all: FuncId,
    task_require_completed: FuncId,
    task_require_race_winner: FuncId,
    list_alloc: FuncId,
    list_retain: FuncId,
    list_release: FuncId,
    list_dealloc: FuncId,
    list_len: FuncId,
    list_require_index: FuncId,
}

impl ForeignFunctions {
    fn declare<M: Module>(module: &mut M) -> CodegenResult<Self> {
        let native_call = declare_foreign(
            module,
            "__kome_native_call",
            &[types::I64, types::I64, types::I64, types::I64],
            Some(types::I64),
        )?;

        let number_parse = declare_foreign(
            module,
            "__kome_number_parse",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let optional_require = declare_foreign(
            module,
            "__kome_optional_require",
            &[types::I64],
            Some(types::I64),
        )?;

        let number_retain = declare_foreign(module, "__kome_number_retain", &[types::I64], None)?;
        let number_release = declare_foreign(module, "__kome_number_release", &[types::I64], None)?;

        let number_add = declare_foreign(
            module,
            "__kome_number_add",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let number_sub = declare_foreign(
            module,
            "__kome_number_sub",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let number_mul = declare_foreign(
            module,
            "__kome_number_mul",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let number_div = declare_foreign(
            module,
            "__kome_number_div",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let number_compare = declare_foreign(
            module,
            "__kome_number_compare",
            &[types::I64, types::I64],
            Some(types::I32),
        )?;
        let number_to_i64 = declare_foreign(
            module,
            "__kome_number_to_i64",
            &[types::I64],
            Some(types::I64),
        )?;
        let number_to_string = declare_foreign(
            module,
            "__kome_number_to_string",
            &[types::I64],
            Some(types::I64),
        )?;

        let string_create = declare_foreign(
            module,
            "__kome_string_create",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let string_retain = declare_foreign(module, "__kome_string_retain", &[types::I64], None)?;
        let string_release = declare_foreign(module, "__kome_string_release", &[types::I64], None)?;
        let string_concat = declare_foreign(
            module,
            "__kome_string_concat",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;
        let string_compare = declare_foreign(
            module,
            "__kome_string_compare",
            &[types::I64, types::I64],
            Some(types::I32),
        )?;
        let boolean_to_string = declare_foreign(
            module,
            "__kome_boolean_to_string",
            &[types::I8],
            Some(types::I64),
        )?;
        let signed_integer_to_string = declare_foreign(
            module,
            "__kome_signed_integer_to_string",
            &[types::I64],
            Some(types::I64),
        )?;
        let unsigned_integer_to_string = declare_foreign(
            module,
            "__kome_unsigned_integer_to_string",
            &[types::I64],
            Some(types::I64),
        )?;
        let f32_to_string = declare_foreign(
            module,
            "__kome_f32_to_string",
            &[types::F32],
            Some(types::I64),
        )?;
        let f64_to_string = declare_foreign(
            module,
            "__kome_f64_to_string",
            &[types::F64],
            Some(types::I64),
        )?;
        let socket_retain = declare_foreign(module, "__kome_socket_retain", &[types::I64], None)?;
        let socket_release = declare_foreign(module, "__kome_socket_release", &[types::I64], None)?;
        let struct_alloc = declare_foreign(
            module,
            "__kome_struct_alloc",
            &[types::I64],
            Some(types::I64),
        )?;
        let struct_retain = declare_foreign(module, "__kome_struct_retain", &[types::I64], None)?;
        let struct_release = declare_foreign(
            module,
            "__kome_struct_release",
            &[types::I64],
            Some(types::I8),
        )?;
        let struct_dealloc = declare_foreign(
            module,
            "__kome_struct_dealloc",
            &[types::I64, types::I64],
            None,
        )?;
        let closure_alloc = declare_foreign(
            module,
            "__kome_closure_alloc",
            &[types::I64, types::I64, types::I64],
            Some(types::I64),
        )?;
        let closure_retain = declare_foreign(module, "__kome_closure_retain", &[types::I64], None)?;
        let closure_release =
            declare_foreign(module, "__kome_closure_release", &[types::I64], None)?;
        let closure_code = declare_foreign(
            module,
            "__kome_closure_code",
            &[types::I64],
            Some(types::I64),
        )?;
        let closure_environment = declare_foreign(
            module,
            "__kome_closure_environment",
            &[types::I64],
            Some(types::I64),
        )?;
        let task_spawn = declare_foreign(
            module,
            "__kome_task_spawn",
            &[
                module.target_config().pointer_type(),
                module.target_config().pointer_type(),
                types::I64,
            ],
            Some(types::I64),
        )?;
        let task_cancel = declare_foreign(module, "__kome_task_cancel", &[types::I64], None)?;
        let task_is_cancelled = declare_foreign(
            module,
            "__kome_task_is_cancelled",
            &[types::I64],
            Some(types::I8),
        )?;
        let task_wait =
            declare_foreign(module, "__kome_task_wait", &[types::I64], Some(types::I8))?;
        let task_result = declare_foreign(
            module,
            "__kome_task_result",
            &[types::I64],
            Some(types::I64),
        )?;
        let task_retain = declare_foreign(module, "__kome_task_retain", &[types::I64], None)?;
        let task_release = declare_foreign(
            module,
            "__kome_task_release",
            &[types::I64],
            Some(types::I8),
        )?;
        let task_dealloc = declare_foreign(module, "__kome_task_dealloc", &[types::I64], None)?;
        let task_race = declare_foreign(
            module,
            "__kome_task_race",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;
        let task_wait_timeout = declare_foreign(
            module,
            "__kome_task_wait_timeout",
            &[types::I64, types::I64],
            Some(types::I8),
        )?;
        let task_all = declare_foreign(
            module,
            "__kome_task_all",
            &[types::I64, types::I64],
            Some(types::I8),
        )?;
        let task_require_completed = declare_foreign(
            module,
            "__kome_task_require_completed",
            &[types::I8, types::I8],
            None,
        )?;
        let task_require_race_winner = declare_foreign(
            module,
            "__kome_task_require_race_winner",
            &[types::I64],
            Some(types::I64),
        )?;
        let list_alloc =
            declare_foreign(module, "__kome_list_alloc", &[types::I64], Some(types::I64))?;
        let list_retain = declare_foreign(module, "__kome_list_retain", &[types::I64], None)?;
        let list_release = declare_foreign(
            module,
            "__kome_list_release",
            &[types::I64],
            Some(types::I8),
        )?;
        let list_dealloc = declare_foreign(module, "__kome_list_dealloc", &[types::I64], None)?;
        let list_len = declare_foreign(module, "__kome_list_len", &[types::I64], Some(types::I64))?;
        let list_require_index = declare_foreign(
            module,
            "__kome_list_require_index",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        Ok(Self {
            native_call,
            optional_require,
            number_parse,
            number_retain,
            number_release,
            number_add,
            number_sub,
            number_mul,
            number_div,
            number_compare,
            number_to_i64,
            number_to_string,
            string_create,
            string_retain,
            string_release,
            string_concat,
            string_compare,
            boolean_to_string,
            signed_integer_to_string,
            unsigned_integer_to_string,
            f32_to_string,
            f64_to_string,
            socket_retain,
            socket_release,
            struct_alloc,
            struct_retain,
            struct_release,
            struct_dealloc,
            closure_alloc,
            closure_retain,
            closure_release,
            closure_code,
            closure_environment,
            task_spawn,
            task_cancel,
            task_is_cancelled,
            task_wait,
            task_result,
            task_retain,
            task_release,
            task_dealloc,
            task_race,
            task_wait_timeout,
            task_all,
            task_require_completed,
            task_require_race_winner,
            list_alloc,
            list_retain,
            list_release,
            list_dealloc,
            list_len,
            list_require_index,
        })
    }
}

fn declare_foreign<M: Module>(
    module: &mut M,
    name: &str,
    params: &[types::Type],
    ret: Option<types::Type>,
) -> CodegenResult<FuncId> {
    let mut signature = module.make_signature();

    signature
        .params
        .extend(params.iter().map(|ty| AbiParam::new(*ty)));

    if let Some(ret) = ret {
        signature.returns.push(AbiParam::new(ret));
    }

    let func_id = module
        .declare_function(name, Linkage::Import, &signature)
        .map_err(|error| CodegenError::new(error.to_string(), None))?;

    Ok(func_id)
}

/// Interns the NUL-terminated native-symbol names used by `@native` calls.
#[derive(Default)]
struct NativeSymbolPool {
    ids: HashMap<String, DataId>,
}

impl NativeSymbolPool {
    fn intern<M: Module>(&mut self, module: &mut M, symbol: &str) -> CodegenResult<DataId> {
        if let Some(id) = self.ids.get(symbol) {
            return Ok(*id);
        }

        let mut description = DataDescription::new();
        let mut bytes = symbol.as_bytes().to_vec();
        bytes.push(0);
        description.define(bytes.into_boxed_slice());
        description.set_align(8);

        let name = format!("kome_native_symbol_{}", self.ids.len());
        let id = module
            .declare_data(&name, Linkage::Export, false, false)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        module
            .define_data(id, &description)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;
        self.ids.insert(symbol.to_owned(), id);

        Ok(id)
    }
}

fn build_signature<M: Module>(
    module: &M,
    signature: &FunctionSignature,
) -> CodegenResult<ir::Signature> {
    let mut cranelift_signature = module.make_signature();

    for param in &signature.params {
        let ty = param
            .cranelift()
            .ok_or_else(|| CodegenError::new("parameters cannot have type Void", None))?;

        cranelift_signature.params.push(AbiParam::new(ty));
    }

    if let Some(ret) = signature.ret.cranelift() {
        cranelift_signature.returns.push(AbiParam::new(ret));
    }

    Ok(cranelift_signature)
}

fn analyze_signature(
    function: &FunctionDeclaration,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    task_types: &RefCell<Vec<KomeType>>,
    list_types: &RefCell<Vec<KomeType>>,
    optional_types: &RefCell<Vec<KomeType>>,
    closure_types: &RefCell<Vec<FunctionSignature>>,
    self_type: Option<KomeType>,
) -> CodegenResult<FunctionSignature> {
    let mut params = Vec::with_capacity(function.params.len());
    let mut param_names = Vec::with_capacity(function.params.len());
    let mut defaults = Vec::with_capacity(function.params.len());

    for pattern in &function.params {
        let kome_ast::patterns::Pattern::Ident(identifier) = pattern else {
            return Err(CodegenError::at(
                format!(
                    "function `{}` contains an unsupported parameter pattern",
                    function.name
                ),
                pattern.span(),
            ));
        };

        let param_type = if identifier.name == "self" && identifier.type_annotation.is_none() {
            self_type.ok_or_else(|| {
                CodegenError::at(
                    "`self` is only valid in a type implementation",
                    identifier.span,
                )
            })?
        } else {
            let Some(annotation) = &identifier.type_annotation else {
                return Err(CodegenError::at(
                    format!(
                        "parameter `{}` of function `{}` requires a type annotation",
                        identifier.name, function.name
                    ),
                    identifier.span,
                ));
            };
            type_from_annotation(
                annotation,
                runtime_types,
                struct_ids,
                task_types,
                list_types,
                optional_types,
                closure_types,
            )?
        };

        if param_type == KomeType::Void {
            return Err(CodegenError::at(
                "parameters cannot have type Void",
                identifier.span,
            ));
        }

        params.push(param_type);
        param_names.push(identifier.name.clone());
        defaults.push(identifier.default.as_deref().cloned());
    }

    let ret = match &function.return_type {
        Some(annotation) => type_from_annotation(
            annotation,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
        )?,
        None => KomeType::Void,
    };

    Ok(FunctionSignature {
        param_names,
        params,
        defaults,
        ret,
    })
}

fn type_from_annotation(
    annotation: &kome_ast::types::Type,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    task_types: &RefCell<Vec<KomeType>>,
    list_types: &RefCell<Vec<KomeType>>,
    optional_types: &RefCell<Vec<KomeType>>,
    closure_types: &RefCell<Vec<FunctionSignature>>,
) -> CodegenResult<KomeType> {
    if let kome_ast::types::Type::Pointer(_) = annotation {
        return Ok(KomeType::Pointer);
    }
    if let kome_ast::types::Type::Optional(optional) = annotation {
        let inner = type_from_annotation(
            &optional.inner,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
        )?;
        if matches!(inner, KomeType::Void | KomeType::Null) {
            return Err(CodegenError::at(
                "optional values require a non-Void, non-Null inner type",
                optional.span,
            ));
        }
        let mut optional_types = optional_types.borrow_mut();
        let id = optional_types
            .iter()
            .position(|existing| *existing == inner)
            .unwrap_or_else(|| {
                optional_types.push(inner);
                optional_types.len() - 1
            });
        return Ok(KomeType::Optional(id));
    }
    if let kome_ast::types::Type::List(list) = annotation {
        let element = type_from_annotation(
            &list.element,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
        )?;
        let mut list_types = list_types.borrow_mut();
        let id = list_types
            .iter()
            .position(|existing| *existing == element)
            .unwrap_or_else(|| {
                list_types.push(element);
                list_types.len() - 1
            });
        return Ok(KomeType::List(id));
    }
    if let kome_ast::types::Type::Named(named) = annotation {
        if named.name == "Void" && named.type_arguments.is_empty() {
            return Ok(KomeType::Void);
        }
        if named.name == "Task" {
            if named.type_arguments.len() != 1 {
                return Err(CodegenError::at(
                    format!(
                        "type `Task` expects 1 type argument, but received {}",
                        named.type_arguments.len()
                    ),
                    named.span,
                ));
            }
            let result = type_from_annotation(
                &named.type_arguments[0],
                runtime_types,
                struct_ids,
                task_types,
                list_types,
                optional_types,
                closure_types,
            )?;
            let mut task_types = task_types.borrow_mut();
            let id = task_types
                .iter()
                .position(|existing| *existing == result)
                .unwrap_or_else(|| {
                    task_types.push(result);
                    task_types.len() - 1
                });
            return Ok(KomeType::Task(id));
        }
        return runtime_types
            .get(&named.name)
            .copied()
            .or_else(|| struct_ids.get(&named.name).copied().map(KomeType::Struct))
            .ok_or_else(|| {
                CodegenError::at(
                    format!("type `{}` has no runtime representation", named.name),
                    named.span,
                )
            });
    }

    if let kome_ast::types::Type::Function(function) = annotation {
        let mut params = Vec::with_capacity(function.params.len());
        let mut param_names = Vec::with_capacity(function.params.len());
        let mut defaults = Vec::with_capacity(function.params.len());
        for parameter in &function.params {
            params.push(type_from_annotation(
                &parameter.type_,
                runtime_types,
                struct_ids,
                task_types,
                list_types,
                optional_types,
                closure_types,
            )?);
            param_names.push(parameter.name.clone());
            defaults.push(parameter.default.clone());
        }
        let ret = type_from_annotation(
            &function.return_type,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
        )?;
        let signature = FunctionSignature {
            param_names,
            params,
            defaults,
            ret,
        };
        let mut closure_types = closure_types.borrow_mut();
        let id = closure_types
            .iter()
            .position(|existing| {
                existing.params == signature.params && existing.ret == signature.ret
            })
            .unwrap_or_else(|| {
                closure_types.push(signature);
                closure_types.len() - 1
            });
        return Ok(KomeType::Closure(id));
    }

    KomeType::from_annotation(annotation)
}

#[allow(clippy::too_many_arguments)]
fn infer_global_type(
    name: &str,
    globals: &HashMap<String, GlobalInfo>,
    functions: &HashMap<String, FunctionKind>,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    structs: &mut Vec<StructInfo>,
    implementations: &[TypeImplementation],
    task_types: &RefCell<Vec<KomeType>>,
    list_types: &RefCell<Vec<KomeType>>,
    optional_types: &RefCell<Vec<KomeType>>,
    closure_types: &RefCell<Vec<FunctionSignature>>,
    cache: &mut HashMap<String, KomeType>,
    visiting: &mut Vec<String>,
) -> CodegenResult<KomeType> {
    if let Some(type_) = cache.get(name) {
        return Ok(*type_);
    }
    if visiting.iter().any(|current| current == name) {
        return Err(CodegenError::new(
            format!("cyclic global initializer for `{name}`"),
            None,
        ));
    }
    let global = globals
        .get(name)
        .ok_or_else(|| CodegenError::new(format!("global `{name}` was not found"), None))?;
    if let Some(annotation) = &global.binding.type_annotation {
        let type_ = type_from_annotation(
            annotation,
            runtime_types,
            struct_ids,
            task_types,
            list_types,
            optional_types,
            closure_types,
        )?;
        cache.insert(name.to_owned(), type_);
        return Ok(type_);
    }
    let initializer = global.binding.init.as_ref().ok_or_else(|| {
        CodegenError::at(
            format!("global `{name}` requires an initializer or type annotation"),
            global.binding.span,
        )
    })?;
    visiting.push(name.to_owned());
    let result = infer_codegen_expression_type(
        initializer,
        globals,
        functions,
        runtime_types,
        struct_ids,
        structs,
        implementations,
        task_types,
        list_types,
        optional_types,
        closure_types,
        cache,
        visiting,
    );
    visiting.pop();
    let type_ = result.map_err(|_| {
        CodegenError::at(
            format!(
                "cannot infer the code generation type of global `{name}`; add a type annotation"
            ),
            initializer.span(),
        )
    })?;
    cache.insert(name.to_owned(), type_);
    Ok(type_)
}

#[allow(clippy::too_many_arguments)]
fn infer_codegen_expression_type(
    expression: &Expression,
    globals: &HashMap<String, GlobalInfo>,
    functions: &HashMap<String, FunctionKind>,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    structs: &mut Vec<StructInfo>,
    implementations: &[TypeImplementation],
    task_types: &RefCell<Vec<KomeType>>,
    list_types: &RefCell<Vec<KomeType>>,
    optional_types: &RefCell<Vec<KomeType>>,
    closure_types: &RefCell<Vec<FunctionSignature>>,
    cache: &mut HashMap<String, KomeType>,
    visiting: &mut Vec<String>,
) -> CodegenResult<KomeType> {
    let mut recurse = |expression: &Expression,
                       cache: &mut HashMap<String, KomeType>,
                       visiting: &mut Vec<String>| {
        infer_codegen_expression_type(
            expression,
            globals,
            functions,
            runtime_types,
            struct_ids,
            structs,
            implementations,
            task_types,
            list_types,
            optional_types,
            closure_types,
            cache,
            visiting,
        )
    };
    match expression {
        Expression::Literal(literal) => Ok(match literal.kind {
            LiteralKind::String(_) => KomeType::String,
            LiteralKind::Number(_) | LiteralKind::Percent(_) => KomeType::Number,
            LiteralKind::Boolean(_) => KomeType::Boolean,
            LiteralKind::Null => KomeType::Null,
        }),
        Expression::Ident(identifier) => infer_global_type(
            &identifier.name,
            globals,
            functions,
            runtime_types,
            struct_ids,
            structs,
            implementations,
            task_types,
            list_types,
            optional_types,
            closure_types,
            cache,
            visiting,
        ),
        Expression::Unary(_) => Ok(KomeType::Boolean),
        Expression::Unwrap(unwrap) => match recurse(&unwrap.argument, cache, visiting)? {
            KomeType::Optional(id) => Ok(optional_types.borrow()[id]),
            _ => Err(CodegenError::at(
                "postfix `!` expects an optional value",
                unwrap.span,
            )),
        },
        Expression::Task(task) => {
            let result = recurse(&task.argument, cache, visiting)?;
            let mut types = task_types.borrow_mut();
            let id = types
                .iter()
                .position(|item| *item == result)
                .unwrap_or_else(|| {
                    types.push(result);
                    types.len() - 1
                });
            Ok(KomeType::Task(id))
        }
        Expression::Wait(wait) => match recurse(&wait.argument, cache, visiting)? {
            KomeType::Task(id) => Ok(task_types.borrow()[id]),
            _ => Err(CodegenError::at("wait expects Task<T>", wait.span)),
        },
        Expression::Cancel(_) => Ok(KomeType::Void),
        Expression::Binary(binary) => match binary.op {
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Lte
            | BinaryOp::Gt
            | BinaryOp::Gte
            | BinaryOp::And
            | BinaryOp::Or => Ok(KomeType::Boolean),
            _ => recurse(&binary.left, cache, visiting),
        },
        Expression::Call(call) => match call.callee.as_ref() {
            Expression::Ident(identifier) => functions
                .get(&identifier.name)
                .map(|function| function.signature().ret)
                .ok_or_else(|| CodegenError::at("callee type is unknown", identifier.span)),
            Expression::Member(member) => {
                let target = if let Expression::Ident(identifier) = member.object.as_ref() {
                    runtime_types
                        .get(&identifier.name)
                        .copied()
                        .or_else(|| {
                            struct_ids
                                .get(&identifier.name)
                                .copied()
                                .map(KomeType::Struct)
                        })
                        .or_else(|| recurse(&member.object, cache, visiting).ok())
                } else {
                    Some(recurse(&member.object, cache, visiting)?)
                }
                .ok_or_else(|| CodegenError::at("method receiver type is unknown", member.span))?;
                implementations
                    .iter()
                    .find(|implementation| implementation.target == target)
                    .and_then(|implementation| implementation.methods.get(&member.property))
                    .map(|method| method.signature.ret)
                    .ok_or_else(|| CodegenError::at("method type is unknown", member.span))
            }
            Expression::Group(group) => recurse(&group.expression, cache, visiting),
            _ => Err(CodegenError::at(
                "callee type is unknown",
                call.callee.span(),
            )),
        },
        Expression::Member(member) => {
            let target = recurse(&member.object, cache, visiting)?;
            match target {
                KomeType::Struct(id) => structs[id]
                    .fields
                    .iter()
                    .find(|field| field.name == member.property)
                    .map(|field| field.kome_type)
                    .ok_or_else(|| CodegenError::at("field type is unknown", member.span)),
                _ => Err(CodegenError::at("member type is unknown", member.span)),
            }
        }
        Expression::Index(index) => match recurse(&index.object, cache, visiting)? {
            KomeType::List(id) => Ok(list_types.borrow()[id]),
            _ => Err(CodegenError::at("index result type is unknown", index.span)),
        },
        Expression::Assign(assign) => recurse(&assign.value, cache, visiting),
        Expression::Group(group) => recurse(&group.expression, cache, visiting),
        Expression::Block(block) => block
            .tail
            .as_deref()
            .map(|tail| recurse(tail, cache, visiting))
            .unwrap_or(Ok(KomeType::Void)),
        Expression::List(list) => {
            let element = list
                .elems
                .iter()
                .flatten()
                .next()
                .ok_or_else(|| CodegenError::at("empty list type is unknown", list.span))?;
            let element = recurse(element, cache, visiting)?;
            let mut types = list_types.borrow_mut();
            let id = types
                .iter()
                .position(|item| *item == element)
                .unwrap_or_else(|| {
                    types.push(element);
                    types.len() - 1
                });
            Ok(KomeType::List(id))
        }
        Expression::Struct(struct_) => struct_ids
            .get(&struct_.name)
            .copied()
            .map(KomeType::Struct)
            .ok_or_else(|| CodegenError::at("struct type is unknown", struct_.span)),
        Expression::Object(object) => {
            let mut fields = Vec::with_capacity(object.props.len());
            for property in &object.props {
                let ObjectProperty::KeyValue(property) = property;
                let name = static_property_name(&property.key).ok_or_else(|| {
                    CodegenError::at(
                        "computed object keys must be string or number literals",
                        property.span,
                    )
                })?;
                let type_ = recurse(&property.value, cache, visiting)?;
                fields.push((name, type_));
            }
            intern_anonymous_struct(structs, fields, object.span)
        }
        Expression::Template(_) => Ok(KomeType::String),
        Expression::Is(is) => recurse(&is.body, cache, visiting),
        Expression::Component(_) => Ok(KomeType::Null),
        Expression::Closure(_) | Expression::DotIdent(_) => Err(CodegenError::at(
            "expression type requires context",
            expression.span(),
        )),
    }
}

fn static_property_name(key: &PropertyKey) -> Option<String> {
    match key {
        PropertyKey::Ident { name, .. } => Some(name.clone()),
        PropertyKey::String { value, .. } => Some(value.clone()),
        PropertyKey::Number { value, .. } => Some(value.clone()),
        PropertyKey::Computed { expression, .. } => match expression.as_ref() {
            Expression::Literal(LiteralExpression {
                kind: LiteralKind::String(value),
                ..
            }) => Some(value.clone()),
            Expression::Literal(LiteralExpression {
                kind: LiteralKind::Number(value),
                ..
            }) => Some(value.0.clone()),
            _ => None,
        },
    }
}

fn static_index_name(expression: &Expression) -> Option<String> {
    match expression {
        Expression::Literal(LiteralExpression {
            kind: LiteralKind::String(value),
            ..
        }) => Some(value.clone()),
        Expression::Literal(LiteralExpression {
            kind: LiteralKind::Number(value),
            ..
        }) => Some(value.0.clone()),
        Expression::Group(group) => static_index_name(&group.expression),
        _ => None,
    }
}

fn intern_anonymous_struct(
    structs: &mut Vec<StructInfo>,
    fields: Vec<(String, KomeType)>,
    span: Span,
) -> CodegenResult<KomeType> {
    let mut seen = std::collections::HashSet::new();
    for (name, _) in &fields {
        if !seen.insert(name.clone()) {
            return Err(CodegenError::at(
                format!("duplicate object property `{name}`"),
                span,
            ));
        }
    }
    if let Some(id) = structs.iter().position(|layout| {
        layout.anonymous
            && layout.fields.len() == fields.len()
            && layout
                .fields
                .iter()
                .zip(&fields)
                .all(|(existing, (name, type_))| {
                    existing.name == *name && existing.kome_type == *type_
                })
    }) {
        return Ok(KomeType::Struct(id));
    }
    let id = structs.len();
    let layout_fields = fields
        .into_iter()
        .enumerate()
        .map(|(index, (name, kome_type))| StructFieldInfo {
            name,
            kome_type,
            offset: (index * 8) as i32,
        })
        .collect::<Vec<_>>();
    structs.push(StructInfo {
        name: format!("<object#{id}>"),
        size: (layout_fields.len() * 8) as u32,
        fields: layout_fields,
        anonymous: true,
    });
    Ok(KomeType::Struct(id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueOwnership {
    Borrowed,
    BorrowedMovable { scope: usize, variable: Variable },
    Owned,
}

/// The result of evaluating an expression: its native value and type.
///
/// The value is `None` exactly when the type is `Void`.
#[derive(Debug, Clone, Copy)]
struct TypedValue {
    value: Option<ir::Value>,
    kome_type: KomeType,
    ownership: ValueOwnership,
}

impl TypedValue {
    fn some(value: ir::Value, kome_type: KomeType) -> Self {
        Self {
            value: Some(value),
            kome_type,
            ownership: ValueOwnership::Owned,
        }
    }

    fn borrowed(value: ir::Value, kome_type: KomeType) -> Self {
        Self {
            value: Some(value),
            kome_type,
            ownership: ValueOwnership::Borrowed,
        }
    }

    fn void() -> Self {
        Self {
            value: None,
            kome_type: KomeType::Void,
            ownership: ValueOwnership::Borrowed,
        }
    }

    fn expect_value(self, span: Span) -> CodegenResult<ir::Value> {
        self.value.ok_or_else(|| {
            CodegenError::at("expression of type Void cannot be used as a value", span)
        })
    }
}

/// How a call's callee resolves, decided before arguments are evaluated so
/// that mutable translation can proceed without outstanding borrows.
enum CalleePlan {
    User,

    External,

    Native { symbol: String },
}

struct FunctionTranslator<'b, 'c, M: Module> {
    builder: FunctionBuilder<'c>,
    module: &'b mut M,
    info: &'b ModuleInfo,
    func_ids: &'b HashMap<String, FuncId>,
    task_entry_ids: &'b HashMap<String, FuncId>,
    closure_drop_ids: &'b HashMap<usize, FuncId>,
    global_storage: &'b HashMap<String, GlobalStorage>,
    foreign: &'b ForeignFunctions,
    native_symbols: &'b mut NativeSymbolPool,
    scopes: Vec<HashMap<String, ScopedVariable>>,
    remaining_reads: HashMap<usize, usize>,
    next_binding_id: usize,
    return_type: KomeType,
    terminated: bool,
    loops: Vec<LoopContext>,
    allow_managed_moves: bool,
    evaluating_globals: Vec<String>,
    closures: Vec<(String, usize, kome_ast::expressions::ClosureExpression)>,
    local_functions: Vec<(String, usize, FunctionDeclaration)>,
    local_call_stack: Vec<String>,
    inline_returns: Vec<InlineReturnContext>,
}

#[derive(Debug, Clone, Copy)]
struct LoopContext {
    header: ir::Block,
    exit: ir::Block,
    scope_depth: usize,
}

#[derive(Debug, Clone, Copy)]
struct InlineReturnContext {
    target: ir::Block,
    return_type: KomeType,
    scope_depth: usize,
}

#[derive(Debug, Clone, Copy)]
struct ScopedVariable {
    variable: Variable,
    kome_type: KomeType,
    owns_value: bool,
    binding_id: usize,
}

struct ReadCounter {
    scopes: Vec<HashMap<String, usize>>,
    reads: HashMap<usize, usize>,
    next_binding_id: usize,
}

fn count_variable_reads(
    declaration: &FunctionDeclaration,
    block: &BlockStatement,
) -> HashMap<usize, usize> {
    let mut counter = ReadCounter {
        scopes: vec![HashMap::new()],
        reads: HashMap::new(),
        next_binding_id: 0,
    };

    for pattern in &declaration.params {
        let kome_ast::patterns::Pattern::Ident(identifier) = pattern else {
            continue;
        };

        counter.declare(&identifier.name);
    }

    counter.visit_block(block);

    counter.reads
}

fn contains_control_flow(block: &BlockStatement) -> bool {
    block.statements.iter().any(statement_contains_control_flow)
}

fn contains_closure(block: &BlockStatement) -> bool {
    block.statements.iter().any(statement_contains_closure)
}

fn statement_contains_closure(statement: &Statement) -> bool {
    match statement {
        Statement::Expression(value) => expression_contains_closure(&value.expression),
        Statement::Let(value) => value.init.as_ref().is_some_and(expression_contains_closure),
        Statement::Return(value) => value
            .argument
            .as_ref()
            .is_some_and(expression_contains_closure),
        Statement::Block(value) => contains_closure(value),
        Statement::If(value) => {
            expression_contains_closure(&value.test)
                || statement_contains_closure(&value.consequent)
                || value
                    .alternative
                    .as_deref()
                    .is_some_and(statement_contains_closure)
        }
        Statement::While(value) => {
            expression_contains_closure(&value.test) || statement_contains_closure(&value.body)
        }
        Statement::ForIn(value) => {
            expression_contains_closure(&value.right) || statement_contains_closure(&value.body)
        }
        Statement::Is(value) => {
            value
                .value
                .as_ref()
                .is_some_and(expression_contains_closure)
                || statement_contains_closure(&value.body)
        }
        Statement::Declaration(Declaration::Let(value))
        | Statement::Declaration(Declaration::Constant(value)) => {
            value.init.as_ref().is_some_and(expression_contains_closure)
        }
        _ => false,
    }
}

fn expression_contains_closure(expression: &Expression) -> bool {
    match expression {
        Expression::Closure(_) => true,
        Expression::Unary(value) => expression_contains_closure(&value.argument),
        Expression::Unwrap(value) => expression_contains_closure(&value.argument),
        Expression::Task(value) => expression_contains_closure(&value.argument),
        Expression::Wait(value) => expression_contains_closure(&value.argument),
        Expression::Cancel(value) => expression_contains_closure(&value.argument),
        Expression::Binary(value) => {
            expression_contains_closure(&value.left) || expression_contains_closure(&value.right)
        }
        Expression::Call(value) => {
            expression_contains_closure(&value.callee)
                || value.args.iter().any(|argument| match argument {
                    CallArg::Positional(value) => expression_contains_closure(value),
                    CallArg::Named { value, .. } => expression_contains_closure(value),
                })
        }
        Expression::Member(value) => expression_contains_closure(&value.object),
        Expression::Index(value) => {
            expression_contains_closure(&value.object) || expression_contains_closure(&value.index)
        }
        Expression::Assign(value) => {
            expression_contains_closure(&value.target) || expression_contains_closure(&value.value)
        }
        Expression::Group(value) => expression_contains_closure(&value.expression),
        Expression::Block(value) => {
            value.statements.iter().any(statement_contains_closure)
                || value
                    .tail
                    .as_deref()
                    .is_some_and(expression_contains_closure)
        }
        Expression::List(value) => value
            .elems
            .iter()
            .flatten()
            .any(expression_contains_closure),
        Expression::Object(value) => value.props.iter().any(|property| match property {
            ObjectProperty::KeyValue(value) => expression_contains_closure(&value.value),
        }),
        Expression::Struct(value) => value
            .fields
            .iter()
            .any(|field| expression_contains_closure(&field.value)),
        Expression::Template(value) => value.parts.iter().any(|part| match part {
            TemplatePart::String { .. } => false,
            TemplatePart::Expression { expression, .. } => expression_contains_closure(expression),
        }),
        Expression::Is(value) => {
            expression_contains_closure(&value.value) || expression_contains_closure(&value.body)
        }
        Expression::Component(value) => {
            value.args.iter().any(|argument| match argument {
                CallArg::Positional(value) => expression_contains_closure(value),
                CallArg::Named { value, .. } => expression_contains_closure(value),
            }) || value.children.iter().any(expression_contains_closure)
        }
        Expression::Literal(_) | Expression::Ident(_) | Expression::DotIdent(_) => false,
    }
}

fn statement_contains_control_flow(statement: &Statement) -> bool {
    match statement {
        Statement::If(_) | Statement::While(_) | Statement::ForIn(_) => true,
        Statement::Block(block) => contains_control_flow(block),
        _ => false,
    }
}

impl ReadCounter {
    fn declare(&mut self, name: &str) {
        let binding_id = self.next_binding_id;
        self.next_binding_id += 1;

        self.scopes
            .last_mut()
            .expect("read counter scope stack is never empty")
            .insert(name.to_owned(), binding_id);
    }

    fn read(&mut self, name: &str) {
        let Some(binding_id) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .copied()
        else {
            return;
        };

        *self.reads.entry(binding_id).or_default() += 1;
    }

    fn visit_block(&mut self, block: &BlockStatement) {
        self.scopes.push(HashMap::new());

        for statement in &block.statements {
            self.visit_statement(statement);
        }

        self.scopes.pop();
    }

    fn visit_statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Expression(statement) => {
                self.visit_expression(&statement.expression);
            }

            Statement::Return(statement) => {
                if let Some(argument) = &statement.argument {
                    self.visit_expression(argument);
                }
            }

            Statement::Let(binding) => {
                if let Some(init) = &binding.init {
                    self.visit_expression(init);
                }

                if let kome_ast::patterns::Pattern::Ident(identifier) = &binding.pattern {
                    self.declare(&identifier.name);
                }
            }

            Statement::Block(block) => {
                self.visit_block(block);
            }

            Statement::If(statement) => {
                self.visit_expression(&statement.test);
                self.visit_statement(&statement.consequent);
                if let Some(alternative) = &statement.alternative {
                    self.visit_statement(alternative);
                }
            }

            Statement::While(statement) => {
                self.visit_expression(&statement.test);
                self.visit_statement(&statement.body);
            }

            Statement::ForIn(statement) => {
                self.visit_expression(&statement.right);
                self.scopes.push(HashMap::new());
                if let kome_ast::patterns::Pattern::Ident(identifier) = &statement.pattern {
                    self.declare(&identifier.name);
                }
                self.visit_statement(&statement.body);
                self.scopes.pop();
            }

            Statement::Is(statement) => {
                if let Some(value) = &statement.value {
                    self.visit_expression(value);
                }
                if let IsPattern::Ident(pattern) = &statement.pattern {
                    self.scopes.push(HashMap::new());
                    self.declare(&pattern.name);
                    self.visit_statement(&statement.body);
                    self.scopes.pop();
                } else {
                    self.visit_statement(&statement.body);
                }
            }

            Statement::Declaration(Declaration::Let(binding))
            | Statement::Declaration(Declaration::Constant(binding)) => {
                if let Some(init) = &binding.init {
                    self.visit_expression(init);
                }
                if let kome_ast::patterns::Pattern::Ident(identifier) = &binding.pattern {
                    self.declare(&identifier.name);
                }
            }

            Statement::Declaration(_)
            | Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Empty(_) => {}
        }
    }

    fn visit_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::Ident(identifier) => {
                self.read(&identifier.name);
            }

            Expression::Group(group) => {
                self.visit_expression(&group.expression);
            }

            Expression::Unary(unary) => self.visit_expression(&unary.argument),

            Expression::Unwrap(unwrap) => self.visit_expression(&unwrap.argument),

            Expression::Task(task) => self.visit_expression(&task.argument),

            Expression::Wait(wait) => self.visit_expression(&wait.argument),

            Expression::Cancel(cancel) => self.visit_expression(&cancel.argument),

            Expression::Binary(binary) => {
                self.visit_expression(&binary.left);
                self.visit_expression(&binary.right);
            }

            Expression::Call(call) => {
                self.visit_expression(&call.callee);
                for argument in &call.args {
                    match argument {
                        CallArg::Positional(value) => {
                            self.visit_expression(value);
                        }

                        CallArg::Named { value, .. } => {
                            self.visit_expression(value);
                        }
                    }
                }
            }

            Expression::Assign(assignment) => {
                if !matches!(assignment.target.as_ref(), Expression::Ident(_)) {
                    self.visit_expression(&assignment.target);
                }
                self.visit_expression(&assignment.value);
            }

            Expression::Member(member) => self.visit_expression(&member.object),

            Expression::Index(index) => {
                self.visit_expression(&index.object);
                self.visit_expression(&index.index);
            }

            Expression::Struct(struct_) => {
                for field in &struct_.fields {
                    self.visit_expression(&field.value);
                }
            }

            Expression::Block(block) => {
                self.scopes.push(HashMap::new());
                for statement in &block.statements {
                    self.visit_statement(statement);
                }
                if let Some(tail) = &block.tail {
                    self.visit_expression(tail);
                }
                self.scopes.pop();
            }

            Expression::List(list) => {
                for element in list.elems.iter().flatten() {
                    self.visit_expression(element);
                }
            }

            Expression::Object(object) => {
                for property in &object.props {
                    let kome_ast::expressions::ObjectProperty::KeyValue(property) = property;
                    if let PropertyKey::Computed { expression, .. } = &property.key {
                        self.visit_expression(expression);
                    }
                    self.visit_expression(&property.value);
                }
            }

            Expression::Template(template) => {
                for part in &template.parts {
                    if let kome_ast::expressions::TemplatePart::Expression { expression, .. } = part
                    {
                        self.visit_expression(expression);
                    }
                }
            }

            Expression::Closure(closure) => {
                self.scopes.push(HashMap::new());
                for parameter in &closure.params {
                    if let kome_ast::patterns::Pattern::Ident(identifier) = parameter {
                        self.declare(&identifier.name);
                    }
                }
                self.visit_expression(&closure.body);
                self.scopes.pop();
            }

            Expression::Is(is_expression) => {
                self.visit_expression(&is_expression.value);
                if let IsPattern::Ident(pattern) = &is_expression.pattern {
                    self.scopes.push(HashMap::new());
                    self.declare(&pattern.name);
                    self.visit_expression(&is_expression.body);
                    self.scopes.pop();
                } else {
                    self.visit_expression(&is_expression.body);
                }
            }

            Expression::Component(component) => {
                for argument in &component.args {
                    match argument {
                        CallArg::Positional(value) => self.visit_expression(value),
                        CallArg::Named { value, .. } => self.visit_expression(value),
                    }
                }
                for child in &component.children {
                    self.visit_expression(child);
                }
            }

            Expression::Literal(_) | Expression::DotIdent(_) => {}
        }
    }
}

impl<'b, 'c, M: Module> FunctionTranslator<'b, 'c, M> {
    fn translate_task_entry(
        mut self,
        function_key: &str,
        signature: &FunctionSignature,
    ) -> CodegenResult<()> {
        let entry = self.builder.create_block();
        self.builder.append_block_params_for_function_params(entry);
        self.builder.switch_to_block(entry);
        self.builder.seal_block(entry);
        let parameters = self.builder.block_params(entry).to_vec();
        let handle = parameters[0];
        let buffer = parameters[1];
        let execute = parameters[2];
        let run = self.builder.create_block();
        let cleanup = self.builder.create_block();
        self.builder.ins().brif(execute, run, &[], cleanup, &[]);

        self.builder.switch_to_block(run);
        self.builder.seal_block(run);
        let arguments = signature
            .params
            .iter()
            .enumerate()
            .map(|(index, kome_type)| {
                let slot = self.builder.ins().load(
                    types::I64,
                    MachMemFlags::new(),
                    buffer,
                    (index * 8) as i32,
                );
                self.task_slot_to_value(slot, *kome_type)
            })
            .collect::<Vec<_>>();
        let result = self.emit_user_call(function_key, &arguments, signature)?;
        for (argument, kome_type) in arguments.iter().zip(&signature.params) {
            if kome_type.is_managed() {
                self.release_managed(*argument, *kome_type);
            }
        }
        let result = if signature.ret == KomeType::Void {
            self.builder.ins().iconst(types::I64, 0)
        } else {
            result.expect_value(kome_ast::Span::new(0, 0))?
        };
        let is_cancelled = Module::declare_func_in_func(
            self.module,
            self.foreign.task_is_cancelled,
            self.builder.func,
        );
        let cancelled_call = self.builder.ins().call(is_cancelled, &[handle]);
        let cancelled = self.builder.inst_results(cancelled_call)[0];
        let discard = self.builder.create_block();
        let finish = self.builder.create_block();
        self.builder
            .ins()
            .brif(cancelled, discard, &[], finish, &[]);
        self.builder.switch_to_block(discard);
        self.builder.seal_block(discard);
        if signature.ret.is_managed() {
            self.release_managed(result, signature.ret);
        }
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().return_(&[zero]);
        self.builder.switch_to_block(finish);
        self.builder.seal_block(finish);
        let result = if signature.ret == KomeType::Void {
            result
        } else {
            self.value_to_task_slot(result, signature.ret)
        };
        self.builder.ins().return_(&[result]);

        self.builder.switch_to_block(cleanup);
        self.builder.seal_block(cleanup);
        for (index, kome_type) in signature.params.iter().enumerate() {
            if kome_type.is_managed() {
                let slot = self.builder.ins().load(
                    types::I64,
                    MachMemFlags::new(),
                    buffer,
                    (index * 8) as i32,
                );
                let value = self.task_slot_to_value(slot, *kome_type);
                self.release_managed(value, *kome_type);
            }
        }
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.ins().return_(&[zero]);
        self.builder.finalize(self.module.target_config());
        Ok(())
    }

    fn translate_function(
        mut self,
        declaration: &FunctionDeclaration,
        signature: &FunctionSignature,
        is_drop_function: bool,
    ) -> CodegenResult<()> {
        let entry_block = self.builder.create_block();

        self.builder
            .append_block_params_for_function_params(entry_block);

        self.builder.switch_to_block(entry_block);
        self.builder.seal_block(entry_block);

        let body = declaration.body.as_ref().ok_or_else(|| {
            CodegenError::at(
                format!("function `{}` has no body", declaration.name),
                declaration.span,
            )
        })?;

        self.remaining_reads = count_variable_reads(declaration, body);
        self.allow_managed_moves = !contains_control_flow(body) && !contains_closure(body);

        let parameters = self.builder.block_params(entry_block).to_vec();

        for ((pattern, value), param_type) in declaration
            .params
            .iter()
            .zip(parameters)
            .zip(&signature.params)
        {
            let kome_ast::patterns::Pattern::Ident(identifier) = pattern else {
                unreachable!("non-identifier parameter patterns are rejected during analysis");
            };

            if is_drop_function && identifier.name == "self" {
                self.declare_nonowning_variable(&identifier.name, value, *param_type)?;
            } else {
                self.declare_variable(
                    &identifier.name,
                    value,
                    *param_type,
                    ValueOwnership::Borrowed,
                )?;
            }
        }

        self.translate_block(body)?;

        if !self.terminated {
            self.release_owned_managed();

            match self.return_type {
                KomeType::Void => {
                    self.builder.ins().return_(&[]);
                }

                return_type => {
                    let zero = self.zero_value(return_type)?;
                    self.builder.ins().return_(&[zero]);
                }
            }

            self.terminated = true;
        }

        self.builder.finalize(self.module.target_config());

        Ok(())
    }

    fn translate_block(&mut self, block: &BlockStatement) -> CodegenResult<()> {
        self.scopes.push(HashMap::new());
        let closure_depth = self.scopes.len();

        for statement in &block.statements {
            if self.terminated {
                break;
            }

            self.translate_statement(statement)?;
        }

        if !self.terminated {
            let scope = self.scopes.pop().expect("scope stack is never empty");

            for scoped in scope.values() {
                if scoped.kome_type.is_managed() && scoped.owns_value {
                    let value = self.builder.use_var(scoped.variable);
                    self.release_managed(value, scoped.kome_type);
                }
            }
        } else {
            self.scopes.pop();
        }
        self.closures.retain(|(_, depth, _)| *depth < closure_depth);
        self.local_functions
            .retain(|(_, depth, _)| *depth < closure_depth);

        Ok(())
    }

    fn translate_statement(&mut self, statement: &Statement) -> CodegenResult<()> {
        match statement {
            Statement::Empty(_) => {}

            Statement::Expression(statement) => {
                let value = self.evaluate(&statement.expression)?;
                self.release_owned_temporary(value, statement.expression.span())?;
            }

            Statement::Return(statement) => {
                if let Some(context) = self.inline_returns.last().copied() {
                    let value = match &statement.argument {
                        Some(expression) => {
                            let typed =
                                self.evaluate_with_expected(expression, Some(context.return_type))?;
                            if typed.kome_type != context.return_type {
                                return Err(CodegenError::at(
                                    format!(
                                        "`return` expects {}, but the expression has type {}",
                                        self.info.type_name(context.return_type),
                                        self.info.type_name(typed.kome_type)
                                    ),
                                    expression.span(),
                                ));
                            }
                            Some(self.own_value(typed, expression.span())?)
                        }
                        None if context.return_type == KomeType::Void => None,
                        None => Some(self.zero_value(context.return_type)?),
                    };
                    self.release_owned_scopes_from(context.scope_depth);
                    if let Some(value) = value {
                        self.builder
                            .ins()
                            .jump(context.target, &[ir::BlockArg::Value(value)]);
                    } else {
                        self.builder.ins().jump(context.target, &[]);
                    }
                    self.terminated = true;
                    return Ok(());
                }
                let value = match &statement.argument {
                    Some(expression) => {
                        let typed =
                            self.evaluate_with_expected(expression, Some(self.return_type))?;

                        if typed.kome_type != self.return_type {
                            return Err(CodegenError::at(
                                format!(
                                    "`return` expects {}, but the expression has type {}",
                                    self.return_type.name(),
                                    typed.kome_type.name(),
                                ),
                                expression.span(),
                            ));
                        }

                        self.own_value(typed, expression.span())?
                    }

                    None => match self.return_type {
                        KomeType::Void => {
                            self.release_owned_managed();
                            self.builder.ins().return_(&[]);
                            self.terminated = true;
                            return Ok(());
                        }

                        return_type => self.zero_value(return_type)?,
                    },
                };

                self.release_owned_managed();
                self.builder.ins().return_(&[value]);
                self.terminated = true;
            }

            Statement::Let(binding) => {
                let kome_ast::patterns::Pattern::Ident(pattern) = &binding.pattern else {
                    return Err(CodegenError::at(
                        "destructuring `let` is not supported yet",
                        binding.pattern.span(),
                    ));
                };

                let annotated_type = binding
                    .type_annotation
                    .as_ref()
                    .map(|annotation| {
                        type_from_annotation(
                            annotation,
                            &self.info.runtime_types,
                            &self.info.struct_ids,
                            &self.info.task_types,
                            &self.info.list_types,
                            &self.info.optional_types,
                            &self.info.closure_types,
                        )
                    })
                    .transpose()?;

                let typed = match &binding.init {
                    Some(expression) => self.evaluate_with_expected(expression, annotated_type)?,

                    None => {
                        let annotated = annotated_type.ok_or_else(|| {
                            CodegenError::at(
                                "`let` requires an initializer or a type annotation",
                                binding.span,
                            )
                        })?;

                        let zero = self.zero_value(annotated)?;

                        TypedValue::some(zero, annotated)
                    }
                };

                if typed.kome_type == KomeType::Void {
                    return Err(CodegenError::at(
                        "cannot bind a value of type Void",
                        binding.span,
                    ));
                }

                if let Some(annotated_type) = annotated_type
                    && annotated_type != typed.kome_type
                {
                    return Err(CodegenError::at(
                        format!(
                            "`let` is annotated as {}, but the initializer has type {}",
                            annotated_type.name(),
                            typed.kome_type.name(),
                        ),
                        binding.span,
                    ));
                }

                let value = typed.expect_value(binding.span)?;

                self.declare_variable(&pattern.name, value, typed.kome_type, typed.ownership)?;
            }

            Statement::Block(block) => self.translate_block(block)?,

            Statement::If(statement) => self.translate_if(statement)?,

            Statement::While(statement) => self.translate_while(statement)?,

            Statement::ForIn(statement) => self.translate_for_in(statement)?,

            Statement::Break(statement) => self.translate_break(statement.span)?,

            Statement::Continue(statement) => self.translate_continue(statement.span)?,

            Statement::Is(statement) => self.translate_is(statement)?,

            Statement::Declaration(Declaration::Let(binding))
            | Statement::Declaration(Declaration::Constant(binding)) => {
                self.translate_statement(&Statement::Let(binding.clone()))?;
            }

            Statement::Declaration(_) => {
                return Err(unsupported_statement("nested declaration", statement));
            }
        }

        Ok(())
    }

    fn translate_if(&mut self, statement: &kome_ast::statements::IfStatement) -> CodegenResult<()> {
        let condition = self.evaluate(&statement.test)?;
        if condition.kome_type != KomeType::Boolean {
            return Err(CodegenError::at(
                format!(
                    "`if` condition expects bool, but found {}",
                    self.info.type_name(condition.kome_type)
                ),
                statement.test.span(),
            ));
        }
        let condition_value = condition.expect_value(statement.test.span())?;
        let consequent = self.builder.create_block();
        let alternative = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder
            .ins()
            .brif(condition_value, consequent, &[], alternative, &[]);

        let base_scopes = self.scopes.clone();

        self.builder.switch_to_block(consequent);
        self.builder.seal_block(consequent);
        self.scopes = base_scopes.clone();
        self.terminated = false;
        self.translate_statement(&statement.consequent)?;
        let consequent_reaches_done = !self.terminated;
        if consequent_reaches_done {
            self.builder.ins().jump(done, &[]);
        }

        self.builder.switch_to_block(alternative);
        self.builder.seal_block(alternative);
        self.scopes = base_scopes.clone();
        self.terminated = false;
        if let Some(statement) = &statement.alternative {
            self.translate_statement(statement)?;
        }
        let alternative_reaches_done = !self.terminated;
        if alternative_reaches_done {
            self.builder.ins().jump(done, &[]);
        }

        self.scopes = base_scopes;
        self.builder.seal_block(done);
        if consequent_reaches_done || alternative_reaches_done {
            self.builder.switch_to_block(done);
            self.terminated = false;
        } else {
            self.terminated = true;
        }
        Ok(())
    }

    fn translate_while(
        &mut self,
        statement: &kome_ast::statements::WhileStatement,
    ) -> CodegenResult<()> {
        let header = self.builder.create_block();
        let body = self.builder.create_block();
        let exit = self.builder.create_block();
        self.builder.ins().jump(header, &[]);

        self.builder.switch_to_block(header);
        let condition = self.evaluate(&statement.test)?;
        if condition.kome_type != KomeType::Boolean {
            return Err(CodegenError::at(
                format!(
                    "`while` condition expects bool, but found {}",
                    self.info.type_name(condition.kome_type)
                ),
                statement.test.span(),
            ));
        }
        let condition_value = condition.expect_value(statement.test.span())?;
        self.builder
            .ins()
            .brif(condition_value, body, &[], exit, &[]);

        let base_scopes = self.scopes.clone();
        self.builder.switch_to_block(body);
        self.builder.seal_block(body);
        self.loops.push(LoopContext {
            header,
            exit,
            scope_depth: base_scopes.len(),
        });
        self.scopes = base_scopes.clone();
        self.terminated = false;
        self.translate_statement(&statement.body)?;
        self.loops.pop();
        if !self.terminated {
            self.builder.ins().jump(header, &[]);
        }

        self.scopes = base_scopes;
        self.builder.seal_block(header);
        self.builder.seal_block(exit);
        self.builder.switch_to_block(exit);
        self.terminated = false;
        Ok(())
    }

    fn translate_break(&mut self, span: Span) -> CodegenResult<()> {
        let context = self
            .loops
            .last()
            .copied()
            .ok_or_else(|| CodegenError::at("`break` can only be used inside a loop", span))?;
        self.release_owned_scopes_from(context.scope_depth);
        self.builder.ins().jump(context.exit, &[]);
        self.terminated = true;
        Ok(())
    }

    fn translate_for_in(
        &mut self,
        statement: &kome_ast::statements::ForInStatement,
    ) -> CodegenResult<()> {
        let kome_ast::patterns::Pattern::Ident(pattern) = &statement.pattern else {
            return Err(CodegenError::at(
                "`for in` requires an identifier binding",
                statement.pattern.span(),
            ));
        };
        let iterable = self.evaluate(&statement.right)?;
        let KomeType::List(id) = iterable.kome_type else {
            return Err(CodegenError::at(
                format!(
                    "`for in` expects a List, but found {}",
                    self.info.type_name(iterable.kome_type)
                ),
                statement.right.span(),
            ));
        };
        let list = iterable.expect_value(statement.right.span())?;
        let len =
            Module::declare_func_in_func(self.module, self.foreign.list_len, self.builder.func);
        let len_call = self.builder.ins().call(len, &[list]);
        let length = self.builder.inst_results(len_call)[0];
        let index = self.builder.declare_var(types::I64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder.def_var(index, zero);

        let header = self.builder.create_block();
        let body = self.builder.create_block();
        let increment = self.builder.create_block();
        let exit = self.builder.create_block();
        self.builder.ins().jump(header, &[]);

        self.builder.switch_to_block(header);
        let current = self.builder.use_var(index);
        let more = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, current, length);
        self.builder.ins().brif(more, body, &[], exit, &[]);

        let base_scopes = self.scopes.clone();
        self.builder.switch_to_block(body);
        self.builder.seal_block(body);
        let offset = self.builder.ins().imul_imm_u(current, 8);
        let address = self.builder.ins().iadd(list, offset);
        let slot = self
            .builder
            .ins()
            .load(types::I64, MachMemFlags::new(), address, 0);
        let element_type = self.info.list_element(id);
        let element = self.task_slot_to_value(slot, element_type);
        if element_type.is_managed() {
            self.retain_managed(element, element_type);
        }
        self.scopes.push(HashMap::new());
        self.declare_variable(&pattern.name, element, element_type, ValueOwnership::Owned)?;
        self.loops.push(LoopContext {
            header: increment,
            exit,
            scope_depth: base_scopes.len(),
        });
        self.terminated = false;
        self.translate_statement(&statement.body)?;
        self.loops.pop();
        if !self.terminated {
            let scope = self.scopes.pop().expect("for binding scope exists");
            for scoped in scope.values() {
                if scoped.kome_type.is_managed() && scoped.owns_value {
                    let value = self.builder.use_var(scoped.variable);
                    self.release_managed(value, scoped.kome_type);
                }
            }
            self.builder.ins().jump(increment, &[]);
        } else {
            self.scopes.pop();
        }

        self.builder.switch_to_block(increment);
        self.builder.seal_block(increment);
        let current = self.builder.use_var(index);
        let next = self.builder.ins().iadd_imm_u(current, 1);
        self.builder.def_var(index, next);
        self.builder.ins().jump(header, &[]);

        self.scopes = base_scopes;
        self.builder.seal_block(header);
        self.builder.seal_block(exit);
        self.builder.switch_to_block(exit);
        self.terminated = false;
        self.release_owned_temporary(iterable, statement.right.span())?;
        Ok(())
    }

    fn translate_is(&mut self, statement: &kome_ast::statements::IsStatement) -> CodegenResult<()> {
        let Some(value_expression) = &statement.value else {
            return Err(CodegenError::at(
                "implicit `is` values are only available inside component event recipes",
                statement.span,
            ));
        };
        if let IsPattern::Ident(pattern) = &statement.pattern {
            let value = self.evaluate(value_expression)?;
            let raw = self.own_value(value, value_expression.span())?;
            self.scopes.push(HashMap::new());
            self.declare_variable(&pattern.name, raw, value.kome_type, ValueOwnership::Owned)?;
            self.translate_statement(&statement.body)?;
            if !self.terminated {
                let scope = self.scopes.pop().expect("is binding scope exists");
                for scoped in scope.values() {
                    if scoped.kome_type.is_managed() && scoped.owns_value {
                        let value = self.builder.use_var(scoped.variable);
                        self.release_managed(value, scoped.kome_type);
                    }
                }
            } else {
                self.scopes.pop();
            }
            return Ok(());
        }
        let pattern = match &statement.pattern {
            IsPattern::Literal(pattern) => Expression::literal(pattern.value.clone(), pattern.span),
            IsPattern::DotIdent(pattern) => {
                Expression::DotIdent(kome_ast::expressions::DotIdentifierExpression {
                    span: pattern.span,
                    name: pattern.name.clone(),
                })
            }
            IsPattern::Ident(_) => unreachable!(),
        };
        let comparison = BinaryExpression {
            span: statement.span,
            op: BinaryOp::Eq,
            left: Box::new(value_expression.clone()),
            right: Box::new(pattern),
        };
        let condition = self.evaluate_binary(&comparison)?;
        let condition = condition.expect_value(statement.span)?;
        let body = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.ins().brif(condition, body, &[], done, &[]);
        let base_scopes = self.scopes.clone();
        self.builder.switch_to_block(body);
        self.builder.seal_block(body);
        self.terminated = false;
        self.translate_statement(&statement.body)?;
        if !self.terminated {
            self.builder.ins().jump(done, &[]);
        }
        self.scopes = base_scopes;
        self.builder.seal_block(done);
        self.builder.switch_to_block(done);
        self.terminated = false;
        Ok(())
    }

    fn translate_continue(&mut self, span: Span) -> CodegenResult<()> {
        let context =
            self.loops.last().copied().ok_or_else(|| {
                CodegenError::at("`continue` can only be used inside a loop", span)
            })?;
        self.release_owned_scopes_from(context.scope_depth);
        self.builder.ins().jump(context.header, &[]);
        self.terminated = true;
        Ok(())
    }

    fn release_owned_scopes_from(&mut self, scope_depth: usize) {
        let values = self
            .scopes
            .iter()
            .skip(scope_depth)
            .flat_map(|scope| scope.values())
            .filter(|value| value.kome_type.is_managed() && value.owns_value)
            .map(|value| (self.builder.use_var(value.variable), value.kome_type))
            .collect::<Vec<_>>();
        for (value, kome_type) in values {
            self.release_managed(value, kome_type);
        }
    }

    fn declare_variable(
        &mut self,
        name: &str,
        value: ir::Value,
        kome_type: KomeType,
        ownership: ValueOwnership,
    ) -> CodegenResult<()> {
        let representation = kome_type.cranelift().ok_or_else(|| {
            CodegenError::new("internal error: Void cannot be stored in a variable", None)
        })?;

        let owns_value = if kome_type.is_managed() {
            self.take_managed_ownership(value, kome_type, ownership);
            true
        } else {
            false
        };

        let variable = self.builder.declare_var(representation);
        self.builder.def_var(variable, value);

        let binding_id = self.next_binding_id;
        self.next_binding_id += 1;

        let scope = self.scopes.last_mut().expect("scope stack is never empty");

        scope.insert(
            name.to_owned(),
            ScopedVariable {
                variable,
                kome_type,
                owns_value,
                binding_id,
            },
        );

        Ok(())
    }

    fn declare_nonowning_variable(
        &mut self,
        name: &str,
        value: ir::Value,
        kome_type: KomeType,
    ) -> CodegenResult<()> {
        let representation = kome_type.cranelift().ok_or_else(|| {
            CodegenError::new("internal error: Void cannot be stored in a variable", None)
        })?;
        let variable = self.builder.declare_var(representation);
        self.builder.def_var(variable, value);
        let binding_id = self.next_binding_id;
        self.next_binding_id += 1;
        self.scopes
            .last_mut()
            .expect("scope stack is never empty")
            .insert(
                name.to_owned(),
                ScopedVariable {
                    variable,
                    kome_type,
                    owns_value: false,
                    binding_id,
                },
            );
        Ok(())
    }

    fn evaluate(&mut self, expression: &Expression) -> CodegenResult<TypedValue> {
        match expression {
            Expression::Literal(literal) => self.evaluate_literal(literal),

            Expression::Ident(identifier) => self.evaluate_identifier(identifier),

            Expression::Unary(unary) => self.evaluate_unary(unary),

            Expression::Unwrap(unwrap) => self.evaluate_unwrap(unwrap),

            Expression::Group(group) => self.evaluate_group(group),

            Expression::Task(task) => self.evaluate_task(task, None),

            Expression::Wait(wait) => self.evaluate_wait(wait),

            Expression::Cancel(cancel) => self.evaluate_cancel(cancel),

            Expression::Binary(binary) => self.evaluate_binary(binary),

            Expression::Call(call) => self.evaluate_call(call),

            Expression::Assign(assign) => self.evaluate_assign(assign),

            Expression::Struct(struct_) => self.evaluate_struct(struct_),

            Expression::Member(member) => self.evaluate_member(member),

            Expression::Index(index) => self.evaluate_index(index),

            Expression::List(list) => self.evaluate_list(list, None),

            Expression::Block(block) => self.evaluate_block_expression(block, None),

            Expression::Object(object) => self.evaluate_object(object, None),

            Expression::Template(template) => self.evaluate_template(template),

            Expression::Component(component) => self.evaluate_component(
                &component.name,
                &component.args,
                &component.children,
                component.span,
            ),

            Expression::Is(is) => self.evaluate_is_expression(is, None),

            Expression::Closure(closure) => self.evaluate_closure_value(closure),

            Expression::DotIdent(dot) => Err(CodegenError::at(
                "a dot-prefixed case requires an enum context",
                dot.span,
            )),
        }
    }

    fn evaluate_task(
        &mut self,
        task: &TaskExpression,
        expected_result: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        let Expression::Call(call) = task.argument.as_ref() else {
            return Err(CodegenError::at(
                "task body was not lowered to a runtime entry function",
                task.span,
            ));
        };
        let Expression::Ident(callee) = call.callee.as_ref() else {
            return Err(CodegenError::at(
                "task body was not lowered to a runtime entry function",
                task.span,
            ));
        };
        if !callee.name.starts_with("__kome_task_body_") {
            return Err(CodegenError::at(
                "task body was not lowered to a runtime entry function",
                task.span,
            ));
        }
        let signature = self
            .info
            .get(&callee.name)
            .expect("lowered task body must be analyzed")
            .signature()
            .clone();
        if expected_result.is_some_and(|expected| expected != signature.ret) {
            return Err(CodegenError::at(
                "task result has the wrong type",
                task.span,
            ));
        }
        let size = (call.args.len().max(1) * 8) as StackSize;
        let captures = self.builder.create_sized_stack_slot(StackSlotData {
            kind: cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            size,
            align_shift: 3,
            key: None,
        });
        let buffer =
            self.builder
                .ins()
                .stack_addr(self.module.target_config().pointer_type(), captures, 0);
        for (index, (argument, expected)) in call.args.iter().zip(&signature.params).enumerate() {
            let CallArg::Positional(argument) = argument else {
                return Err(CodegenError::at(
                    "task captures must be positional",
                    call.span,
                ));
            };
            let value = self.evaluate_with_expected(argument, Some(*expected))?;
            if value.kome_type != *expected {
                return Err(CodegenError::at(
                    "task capture has the wrong type",
                    argument.span(),
                ));
            }
            let value = self.own_value(value, argument.span())?;
            let slot = self.value_to_task_slot(value, *expected);
            self.builder
                .ins()
                .store(MachMemFlags::new(), slot, buffer, (index * 8) as i32);
        }
        let entry_id = self.task_entry_ids[&callee.name];
        let entry_ref = Module::declare_func_in_func(self.module, entry_id, self.builder.func);
        let entry = self
            .builder
            .ins()
            .func_addr(self.module.target_config().pointer_type(), entry_ref);
        let count = self
            .builder
            .ins()
            .iconst(types::I64, call.args.len() as i64);
        let spawn =
            Module::declare_func_in_func(self.module, self.foreign.task_spawn, self.builder.func);
        let spawn = self.builder.ins().call(spawn, &[entry, buffer, count]);
        let handle = self.builder.inst_results(spawn)[0];

        Ok(TypedValue::some(handle, self.info.task_type(signature.ret)))
    }

    fn evaluate_wait(&mut self, wait: &WaitExpression) -> CodegenResult<TypedValue> {
        let task = self.evaluate(&wait.argument)?;
        let KomeType::Task(id) = task.kome_type else {
            return Err(CodegenError::at(
                format!(
                    "`wait` expects Task<T>, but found {}",
                    self.info.type_name(task.kome_type)
                ),
                wait.argument.span(),
            ));
        };
        let handle = task.expect_value(wait.argument.span())?;
        let wait_fn =
            Module::declare_func_in_func(self.module, self.foreign.task_wait, self.builder.func);
        let wait_call = self.builder.ins().call(wait_fn, &[handle]);
        let state = self.builder.inst_results(wait_call)[0];
        self.require_task_completed(state, 0);
        let result_type = self.info.task_result(id);
        if result_type == KomeType::Void {
            self.release_owned_temporary(task, wait.argument.span())?;
            return Ok(TypedValue::void());
        }
        let result_fn =
            Module::declare_func_in_func(self.module, self.foreign.task_result, self.builder.func);
        let call = self.builder.ins().call(result_fn, &[handle]);
        let value = self.task_slot_to_value(self.builder.inst_results(call)[0], result_type);
        if result_type.is_managed() {
            self.retain_managed(value, result_type);
        }
        self.release_owned_temporary(task, wait.argument.span())?;
        Ok(TypedValue::some(value, result_type))
    }

    fn evaluate_cancel(&mut self, cancel: &CancelExpression) -> CodegenResult<TypedValue> {
        let task = self.evaluate(&cancel.argument)?;
        if !matches!(task.kome_type, KomeType::Task(_)) {
            return Err(CodegenError::at(
                format!(
                    "`cancel` expects Task<T>, but found {}",
                    self.info.type_name(task.kome_type)
                ),
                cancel.argument.span(),
            ));
        }
        let handle = task.expect_value(cancel.argument.span())?;
        let function =
            Module::declare_func_in_func(self.module, self.foreign.task_cancel, self.builder.func);
        self.builder.ins().call(function, &[handle]);
        self.release_owned_temporary(task, cancel.argument.span())?;
        Ok(TypedValue::void())
    }

    fn value_to_task_slot(&mut self, value: ir::Value, kome_type: KomeType) -> ir::Value {
        match kome_type.cranelift().expect("task result representation") {
            types::I8 | types::I16 | types::I32 => self.builder.ins().uextend(types::I64, value),
            types::F32 => {
                let bits = self
                    .builder
                    .ins()
                    .bitcast(types::I32, MachMemFlags::new(), value);
                self.builder.ins().uextend(types::I64, bits)
            }
            types::F64 => self
                .builder
                .ins()
                .bitcast(types::I64, MachMemFlags::new(), value),
            _ => value,
        }
    }

    fn require_task_completed(&mut self, state: ir::Value, operation: i64) {
        let operation = self.builder.ins().iconst(types::I8, operation);
        let require = Module::declare_func_in_func(
            self.module,
            self.foreign.task_require_completed,
            self.builder.func,
        );
        self.builder.ins().call(require, &[state, operation]);
    }

    fn task_slot_to_value(&mut self, slot: ir::Value, kome_type: KomeType) -> ir::Value {
        match kome_type.cranelift().expect("task result representation") {
            types::I8 => self.builder.ins().ireduce(types::I8, slot),
            types::I16 => self.builder.ins().ireduce(types::I16, slot),
            types::I32 => self.builder.ins().ireduce(types::I32, slot),
            types::F32 => {
                let bits = self.builder.ins().ireduce(types::I32, slot);
                self.builder
                    .ins()
                    .bitcast(types::F32, MachMemFlags::new(), bits)
            }
            types::F64 => self
                .builder
                .ins()
                .bitcast(types::F64, MachMemFlags::new(), slot),
            _ => slot,
        }
    }

    fn evaluate_literal(&mut self, literal: &LiteralExpression) -> CodegenResult<TypedValue> {
        match &literal.kind {
            LiteralKind::Number(number) => {
                let pointer = self.c_string_pointer(&number.0)?;
                let length = self.builder.ins().iconst(types::I64, number.0.len() as i64);

                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.number_parse,
                    self.builder.func,
                );

                let call = self.builder.ins().call(function, &[pointer, length]);
                let value = self.builder.inst_results(call)[0];

                Ok(TypedValue::some(value, KomeType::Number))
            }

            LiteralKind::String(string) => {
                let pointer = self.c_string_pointer(string)?;
                let length = self.builder.ins().iconst(types::I64, string.len() as i64);

                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.string_create,
                    self.builder.func,
                );

                let call = self.builder.ins().call(function, &[pointer, length]);
                let value = self.builder.inst_results(call)[0];

                Ok(TypedValue::some(value, KomeType::String))
            }

            LiteralKind::Boolean(flag) => Ok(TypedValue::some(
                self.builder.ins().iconst(types::I8, i64::from(*flag)),
                KomeType::Boolean,
            )),

            LiteralKind::Null => Ok(TypedValue::some(
                self.builder.ins().iconst(types::I8, 0),
                KomeType::Null,
            )),

            LiteralKind::Percent(number) => self.evaluate_literal(&LiteralExpression {
                span: literal.span,
                kind: LiteralKind::Number(number.clone()),
            }),
        }
    }

    fn evaluate_unary(&mut self, unary: &UnaryExpression) -> CodegenResult<TypedValue> {
        let argument = self.evaluate(&unary.argument)?;
        match unary.op {
            UnaryOp::Not if argument.kome_type == KomeType::Boolean => {
                let value = argument.expect_value(unary.argument.span())?;
                Ok(TypedValue::some(
                    self.builder.ins().icmp_imm_u(IntCC::Equal, value, 0),
                    KomeType::Boolean,
                ))
            }
            UnaryOp::Not => Err(CodegenError::at(
                format!(
                    "operator `!` expects bool, but found {}",
                    self.info.type_name(argument.kome_type)
                ),
                unary.span,
            )),
        }
    }

    fn evaluate_unwrap(&mut self, unwrap: &UnwrapExpression) -> CodegenResult<TypedValue> {
        let optional = self.evaluate(&unwrap.argument)?;
        let KomeType::Optional(id) = optional.kome_type else {
            return Err(CodegenError::at(
                format!(
                    "postfix `!` expects an optional value, but found {}",
                    self.info.type_name(optional.kome_type)
                ),
                unwrap.argument.span(),
            ));
        };
        let pointer = optional.expect_value(unwrap.argument.span())?;
        let require = Module::declare_func_in_func(
            self.module,
            self.foreign.optional_require,
            self.builder.func,
        );
        let call = self.builder.ins().call(require, &[pointer]);
        let pointer = self.builder.inst_results(call)[0];
        let inner = self.info.optional_inner(id);
        let slot = self
            .builder
            .ins()
            .load(types::I64, MachMemFlags::new(), pointer, 0);
        let value = self.task_slot_to_value(slot, inner);
        if inner.is_managed() {
            self.retain_managed(value, inner);
        }
        self.release_owned_temporary(optional, unwrap.argument.span())?;
        Ok(TypedValue::some(value, inner))
    }

    fn evaluate_identifier(
        &mut self,
        identifier: &IdentifierExpression,
    ) -> CodegenResult<TypedValue> {
        let scope = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(&identifier.name));
        let Some(scope) = scope else {
            return self.evaluate_global(identifier);
        };

        let scoped = self.scopes[scope][&identifier.name];

        let ownership = if self.allow_managed_moves {
            let remaining = self
                .remaining_reads
                .get_mut(&scoped.binding_id)
                .ok_or_else(|| {
                    CodegenError::at(
                        format!(
                            "internal error: missing read count for `{}`",
                            identifier.name
                        ),
                        identifier.span,
                    )
                })?;
            *remaining -= 1;
            if scoped.kome_type.is_managed() && scoped.owns_value && *remaining == 0 {
                ValueOwnership::BorrowedMovable {
                    scope,
                    variable: scoped.variable,
                }
            } else {
                ValueOwnership::Borrowed
            }
        } else {
            ValueOwnership::Borrowed
        };

        Ok(TypedValue {
            value: Some(self.builder.use_var(scoped.variable)),
            kome_type: scoped.kome_type,
            ownership,
        })
    }

    fn evaluate_global(&mut self, identifier: &IdentifierExpression) -> CodegenResult<TypedValue> {
        let global = self
            .info
            .globals
            .get(&identifier.name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;
        let expected = global.kome_type.ok_or_else(|| {
            CodegenError::at(
                format!("global `{}` is not a runtime value", identifier.name),
                identifier.span,
            )
        })?;
        let storage = self.global_storage[&identifier.name];
        let initialized_data =
            Module::declare_data_in_func(self.module, storage.initialized, self.builder.func);
        let initialized_pointer = self
            .builder
            .ins()
            .symbol_value(self.module.target_config().pointer_type(), initialized_data);
        let value_data =
            Module::declare_data_in_func(self.module, storage.value, self.builder.func);
        let value_pointer = self
            .builder
            .ins()
            .symbol_value(self.module.target_config().pointer_type(), value_data);
        let initialized =
            self.builder
                .ins()
                .load(types::I8, MachMemFlags::new(), initialized_pointer, 0);
        let initialize = self.builder.create_block();
        let ready = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.append_block_param(
            done,
            expected.cranelift().expect("global has a representation"),
        );
        self.builder
            .ins()
            .brif(initialized, ready, &[], initialize, &[]);

        self.builder.switch_to_block(ready);
        self.builder.seal_block(ready);
        let existing = self.builder.ins().load(
            expected.cranelift().expect("global has a representation"),
            MachMemFlags::new(),
            value_pointer,
            0,
        );
        self.builder
            .ins()
            .jump(done, &[ir::BlockArg::Value(existing)]);

        self.builder.switch_to_block(initialize);
        self.builder.seal_block(initialize);
        if self.evaluating_globals.contains(&identifier.name) {
            return Err(CodegenError::at(
                format!("cyclic global initializer for `{}`", identifier.name),
                identifier.span,
            ));
        }
        self.evaluating_globals.push(identifier.name.clone());
        let result = match &global.binding.init {
            Some(initializer) => self.evaluate_with_expected(initializer, Some(expected)),
            None => Ok(TypedValue::some(self.zero_value(expected)?, expected)),
        };
        self.evaluating_globals.pop();
        let result = result?;
        if result.kome_type != expected {
            return Err(CodegenError::at(
                format!(
                    "global `{}` expects {}, but found {}",
                    identifier.name,
                    self.info.type_name(expected),
                    self.info.type_name(result.kome_type)
                ),
                identifier.span,
            ));
        }
        let value = self.own_value(result, identifier.span)?;
        self.builder
            .ins()
            .store(MachMemFlags::new(), value, value_pointer, 0);
        let initialized = self.builder.ins().iconst(types::I8, 1);
        self.builder
            .ins()
            .store(MachMemFlags::new(), initialized, initialized_pointer, 0);
        self.builder.ins().jump(done, &[ir::BlockArg::Value(value)]);

        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
        Ok(TypedValue::borrowed(
            self.builder.block_params(done)[0],
            expected,
        ))
    }

    fn evaluate_group(&mut self, group: &GroupExpression) -> CodegenResult<TypedValue> {
        self.evaluate(&group.expression)
    }

    fn evaluate_with_expected(
        &mut self,
        expression: &Expression,
        expected: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        if let Expression::Group(group) = expression {
            return self.evaluate_with_expected(&group.expression, expected);
        }
        if let Expression::Task(task) = expression
            && let Some(KomeType::Task(id)) = expected
        {
            return self.evaluate_task(task, Some(self.info.task_result(id)));
        }
        if let Expression::Literal(literal) = expression {
            if matches!(literal.kind, LiteralKind::Null)
                && let Some(optional @ KomeType::Optional(_)) = expected
            {
                return Ok(TypedValue::some(
                    self.builder.ins().iconst(types::I64, 0),
                    optional,
                ));
            }
            if let LiteralKind::Number(number) = &literal.kind {
                if let Some(expected) = expected {
                    return self.evaluate_numeric_literal(number, literal.span, expected);
                }
            }
        }
        if let Expression::List(list) = expression {
            let element = match expected {
                Some(KomeType::List(id)) => Some(self.info.list_element(id)),
                _ => None,
            };
            return self.evaluate_list(list, element);
        }
        if let Expression::Block(block) = expression {
            return self.evaluate_block_expression(block, expected);
        }
        if let Expression::Object(object) = expression {
            return self.evaluate_object(object, expected);
        }
        if let Expression::DotIdent(dot) = expression {
            let Some(KomeType::Enum(id)) = expected else {
                return Err(CodegenError::at(
                    "a dot-prefixed case requires an enum context",
                    dot.span,
                ));
            };
            return self.evaluate_enum_case(id, &dot.name, dot.span);
        }
        if let Expression::Is(is) = expression {
            return self.evaluate_is_expression(is, expected);
        }

        if let Some(optional @ KomeType::Optional(id)) = expected {
            let inner = self.info.optional_inner(id);
            let value = self.evaluate_with_expected(expression, Some(inner))?;
            if value.kome_type == optional {
                return Ok(value);
            }
            if value.kome_type != inner {
                return Ok(value);
            }
            let payload = self.own_value(value, expression.span())?;
            let size = self.builder.ins().iconst(types::I64, 8);
            let alloc = Module::declare_func_in_func(
                self.module,
                self.foreign.struct_alloc,
                self.builder.func,
            );
            let call = self.builder.ins().call(alloc, &[size]);
            let pointer = self.builder.inst_results(call)[0];
            let slot = self.value_to_task_slot(payload, inner);
            self.builder
                .ins()
                .store(MachMemFlags::new(), slot, pointer, 0);
            return Ok(TypedValue::some(pointer, optional));
        }

        self.evaluate(expression)
    }

    fn evaluate_enum_case(
        &mut self,
        id: usize,
        case_name: &str,
        span: Span,
    ) -> CodegenResult<TypedValue> {
        let enum_ = &self.info.enums[id];
        let tag = enum_
            .cases
            .iter()
            .position(|case| case == case_name)
            .ok_or_else(|| {
                CodegenError::at(
                    format!("enum `{}` has no case `{case_name}`", enum_.name),
                    span,
                )
            })?;
        Ok(TypedValue::some(
            self.builder.ins().iconst(types::I64, tag as i64),
            KomeType::Enum(id),
        ))
    }

    fn evaluate_is_expression(
        &mut self,
        expression: &kome_ast::expressions::IsExpression,
        expected: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        if let IsPattern::Ident(pattern) = &expression.pattern {
            let value = self.evaluate(&expression.value)?;
            let raw = self.own_value(value, expression.value.span())?;
            self.scopes.push(HashMap::new());
            self.declare_variable(&pattern.name, raw, value.kome_type, ValueOwnership::Owned)?;
            let mut result = self.evaluate_with_expected(&expression.body, expected)?;
            if result.kome_type.is_managed() {
                let raw = self.own_value(result, expression.body.span())?;
                result = TypedValue::some(raw, result.kome_type);
            }
            let scope = self.scopes.pop().expect("is expression scope exists");
            for scoped in scope.values() {
                if scoped.kome_type.is_managed() && scoped.owns_value {
                    let value = self.builder.use_var(scoped.variable);
                    self.release_managed(value, scoped.kome_type);
                }
            }
            return Ok(result);
        }
        let pattern = match &expression.pattern {
            IsPattern::Literal(pattern) => Expression::literal(pattern.value.clone(), pattern.span),
            IsPattern::DotIdent(pattern) => {
                Expression::DotIdent(kome_ast::expressions::DotIdentifierExpression {
                    span: pattern.span,
                    name: pattern.name.clone(),
                })
            }
            IsPattern::Ident(_) => unreachable!(),
        };
        let comparison = BinaryExpression {
            span: expression.span,
            op: BinaryOp::Eq,
            left: expression.value.clone(),
            right: Box::new(pattern),
        };
        let condition = self
            .evaluate_binary(&comparison)?
            .expect_value(expression.span)?;
        let matched = self.builder.create_block();
        let unmatched = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder
            .ins()
            .brif(condition, matched, &[], unmatched, &[]);
        self.builder.switch_to_block(matched);
        self.builder.seal_block(matched);
        let result = self.evaluate_with_expected(&expression.body, expected)?;
        if result.kome_type == KomeType::Void {
            return Err(CodegenError::at(
                "an inline `is` body must produce a value",
                expression.body.span(),
            ));
        }
        let result_type = result.kome_type;
        let result_value = self.own_value(result, expression.body.span())?;
        self.builder.append_block_param(
            done,
            result_type.cranelift().expect("is result representation"),
        );
        self.builder
            .ins()
            .jump(done, &[ir::BlockArg::Value(result_value)]);
        self.builder.switch_to_block(unmatched);
        self.builder.seal_block(unmatched);
        let fallback = match result_type {
            KomeType::Number => self
                .evaluate_literal(&LiteralExpression {
                    span: expression.span,
                    kind: LiteralKind::Number(NumberLiteral("0".into())),
                })?
                .expect_value(expression.span)?,
            KomeType::String => self
                .evaluate_literal(&LiteralExpression {
                    span: expression.span,
                    kind: LiteralKind::String(String::new()),
                })?
                .expect_value(expression.span)?,
            other => self.zero_value(other)?,
        };
        self.builder
            .ins()
            .jump(done, &[ir::BlockArg::Value(fallback)]);
        self.builder.seal_block(done);
        self.builder.switch_to_block(done);
        Ok(TypedValue::some(
            self.builder.block_params(done)[0],
            result_type,
        ))
    }

    fn evaluate_block_expression(
        &mut self,
        expression: &BlockExpression,
        expected: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        self.scopes.push(HashMap::new());
        for statement in &expression.statements {
            if self.terminated {
                break;
            }
            self.translate_statement(statement)?;
        }
        if self.terminated {
            self.scopes.pop();
            return Ok(TypedValue::void());
        }
        let mut result = match &expression.tail {
            Some(tail) => self.evaluate_with_expected(tail, expected)?,
            None => TypedValue::void(),
        };
        if result.kome_type.is_managed() {
            let value = self.own_value(result, expression.span)?;
            result = TypedValue::some(value, result.kome_type);
        }
        let scope = self.scopes.pop().expect("block expression scope exists");
        for scoped in scope.values() {
            if scoped.kome_type.is_managed() && scoped.owns_value {
                let value = self.builder.use_var(scoped.variable);
                self.release_managed(value, scoped.kome_type);
            }
        }
        Ok(result)
    }

    fn evaluate_object(
        &mut self,
        expression: &ObjectExpression,
        expected: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        if let Some(KomeType::Struct(id)) = expected {
            let fields = expression
                .props
                .iter()
                .map(|property| match property {
                    ObjectProperty::KeyValue(property) => property.clone(),
                })
                .collect::<Vec<_>>();
            let name = self.info.struct_info(id).name.clone();
            return self.evaluate_struct_fields(id, &name, &fields, expression.span);
        }

        let mut values = Vec::with_capacity(expression.props.len());
        let mut field_types = Vec::with_capacity(expression.props.len());
        let mut seen = std::collections::HashSet::new();
        for property in &expression.props {
            let ObjectProperty::KeyValue(property) = property;
            let name = static_property_name(&property.key).ok_or_else(|| {
                CodegenError::at(
                    "computed object keys must be string or number literals",
                    property.span,
                )
            })?;
            if !seen.insert(name.clone()) {
                return Err(CodegenError::at(
                    format!("duplicate object property `{name}`"),
                    property.span,
                ));
            }
            let value = self.evaluate(&property.value)?;
            if value.kome_type == KomeType::Void {
                return Err(CodegenError::at(
                    "object properties cannot have type Void",
                    property.value.span(),
                ));
            }
            field_types.push((name.clone(), value.kome_type));
            values.push((name, value, property.value.span()));
        }
        let object_type = {
            let mut structs = self.info.structs.borrow_mut();
            intern_anonymous_struct(&mut structs, field_types, expression.span)?
        };
        let KomeType::Struct(id) = object_type else {
            unreachable!();
        };
        let layout = self.info.struct_info(id);
        let size = self
            .builder
            .ins()
            .iconst(types::I64, i64::from(layout.size));
        let alloc =
            Module::declare_func_in_func(self.module, self.foreign.struct_alloc, self.builder.func);
        let call = self.builder.ins().call(alloc, &[size]);
        let pointer = self.builder.inst_results(call)[0];
        for field in &layout.fields {
            let (_, value, span) = values
                .iter()
                .find(|(name, _, _)| name == &field.name)
                .expect("anonymous object fields match their interned layout");
            let raw = self.own_value(*value, *span)?;
            self.builder
                .ins()
                .store(MachMemFlags::new(), raw, pointer, field.offset);
        }
        Ok(TypedValue::some(pointer, object_type))
    }

    fn evaluate_template(&mut self, expression: &TemplateExpression) -> CodegenResult<TypedValue> {
        let mut result = self.evaluate_literal(&LiteralExpression {
            span: expression.span,
            kind: LiteralKind::String(String::new()),
        })?;
        for part in &expression.parts {
            let next = match part {
                TemplatePart::String { value, span } => {
                    self.evaluate_literal(&LiteralExpression {
                        span: *span,
                        kind: LiteralKind::String(value.clone()),
                    })?
                }
                TemplatePart::Expression { expression, span } => {
                    let value = self.evaluate(expression)?;
                    self.format_template_value(value, *span)?
                }
            };
            let left = result.expect_value(expression.span)?;
            let right = next.expect_value(expression.span)?;
            let combined = self.emit_number_binary(self.foreign.string_concat, left, right);
            self.release_owned_temporary(result, expression.span)?;
            self.release_owned_temporary(next, expression.span)?;
            result = TypedValue::some(combined, KomeType::String);
        }
        Ok(result)
    }

    fn format_template_value(
        &mut self,
        value: TypedValue,
        span: Span,
    ) -> CodegenResult<TypedValue> {
        if value.kome_type == KomeType::String {
            return Ok(value);
        }
        let raw = value.expect_value(span)?;
        let formatted = match value.kome_type {
            KomeType::Number => {
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.number_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[raw]);
                self.builder.inst_results(call)[0]
            }
            KomeType::Boolean => {
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.boolean_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[raw]);
                self.builder.inst_results(call)[0]
            }
            KomeType::I8 | KomeType::I16 | KomeType::I32 | KomeType::I64 => {
                let widened = if value.kome_type == KomeType::I64 {
                    raw
                } else {
                    self.builder.ins().sextend(types::I64, raw)
                };
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.signed_integer_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[widened]);
                self.builder.inst_results(call)[0]
            }
            KomeType::U8 | KomeType::U16 | KomeType::U32 | KomeType::U64 => {
                let widened = if value.kome_type == KomeType::U64 {
                    raw
                } else {
                    self.builder.ins().uextend(types::I64, raw)
                };
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.unsigned_integer_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[widened]);
                self.builder.inst_results(call)[0]
            }
            KomeType::F32 => {
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.f32_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[raw]);
                self.builder.inst_results(call)[0]
            }
            KomeType::F64 => {
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.f64_to_string,
                    self.builder.func,
                );
                let call = self.builder.ins().call(function, &[raw]);
                self.builder.inst_results(call)[0]
            }
            KomeType::Null => {
                return self.evaluate_literal(&LiteralExpression {
                    span,
                    kind: LiteralKind::String("null".into()),
                });
            }
            KomeType::Enum(id) => return self.format_enum_value(id, raw, span),
            other => {
                return Err(CodegenError::at(
                    format!(
                        "template interpolation does not support {}",
                        self.info.type_name(other)
                    ),
                    span,
                ));
            }
        };
        self.release_owned_temporary(value, span)?;
        Ok(TypedValue::some(formatted, KomeType::String))
    }

    fn format_enum_value(
        &mut self,
        id: usize,
        tag: ir::Value,
        span: Span,
    ) -> CodegenResult<TypedValue> {
        let enum_ = self.info.enums[id].clone();
        let done = self.builder.create_block();
        self.builder.append_block_param(done, types::I64);
        let mut next = None;
        for (index, case_name) in enum_.cases.iter().enumerate() {
            if let Some(block) = next.take() {
                self.builder.switch_to_block(block);
                self.builder.seal_block(block);
            }
            let matched = self.builder.create_block();
            let unmatched = self.builder.create_block();
            let condition = self
                .builder
                .ins()
                .icmp_imm_u(IntCC::Equal, tag, index as i64);
            self.builder
                .ins()
                .brif(condition, matched, &[], unmatched, &[]);
            self.builder.switch_to_block(matched);
            self.builder.seal_block(matched);
            let formatted = if let Some(raw_value) = &enum_.raw_values[index] {
                let value = self.evaluate(raw_value)?;
                self.format_template_value(value, raw_value.span())?
            } else {
                self.evaluate_literal(&LiteralExpression {
                    span,
                    kind: LiteralKind::String(case_name.clone()),
                })?
            };
            self.builder
                .ins()
                .jump(done, &[ir::BlockArg::Value(formatted.expect_value(span)?)]);
            next = Some(unmatched);
        }
        let fallback = if let Some(block) = next {
            block
        } else {
            let block = self.builder.create_block();
            self.builder.ins().jump(block, &[]);
            block
        };
        self.builder.switch_to_block(fallback);
        self.builder.seal_block(fallback);
        let invalid = self.evaluate_literal(&LiteralExpression {
            span,
            kind: LiteralKind::String("<invalid enum>".into()),
        })?;
        self.builder
            .ins()
            .jump(done, &[ir::BlockArg::Value(invalid.expect_value(span)?)]);
        self.builder.seal_block(done);
        self.builder.switch_to_block(done);
        Ok(TypedValue::some(
            self.builder.block_params(done)[0],
            KomeType::String,
        ))
    }

    fn evaluate_list(
        &mut self,
        expression: &ListExpression,
        expected_element: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        let mut values = Vec::with_capacity(expression.elems.len());
        let mut element_type = expected_element;
        for (index, element) in expression.elems.iter().enumerate() {
            let Some(element) = element else {
                let Some(expected) = element_type else {
                    return Err(CodegenError::at(
                        "a list beginning with an empty element requires a concrete list type annotation",
                        expression.span,
                    ));
                };
                values.push((
                    TypedValue::some(self.zero_value(expected)?, expected),
                    expression.span,
                ));
                continue;
            };
            let typed = self.evaluate_with_expected(element, element_type)?;
            if let Some(expected) = element_type {
                if typed.kome_type != expected {
                    return Err(CodegenError::at(
                        format!(
                            "list element {index} expects {}, but found {}",
                            self.info.type_name(expected),
                            self.info.type_name(typed.kome_type)
                        ),
                        element.span(),
                    ));
                }
            } else {
                element_type = Some(typed.kome_type);
            }
            values.push((typed, element.span()));
        }
        if values.is_empty() && element_type.is_none() {
            return Err(CodegenError::at(
                "an empty list requires a concrete list type annotation",
                expression.span,
            ));
        }
        let element_type = element_type.expect("nonempty or context-typed list");
        if element_type == KomeType::Void {
            return Err(CodegenError::at(
                "list elements cannot have type Void",
                expression.span,
            ));
        }
        let length = self
            .builder
            .ins()
            .iconst(types::I64, expression.elems.len() as i64);
        let alloc =
            Module::declare_func_in_func(self.module, self.foreign.list_alloc, self.builder.func);
        let call = self.builder.ins().call(alloc, &[length]);
        let list = self.builder.inst_results(call)[0];
        for (index, (typed, span)) in values.into_iter().enumerate() {
            let value = self.own_value(typed, span)?;
            let slot = self.value_to_task_slot(value, element_type);
            self.builder
                .ins()
                .store(MachMemFlags::new(), slot, list, (index * 8) as i32);
        }
        Ok(TypedValue::some(list, self.info.list_type(element_type)))
    }

    fn evaluate_numeric_literal(
        &mut self,
        number: &NumberLiteral,
        span: Span,
        expected: KomeType,
    ) -> CodegenResult<TypedValue> {
        match expected {
            KomeType::I8
            | KomeType::I16
            | KomeType::I32
            | KomeType::I64
            | KomeType::U8
            | KomeType::U16
            | KomeType::U32
            | KomeType::U64 => {
                if number.0.contains('.') {
                    return Err(CodegenError::at(
                        format!("fractional literal cannot be used as {}", expected.name(),),
                        span,
                    ));
                }

                let value = number.0.parse::<i64>().map_err(|_| {
                    CodegenError::at(format!("`{}` is not a valid integer", number.0), span)
                })?;

                let representation = expected
                    .cranelift()
                    .expect("integer types always have a Cranelift representation");

                Ok(TypedValue::some(
                    self.builder.ins().iconst(representation, value),
                    expected,
                ))
            }

            KomeType::F32 => {
                let value = number.0.parse::<f32>().map_err(|_| {
                    CodegenError::at(format!("`{}` is not a valid f32", number.0), span)
                })?;

                Ok(TypedValue::some(
                    self.builder.ins().f32const(value),
                    KomeType::F32,
                ))
            }

            KomeType::F64 => {
                let value = number.0.parse::<f64>().map_err(|_| {
                    CodegenError::at(format!("`{}` is not a valid number", number.0), span)
                })?;

                Ok(TypedValue::some(
                    self.builder.ins().f64const(value),
                    KomeType::F64,
                ))
            }

            KomeType::Number => self.evaluate_literal(&LiteralExpression {
                span,
                kind: LiteralKind::Number(number.clone()),
            }),

            _ => self.evaluate_literal(&LiteralExpression {
                span,
                kind: LiteralKind::Number(number.clone()),
            }),
        }
    }

    fn c_string_pointer(&mut self, symbol: &str) -> CodegenResult<ir::Value> {
        let data_id = self.native_symbols.intern(self.module, symbol)?;
        let global = Module::declare_data_in_func(self.module, data_id, self.builder.func);

        Ok(self
            .builder
            .ins()
            .symbol_value(self.module.target_config().pointer_type(), global))
    }

    fn evaluate_struct(&mut self, expression: &StructExpression) -> CodegenResult<TypedValue> {
        let id = self
            .info
            .struct_ids
            .get(&expression.name)
            .copied()
            .ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "struct `{}` was not found or has a runtime representation",
                        expression.name
                    ),
                    expression.span,
                )
            })?;
        self.evaluate_struct_fields(id, &expression.name, &expression.fields, expression.span)
    }

    fn evaluate_struct_fields(
        &mut self,
        id: usize,
        name: &str,
        properties: &[kome_ast::expressions::KeyValueProperty],
        span: Span,
    ) -> CodegenResult<TypedValue> {
        let layout = self.info.struct_info(id).clone();
        if properties.len() != layout.fields.len() {
            return Err(CodegenError::at(
                format!(
                    "struct `{}` expects {} field(s), but received {}",
                    name,
                    layout.fields.len(),
                    properties.len()
                ),
                span,
            ));
        }
        let size = self
            .builder
            .ins()
            .iconst(types::I64, i64::from(layout.size));
        let alloc =
            Module::declare_func_in_func(self.module, self.foreign.struct_alloc, self.builder.func);
        let call = self.builder.ins().call(alloc, &[size]);
        let pointer = self.builder.inst_results(call)[0];

        for field in &layout.fields {
            let property = properties
                .iter()
                .find(|property| {
                    static_property_name(&property.key).as_deref() == Some(field.name.as_str())
                })
                .ok_or_else(|| {
                    CodegenError::at(
                        format!("missing field `{}` in `{}`", field.name, name),
                        span,
                    )
                })?;
            let duplicates = properties
                .iter()
                .filter(|candidate| {
                    static_property_name(&candidate.key).as_deref() == Some(field.name.as_str())
                })
                .count();
            if duplicates != 1 {
                return Err(CodegenError::at(
                    format!("duplicate field `{}`", field.name),
                    property.span,
                ));
            }
            let typed = self.evaluate_with_expected(&property.value, Some(field.kome_type))?;
            if typed.kome_type != field.kome_type {
                return Err(CodegenError::at(
                    format!(
                        "field `{}` expects {}, but received {}",
                        field.name,
                        self.info.type_name(field.kome_type),
                        self.info.type_name(typed.kome_type)
                    ),
                    property.value.span(),
                ));
            }
            let value = self.own_value(typed, property.value.span())?;
            self.builder
                .ins()
                .store(MachMemFlags::new(), value, pointer, field.offset);
        }
        for property in properties {
            let property_name = static_property_name(&property.key).ok_or_else(|| {
                CodegenError::at(
                    "computed object keys must be string or number literals",
                    property.span,
                )
            })?;
            if !layout
                .fields
                .iter()
                .any(|field| field.name == property_name)
            {
                return Err(CodegenError::at(
                    format!("struct `{name}` has no field `{property_name}`"),
                    property.span,
                ));
            }
        }
        Ok(TypedValue::some(pointer, KomeType::Struct(id)))
    }

    fn evaluate_member(&mut self, member: &MemberExpression) -> CodegenResult<TypedValue> {
        if let Expression::Ident(identifier) = member.object.as_ref()
            && !self
                .scopes
                .iter()
                .any(|scope| scope.contains_key(&identifier.name))
            && let Some(KomeType::Enum(id)) = self.info.runtime_types.get(&identifier.name).copied()
        {
            return self.evaluate_enum_case(id, &member.property, member.span);
        }
        if let Expression::Ident(identifier) = member.object.as_ref()
            && !self
                .scopes
                .iter()
                .any(|scope| scope.contains_key(&identifier.name))
            && let Some(&id) = self.info.struct_ids.get(&identifier.name)
        {
            let target = KomeType::Struct(id);
            let (_, _, constant) = self
                .info
                .implementation_member(target, &member.property)
                .ok_or_else(|| {
                    CodegenError::at(
                        format!(
                            "type `{}` has no static member `{}`",
                            identifier.name, member.property
                        ),
                        member.span,
                    )
                })?;
            let constant = constant.ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "`{}.{}` is a method and must be called",
                        identifier.name, member.property
                    ),
                    member.span,
                )
            })?;
            let initializer = constant.binding.init.clone().ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "associated constant `{}` has no initializer",
                        member.property
                    ),
                    constant.binding.span,
                )
            })?;
            let expected = constant.kome_type;
            let value = self.evaluate_with_expected(&initializer, Some(expected))?;
            if value.kome_type != expected {
                return Err(CodegenError::at(
                    "associated constant initializer has the wrong type",
                    initializer.span(),
                ));
            }
            return Ok(value);
        }

        let object = self.evaluate(&member.object)?;
        let KomeType::Struct(id) = object.kome_type else {
            return Err(CodegenError::at(
                format!(
                    "type `{}` has no field `{}`",
                    self.info.type_name(object.kome_type),
                    member.property
                ),
                member.span,
            ));
        };
        let field = self
            .info
            .struct_info(id)
            .fields
            .iter()
            .find(|field| field.name == member.property)
            .cloned()
            .ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "struct `{}` has no field `{}`",
                        self.info.struct_info(id).name,
                        member.property
                    ),
                    member.span,
                )
            })?;
        let pointer = object.expect_value(member.object.span())?;
        let value = self.builder.ins().load(
            field.kome_type.cranelift().expect("field representation"),
            MachMemFlags::new(),
            pointer,
            field.offset,
        );
        if field.kome_type.is_managed() {
            self.retain_managed(value, field.kome_type);
        }
        self.release_owned_temporary(object, member.object.span())?;
        Ok(TypedValue::some(value, field.kome_type))
    }

    fn evaluate_index(
        &mut self,
        index: &kome_ast::expressions::IndexExpression,
    ) -> CodegenResult<TypedValue> {
        let object = self.evaluate(&index.object)?;
        if let KomeType::Struct(id) = object.kome_type
            && self.info.struct_info(id).anonymous
        {
            let key = static_index_name(&index.index).ok_or_else(|| {
                CodegenError::at(
                    "structural object indices must be string or number literals",
                    index.index.span(),
                )
            })?;
            let layout = self.info.struct_info(id);
            let field = layout
                .fields
                .iter()
                .find(|field| field.name == key)
                .cloned()
                .ok_or_else(|| {
                    CodegenError::at(format!("object has no property `{key}`"), index.span)
                })?;
            let pointer = object.expect_value(index.object.span())?;
            let value = self.builder.ins().load(
                field.kome_type.cranelift().expect("field representation"),
                MachMemFlags::new(),
                pointer,
                field.offset,
            );
            if field.kome_type.is_managed() {
                self.retain_managed(value, field.kome_type);
            }
            self.release_owned_temporary(object, index.object.span())?;
            return Ok(TypedValue::some(value, field.kome_type));
        }
        let KomeType::List(id) = object.kome_type else {
            return Err(CodegenError::at(
                "indexing expects a List",
                index.object.span(),
            ));
        };
        let typed_index = self.evaluate(&index.index)?;
        let raw_index = typed_index.expect_value(index.index.span())?;
        let index_value = match typed_index.kome_type {
            KomeType::Number => {
                let convert = Module::declare_func_in_func(
                    self.module,
                    self.foreign.number_to_i64,
                    self.builder.func,
                );
                let call = self.builder.ins().call(convert, &[raw_index]);
                self.builder.inst_results(call)[0]
            }
            KomeType::I8 | KomeType::I16 | KomeType::I32 => {
                self.builder.ins().sextend(types::I64, raw_index)
            }
            KomeType::U8 | KomeType::U16 | KomeType::U32 => {
                self.builder.ins().uextend(types::I64, raw_index)
            }
            KomeType::I64 | KomeType::U64 => raw_index,
            other => {
                return Err(CodegenError::at(
                    format!(
                        "list index must be an integer, but found {}",
                        self.info.type_name(other)
                    ),
                    index.index.span(),
                ));
            }
        };
        let pointer = object.expect_value(index.object.span())?;
        let require = Module::declare_func_in_func(
            self.module,
            self.foreign.list_require_index,
            self.builder.func,
        );
        let call = self.builder.ins().call(require, &[pointer, index_value]);
        let checked_index = self.builder.inst_results(call)[0];
        let offset = self.builder.ins().imul_imm_u(checked_index, 8);
        let address = self.builder.ins().iadd(pointer, offset);
        let element_type = self.info.list_element(id);
        let slot = self
            .builder
            .ins()
            .load(types::I64, MachMemFlags::new(), address, 0);
        let value = self.task_slot_to_value(slot, element_type);
        if element_type.is_managed() {
            self.retain_managed(value, element_type);
        }
        self.release_owned_temporary(typed_index, index.index.span())?;
        self.release_owned_temporary(object, index.object.span())?;
        Ok(TypedValue::some(value, element_type))
    }

    fn release_owned_temporary(&mut self, value: TypedValue, span: Span) -> CodegenResult<()> {
        if value.kome_type.is_managed() && value.ownership == ValueOwnership::Owned {
            self.release_managed(value.expect_value(span)?, value.kome_type);
        }

        Ok(())
    }

    fn emit_number_binary(
        &mut self,
        function: FuncId,
        left: ir::Value,
        right: ir::Value,
    ) -> ir::Value {
        let function = Module::declare_func_in_func(self.module, function, self.builder.func);
        let call = self.builder.ins().call(function, &[left, right]);

        self.builder.inst_results(call)[0]
    }

    fn evaluate_binary(&mut self, binary: &BinaryExpression) -> CodegenResult<TypedValue> {
        if matches!(binary.op, BinaryOp::And | BinaryOp::Or) {
            return self.evaluate_logical(binary);
        }
        let left = self.evaluate(&binary.left)?;
        let right = self.evaluate_with_expected(&binary.right, Some(left.kome_type))?;

        let span = binary.span;

        let invalid_operands = |left: TypedValue, right: TypedValue| {
            CodegenError::at(
                format!(
                    "binary operation is not supported for {} and {}",
                    left.kome_type.name(),
                    right.kome_type.name(),
                ),
                span,
            )
        };

        match binary.op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                if left.kome_type != right.kome_type {
                    return Err(invalid_operands(left, right));
                }
                let result_type = left.kome_type;
                let left_value = left.expect_value(span)?;
                let right_value = right.expect_value(span)?;
                let value = match result_type {
                    KomeType::Number => {
                        let function = match binary.op {
                            BinaryOp::Add => self.foreign.number_add,
                            BinaryOp::Sub => self.foreign.number_sub,
                            BinaryOp::Mul => self.foreign.number_mul,
                            BinaryOp::Div => self.foreign.number_div,
                            _ => unreachable!(),
                        };
                        self.emit_number_binary(function, left_value, right_value)
                    }
                    KomeType::String if binary.op == BinaryOp::Add => {
                        self.emit_number_binary(self.foreign.string_concat, left_value, right_value)
                    }
                    KomeType::I8
                    | KomeType::I16
                    | KomeType::I32
                    | KomeType::I64
                    | KomeType::U8
                    | KomeType::U16
                    | KomeType::U32
                    | KomeType::U64 => match binary.op {
                        BinaryOp::Add => self.builder.ins().iadd(left_value, right_value),
                        BinaryOp::Sub => self.builder.ins().isub(left_value, right_value),
                        BinaryOp::Mul => self.builder.ins().imul(left_value, right_value),
                        BinaryOp::Div
                            if matches!(
                                result_type,
                                KomeType::U8 | KomeType::U16 | KomeType::U32 | KomeType::U64
                            ) =>
                        {
                            self.builder.ins().udiv(left_value, right_value)
                        }
                        BinaryOp::Div => self.builder.ins().sdiv(left_value, right_value),
                        _ => unreachable!(),
                    },
                    KomeType::F32 | KomeType::F64 => match binary.op {
                        BinaryOp::Add => self.builder.ins().fadd(left_value, right_value),
                        BinaryOp::Sub => self.builder.ins().fsub(left_value, right_value),
                        BinaryOp::Mul => self.builder.ins().fmul(left_value, right_value),
                        BinaryOp::Div => self.builder.ins().fdiv(left_value, right_value),
                        _ => unreachable!(),
                    },
                    _ => return Err(invalid_operands(left, right)),
                };

                self.release_owned_temporary(left, span)?;
                self.release_owned_temporary(right, span)?;
                Ok(TypedValue::some(value, result_type))
            }

            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Lte
            | BinaryOp::Gt
            | BinaryOp::Gte => {
                if left.kome_type == KomeType::Number && right.kome_type == KomeType::Number {
                    let left_value = left.expect_value(span)?;
                    let right_value = right.expect_value(span)?;

                    let function = Module::declare_func_in_func(
                        self.module,
                        self.foreign.number_compare,
                        self.builder.func,
                    );

                    let call = self
                        .builder
                        .ins()
                        .call(function, &[left_value, right_value]);

                    let comparison = self.builder.inst_results(call)[0];

                    self.release_owned_temporary(left, span)?;
                    self.release_owned_temporary(right, span)?;

                    let condition = match binary.op {
                        BinaryOp::Eq => IntCC::Equal,
                        BinaryOp::NotEq => IntCC::NotEqual,
                        BinaryOp::Lt => IntCC::SignedLessThan,
                        BinaryOp::Lte => IntCC::SignedLessThanOrEqual,
                        BinaryOp::Gt => IntCC::SignedGreaterThan,
                        BinaryOp::Gte => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!("only comparison operations are handled here"),
                    };

                    let zero = self.builder.ins().iconst(types::I32, 0);

                    return Ok(TypedValue::some(
                        self.builder.ins().icmp(condition, comparison, zero),
                        KomeType::Boolean,
                    ));
                }

                if left.kome_type == KomeType::String && right.kome_type == KomeType::String {
                    let function = Module::declare_func_in_func(
                        self.module,
                        self.foreign.string_compare,
                        self.builder.func,
                    );
                    let call = self.builder.ins().call(
                        function,
                        &[left.expect_value(span)?, right.expect_value(span)?],
                    );
                    let comparison = self.builder.inst_results(call)[0];
                    self.release_owned_temporary(left, span)?;
                    self.release_owned_temporary(right, span)?;
                    let condition = match binary.op {
                        BinaryOp::Eq => IntCC::Equal,
                        BinaryOp::NotEq => IntCC::NotEqual,
                        BinaryOp::Lt => IntCC::SignedLessThan,
                        BinaryOp::Lte => IntCC::SignedLessThanOrEqual,
                        BinaryOp::Gt => IntCC::SignedGreaterThan,
                        BinaryOp::Gte => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    let zero = self.builder.ins().iconst(types::I32, 0);
                    return Ok(TypedValue::some(
                        self.builder.ins().icmp(condition, comparison, zero),
                        KomeType::Boolean,
                    ));
                }

                if left.kome_type == right.kome_type
                    && matches!(
                        left.kome_type,
                        KomeType::I8
                            | KomeType::I16
                            | KomeType::I32
                            | KomeType::I64
                            | KomeType::U8
                            | KomeType::U16
                            | KomeType::U32
                            | KomeType::U64
                    )
                {
                    let unsigned = matches!(
                        left.kome_type,
                        KomeType::U8 | KomeType::U16 | KomeType::U32 | KomeType::U64
                    );
                    let condition = match (binary.op.clone(), unsigned) {
                        (BinaryOp::Eq, _) => IntCC::Equal,
                        (BinaryOp::NotEq, _) => IntCC::NotEqual,
                        (BinaryOp::Lt, false) => IntCC::SignedLessThan,
                        (BinaryOp::Lte, false) => IntCC::SignedLessThanOrEqual,
                        (BinaryOp::Gt, false) => IntCC::SignedGreaterThan,
                        (BinaryOp::Gte, false) => IntCC::SignedGreaterThanOrEqual,
                        (BinaryOp::Lt, true) => IntCC::UnsignedLessThan,
                        (BinaryOp::Lte, true) => IntCC::UnsignedLessThanOrEqual,
                        (BinaryOp::Gt, true) => IntCC::UnsignedGreaterThan,
                        (BinaryOp::Gte, true) => IntCC::UnsignedGreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    return Ok(TypedValue::some(
                        self.builder.ins().icmp(
                            condition,
                            left.expect_value(span)?,
                            right.expect_value(span)?,
                        ),
                        KomeType::Boolean,
                    ));
                }

                if left.kome_type == right.kome_type
                    && matches!(left.kome_type, KomeType::F32 | KomeType::F64)
                {
                    let condition = match binary.op {
                        BinaryOp::Eq => FloatCC::Equal,
                        BinaryOp::NotEq => FloatCC::NotEqual,
                        BinaryOp::Lt => FloatCC::LessThan,
                        BinaryOp::Lte => FloatCC::LessThanOrEqual,
                        BinaryOp::Gt => FloatCC::GreaterThan,
                        BinaryOp::Gte => FloatCC::GreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    return Ok(TypedValue::some(
                        self.builder.ins().fcmp(
                            condition,
                            left.expect_value(span)?,
                            right.expect_value(span)?,
                        ),
                        KomeType::Boolean,
                    ));
                }

                match (left.kome_type, right.kome_type) {
                    (KomeType::Boolean, KomeType::Boolean) | (KomeType::Null, KomeType::Null)
                        if matches!(binary.op, BinaryOp::Eq | BinaryOp::NotEq) =>
                    {
                        let left_value = left.expect_value(span)?;
                        let right_value = right.expect_value(span)?;

                        let condition = if binary.op == BinaryOp::Eq {
                            IntCC::Equal
                        } else {
                            IntCC::NotEqual
                        };

                        Ok(TypedValue::some(
                            self.builder.ins().icmp(condition, left_value, right_value),
                            KomeType::Boolean,
                        ))
                    }

                    (KomeType::Enum(left_id), KomeType::Enum(right_id))
                        if left_id == right_id
                            && matches!(binary.op, BinaryOp::Eq | BinaryOp::NotEq) =>
                    {
                        let condition = if binary.op == BinaryOp::Eq {
                            IntCC::Equal
                        } else {
                            IntCC::NotEqual
                        };
                        Ok(TypedValue::some(
                            self.builder.ins().icmp(
                                condition,
                                left.expect_value(span)?,
                                right.expect_value(span)?,
                            ),
                            KomeType::Boolean,
                        ))
                    }

                    (KomeType::Optional(left_id), KomeType::Optional(right_id))
                        if left_id == right_id
                            && matches!(binary.op, BinaryOp::Eq | BinaryOp::NotEq) =>
                    {
                        let condition = if binary.op == BinaryOp::Eq {
                            IntCC::Equal
                        } else {
                            IntCC::NotEqual
                        };
                        let result = self.builder.ins().icmp(
                            condition,
                            left.expect_value(span)?,
                            right.expect_value(span)?,
                        );
                        self.release_owned_temporary(left, span)?;
                        self.release_owned_temporary(right, span)?;
                        Ok(TypedValue::some(result, KomeType::Boolean))
                    }

                    _ => Err(invalid_operands(left, right)),
                }
            }

            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn evaluate_logical(&mut self, binary: &BinaryExpression) -> CodegenResult<TypedValue> {
        let left = self.evaluate_with_expected(&binary.left, Some(KomeType::Boolean))?;
        if left.kome_type != KomeType::Boolean {
            return Err(CodegenError::at(
                format!(
                    "logical operator expects bool, but found {}",
                    self.info.type_name(left.kome_type)
                ),
                binary.left.span(),
            ));
        }
        let left = left.expect_value(binary.left.span())?;
        let evaluate_right = self.builder.create_block();
        let short_circuit = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.append_block_param(done, types::I8);
        if binary.op == BinaryOp::And {
            self.builder
                .ins()
                .brif(left, evaluate_right, &[], short_circuit, &[]);
        } else {
            self.builder
                .ins()
                .brif(left, short_circuit, &[], evaluate_right, &[]);
        }
        self.builder.switch_to_block(short_circuit);
        self.builder.seal_block(short_circuit);
        let result = self
            .builder
            .ins()
            .iconst(types::I8, if binary.op == BinaryOp::And { 0 } else { 1 });
        self.builder
            .ins()
            .jump(done, &[ir::BlockArg::Value(result)]);

        self.builder.switch_to_block(evaluate_right);
        self.builder.seal_block(evaluate_right);
        let right = self.evaluate_with_expected(&binary.right, Some(KomeType::Boolean))?;
        if right.kome_type != KomeType::Boolean {
            return Err(CodegenError::at(
                format!(
                    "logical operator expects bool, but found {}",
                    self.info.type_name(right.kome_type)
                ),
                binary.right.span(),
            ));
        }
        self.builder.ins().jump(
            done,
            &[ir::BlockArg::Value(
                right.expect_value(binary.right.span())?,
            )],
        );
        self.builder.seal_block(done);
        self.builder.switch_to_block(done);
        Ok(TypedValue::some(
            self.builder.block_params(done)[0],
            KomeType::Boolean,
        ))
    }

    fn evaluate_task_arguments(
        &mut self,
        name: &str,
        call: &CallExpression,
    ) -> CodegenResult<(Vec<TypedValue>, KomeType)> {
        if call.args.is_empty() {
            return Err(CodegenError::at(
                format!("`{name}` requires at least one Task"),
                call.span,
            ));
        }
        let mut tasks = Vec::with_capacity(call.args.len());
        let mut result_type = None;
        for argument in &call.args {
            let expression = match argument {
                CallArg::Positional(value) => value,
                CallArg::Named { value, .. } => value,
            };
            let task = self.evaluate(expression)?;
            let KomeType::Task(id) = task.kome_type else {
                return Err(CodegenError::at(
                    format!("`{name}` expects Task<T> arguments"),
                    expression.span(),
                ));
            };
            let actual = self.info.task_result(id);
            if let Some(expected) = result_type {
                if expected != actual {
                    return Err(CodegenError::at(
                        format!("`{name}` Task result types must match"),
                        expression.span(),
                    ));
                }
            } else {
                result_type = Some(actual);
            }
            tasks.push(task);
        }
        Ok((tasks, result_type.expect("nonempty task arguments")))
    }

    fn wait_task_value(
        &mut self,
        task: TypedValue,
        result_type: KomeType,
        span: Span,
    ) -> CodegenResult<ir::Value> {
        let handle = task.expect_value(span)?;
        let wait =
            Module::declare_func_in_func(self.module, self.foreign.task_wait, self.builder.func);
        let wait_call = self.builder.ins().call(wait, &[handle]);
        let state = self.builder.inst_results(wait_call)[0];
        self.require_task_completed(state, 0);
        let result =
            Module::declare_func_in_func(self.module, self.foreign.task_result, self.builder.func);
        let call = self.builder.ins().call(result, &[handle]);
        let value = self.task_slot_to_value(self.builder.inst_results(call)[0], result_type);
        if result_type.is_managed() {
            self.retain_managed(value, result_type);
        }
        self.release_owned_temporary(task, span)?;
        Ok(value)
    }

    fn evaluate_all(&mut self, call: &CallExpression) -> CodegenResult<TypedValue> {
        let (tasks, result_type) = self.evaluate_task_arguments("all", call)?;
        let length = self.builder.ins().iconst(types::I64, tasks.len() as i64);
        let handles = self.builder.create_sized_stack_slot(StackSlotData {
            kind: cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            size: (tasks.len() * 8) as StackSize,
            align_shift: 3,
            key: None,
        });
        let buffer =
            self.builder
                .ins()
                .stack_addr(self.module.target_config().pointer_type(), handles, 0);
        for (index, task) in tasks.iter().enumerate() {
            self.builder.ins().store(
                MachMemFlags::new(),
                task.expect_value(call.span)?,
                buffer,
                (index * 8) as i32,
            );
        }
        let all =
            Module::declare_func_in_func(self.module, self.foreign.task_all, self.builder.func);
        let all_call = self.builder.ins().call(all, &[buffer, length]);
        let state = self.builder.inst_results(all_call)[0];
        self.require_task_completed(state, 1);
        let alloc =
            Module::declare_func_in_func(self.module, self.foreign.list_alloc, self.builder.func);
        let allocation = self.builder.ins().call(alloc, &[length]);
        let list = self.builder.inst_results(allocation)[0];
        for (index, task) in tasks.into_iter().enumerate() {
            let value = self.wait_task_value(task, result_type, call.span)?;
            let slot = self.value_to_task_slot(value, result_type);
            self.builder
                .ins()
                .store(MachMemFlags::new(), slot, list, (index * 8) as i32);
        }
        Ok(TypedValue::some(list, self.info.list_type(result_type)))
    }

    fn evaluate_race(&mut self, call: &CallExpression) -> CodegenResult<TypedValue> {
        let (tasks, result_type) = self.evaluate_task_arguments("race", call)?;
        let slot = self.builder.create_sized_stack_slot(StackSlotData {
            kind: cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            size: (tasks.len() * 8) as StackSize,
            align_shift: 3,
            key: None,
        });
        let buffer =
            self.builder
                .ins()
                .stack_addr(self.module.target_config().pointer_type(), slot, 0);
        for (index, task) in tasks.iter().enumerate() {
            self.builder.ins().store(
                MachMemFlags::new(),
                task.expect_value(call.span)?,
                buffer,
                (index * 8) as i32,
            );
        }
        let length = self.builder.ins().iconst(types::I64, tasks.len() as i64);
        let race =
            Module::declare_func_in_func(self.module, self.foreign.task_race, self.builder.func);
        let race_call = self.builder.ins().call(race, &[buffer, length]);
        let winner = self.builder.inst_results(race_call)[0];
        let require = Module::declare_func_in_func(
            self.module,
            self.foreign.task_require_race_winner,
            self.builder.func,
        );
        let require_call = self.builder.ins().call(require, &[winner]);
        let winner = self.builder.inst_results(require_call)[0];
        let offset = self.builder.ins().imul_imm_u(winner, 8);
        let address = self.builder.ins().iadd(buffer, offset);
        let handle = self
            .builder
            .ins()
            .load(types::I64, MachMemFlags::new(), address, 0);
        let result =
            Module::declare_func_in_func(self.module, self.foreign.task_result, self.builder.func);
        let result_call = self.builder.ins().call(result, &[handle]);
        let value = self.task_slot_to_value(self.builder.inst_results(result_call)[0], result_type);
        if result_type.is_managed() {
            self.retain_managed(value, result_type);
        }
        let cancel =
            Module::declare_func_in_func(self.module, self.foreign.task_cancel, self.builder.func);
        for task in tasks {
            self.builder
                .ins()
                .call(cancel, &[task.expect_value(call.span)?]);
            self.release_owned_temporary(task, call.span)?;
        }
        Ok(TypedValue::some(value, result_type))
    }

    fn evaluate_timeout(&mut self, call: &CallExpression) -> CodegenResult<TypedValue> {
        if call.args.len() != 2 {
            return Err(CodegenError::at(
                "`timeout` expects a Task and a millisecond duration",
                call.span,
            ));
        }
        let names = ["task".to_owned(), "milliseconds".to_owned()];
        let defaults = [None, None];
        let ordered = self.order_call_arguments(&call.args, &names, &defaults, 0, call.span)?;
        let task_expression = &ordered[0];
        let duration_expression = &ordered[1];
        let task = self.evaluate(task_expression)?;
        let KomeType::Task(id) = task.kome_type else {
            return Err(CodegenError::at(
                "`timeout` expects Task<T>",
                task_expression.span(),
            ));
        };
        let Expression::Literal(LiteralExpression {
            kind: LiteralKind::Number(duration),
            ..
        }) = duration_expression
        else {
            return Err(CodegenError::at(
                "timeout duration must be a non-negative integer literal in milliseconds",
                duration_expression.span(),
            ));
        };
        let milliseconds = duration.0.parse::<i64>().map_err(|_| {
            CodegenError::at(
                "timeout duration must be a non-negative integer literal in milliseconds",
                duration_expression.span(),
            )
        })?;
        let handle = task.expect_value(task_expression.span())?;
        let duration = self.builder.ins().iconst(types::I64, milliseconds);
        let wait = Module::declare_func_in_func(
            self.module,
            self.foreign.task_wait_timeout,
            self.builder.func,
        );
        let wait_call = self.builder.ins().call(wait, &[handle, duration]);
        let state = self.builder.inst_results(wait_call)[0];
        self.require_task_completed(state, 3);
        let result_type = self.info.task_result(id);
        let result =
            Module::declare_func_in_func(self.module, self.foreign.task_result, self.builder.func);
        let result_call = self.builder.ins().call(result, &[handle]);
        let value = self.task_slot_to_value(self.builder.inst_results(result_call)[0], result_type);
        if result_type.is_managed() {
            self.retain_managed(value, result_type);
        }
        self.release_owned_temporary(task, task_expression.span())?;
        Ok(TypedValue::some(value, result_type))
    }

    fn evaluate_closure_value(&mut self, closure: &ClosureExpression) -> CodegenResult<TypedValue> {
        let lowering = closure.lowering.as_ref().ok_or_else(|| {
            CodegenError::at(
                "closure did not receive a concrete code generation environment",
                closure.span,
            )
        })?;
        let environment_id = *self
            .info
            .struct_ids
            .get(&lowering.environment)
            .ok_or_else(|| {
                CodegenError::at("closure environment layout was not generated", closure.span)
            })?;
        let environment_expression = StructExpression {
            span: closure.span,
            name: lowering.environment.clone(),
            type_arguments: Vec::new(),
            fields: lowering
                .captures
                .iter()
                .map(|capture| KeyValueProperty {
                    span: closure.span,
                    key: PropertyKey::Ident {
                        name: capture.clone(),
                        span: closure.span,
                    },
                    value: Box::new(Expression::Ident(IdentifierExpression {
                        span: closure.span,
                        name: capture.clone(),
                    })),
                })
                .collect(),
        };
        let environment = self.evaluate_struct(&environment_expression)?;
        let environment_pointer = environment.expect_value(closure.span)?;
        let closure_type = type_from_annotation(
            &Type::Function(lowering.function_type.clone()),
            &self.info.runtime_types,
            &self.info.struct_ids,
            &self.info.task_types,
            &self.info.list_types,
            &self.info.optional_types,
            &self.info.closure_types,
        )?;
        let function_id = self
            .func_ids
            .get(&lowering.function)
            .copied()
            .ok_or_else(|| {
                CodegenError::at("closure body function was not generated", closure.span)
            })?;
        let function = Module::declare_func_in_func(self.module, function_id, self.builder.func);
        let code = self
            .builder
            .ins()
            .func_addr(self.module.target_config().pointer_type(), function);
        let drop_id = self
            .closure_drop_ids
            .get(&environment_id)
            .copied()
            .ok_or_else(|| {
                CodegenError::at(
                    "closure environment destructor was not generated",
                    closure.span,
                )
            })?;
        let destructor = Module::declare_func_in_func(self.module, drop_id, self.builder.func);
        let destructor = self
            .builder
            .ins()
            .func_addr(self.module.target_config().pointer_type(), destructor);
        let alloc = Module::declare_func_in_func(
            self.module,
            self.foreign.closure_alloc,
            self.builder.func,
        );
        let call = self
            .builder
            .ins()
            .call(alloc, &[code, environment_pointer, destructor]);
        let closure_pointer = self.builder.inst_results(call)[0];
        Ok(TypedValue::some(closure_pointer, closure_type))
    }

    fn evaluate_call(&mut self, call: &CallExpression) -> CodegenResult<TypedValue> {
        if let Expression::Closure(closure) = call.callee.as_ref() {
            if closure.lowering.is_some() {
                let callee = self.evaluate(&call.callee)?;
                return self.evaluate_closure_indirect(callee, call);
            }
            return self.evaluate_closure_call(closure, call);
        }
        if let Expression::Group(group) = call.callee.as_ref()
            && let Expression::Closure(closure) = group.expression.as_ref()
        {
            if closure.lowering.is_some() {
                let callee = self.evaluate(&group.expression)?;
                return self.evaluate_closure_indirect(callee, call);
            }
            return self.evaluate_closure_call(closure, call);
        }
        if let Expression::Group(group) = call.callee.as_ref() {
            if let Expression::Member(member) = group.expression.as_ref()
                && self.member_is_closure_field(member)
            {
                let callee = self.evaluate(&group.expression)?;
                return self.evaluate_closure_indirect(callee, call);
            }
            let mut ungrouped = call.clone();
            ungrouped.callee = group.expression.clone();
            return self.evaluate_call(&ungrouped);
        }
        if let Expression::Ident(identifier) = call.callee.as_ref() {
            let local_type = self.scopes.iter().rev().find_map(|scope| {
                scope
                    .get(&identifier.name)
                    .map(|variable| variable.kome_type)
            });
            if matches!(local_type, Some(KomeType::Closure(_))) {
                let callee = self.evaluate(&call.callee)?;
                return self.evaluate_closure_indirect(callee, call);
            }
            let local = self
                .closures
                .iter()
                .rev()
                .find(|(name, _, _)| name == &identifier.name)
                .map(|(_, _, closure)| closure.clone());
            if let Some(closure) = local {
                return self.evaluate_closure_call(&closure, call);
            }
            let local_function = self
                .local_functions
                .iter()
                .rev()
                .find(|(name, _, _)| name == &identifier.name)
                .map(|(_, _, function)| function.clone());
            if let Some(function) = local_function {
                return self.evaluate_local_function_call(&function, call);
            }
            let global = self
                .info
                .globals
                .get(&identifier.name)
                .and_then(|global| global.binding.init.as_ref())
                .and_then(|initializer| match initializer {
                    Expression::Closure(closure) => Some(closure.clone()),
                    Expression::Group(group) => match group.expression.as_ref() {
                        Expression::Closure(closure) => Some(closure.clone()),
                        _ => None,
                    },
                    _ => None,
                });
            if let Some(closure) = global {
                return self.evaluate_closure_call(&closure, call);
            }
        }
        if let Expression::Ident(identifier) = call.callee.as_ref() {
            if self.info.components.contains_key(&identifier.name) {
                return self.evaluate_component(&identifier.name, &call.args, &[], call.span);
            }
            match identifier.name.as_str() {
                "all" => return self.evaluate_all(call),
                "race" => return self.evaluate_race(call),
                "timeout" => return self.evaluate_timeout(call),
                _ => {}
            }
        }
        if let Expression::Member(member) = call.callee.as_ref() {
            if self.member_is_closure_field(member) {
                let callee = self.evaluate(&call.callee)?;
                return self.evaluate_closure_indirect(callee, call);
            }
            return self.evaluate_method_call(member, call);
        }

        let Expression::Ident(callee) = call.callee.as_ref() else {
            let callee = self.evaluate(&call.callee)?;
            return self.evaluate_closure_indirect(callee, call);
        };

        let plan = match self.info.get(&callee.name) {
            None => {
                return Err(CodegenError::at(
                    format!("function `{}` was not found", callee.name),
                    callee.span,
                ));
            }

            Some(FunctionKind::User { .. }) => CalleePlan::User,

            Some(FunctionKind::External { .. }) => CalleePlan::External,

            Some(FunctionKind::Native { symbol, .. }) => CalleePlan::Native {
                symbol: (*symbol).to_owned(),
            },
        };

        // Fetch the signature again through a cloned snapshot so that no
        // borrow of `self.info` survives into argument evaluation.
        let signature = self
            .info
            .get(&callee.name)
            .expect("callee existence was checked above")
            .signature()
            .clone();
        let ordered = self.order_call_arguments(
            &call.args,
            &signature.param_names,
            &signature.defaults,
            0,
            call.span,
        )?;

        let mut arguments = Vec::with_capacity(call.args.len());
        let mut argument_values = Vec::with_capacity(call.args.len());

        for (expression, param_type) in ordered.into_iter().zip(&signature.params) {
            let typed = self.evaluate_with_expected(&expression, Some(*param_type))?;

            if typed.kome_type != *param_type {
                return Err(CodegenError::at(
                    format!(
                        "argument for `{}` expects {}, but the expression has type {}",
                        callee.name,
                        param_type.name(),
                        typed.kome_type.name(),
                    ),
                    expression.span(),
                ));
            }

            arguments.push(typed.expect_value(expression.span())?);
            argument_values.push(typed);
        }

        let result = match plan {
            CalleePlan::User | CalleePlan::External => {
                self.emit_user_call(&callee.name, &arguments, &signature)?
            }
            CalleePlan::Native { symbol } => {
                self.emit_native_call(&symbol, &arguments, &signature)?
            }
        };

        for argument in argument_values {
            self.release_owned_temporary(argument, call.span)?;
        }

        Ok(result)
    }

    fn member_is_closure_field(&self, member: &MemberExpression) -> bool {
        let Expression::Ident(identifier) = member.object.as_ref() else {
            return false;
        };
        let Some(KomeType::Struct(id)) = self.scopes.iter().rev().find_map(|scope| {
            scope
                .get(&identifier.name)
                .map(|variable| variable.kome_type)
        }) else {
            return false;
        };
        self.info.struct_info(id).fields.iter().any(|field| {
            field.name == member.property && matches!(field.kome_type, KomeType::Closure(_))
        })
    }

    fn evaluate_closure_indirect(
        &mut self,
        closure: TypedValue,
        call: &CallExpression,
    ) -> CodegenResult<TypedValue> {
        let KomeType::Closure(id) = closure.kome_type else {
            return Err(CodegenError::at(
                "expression is not callable",
                call.callee.span(),
            ));
        };
        let signature = self.info.closure_signature(id);
        let ordered = self.order_call_arguments(
            &call.args,
            &signature.param_names,
            &signature.defaults,
            0,
            call.span,
        )?;
        let mut arguments = Vec::with_capacity(ordered.len() + 1);
        let mut typed_arguments = Vec::with_capacity(ordered.len());
        let closure_pointer = closure.expect_value(call.callee.span())?;
        let code_function =
            Module::declare_func_in_func(self.module, self.foreign.closure_code, self.builder.func);
        let code_call = self.builder.ins().call(code_function, &[closure_pointer]);
        let code = self.builder.inst_results(code_call)[0];
        let environment_function = Module::declare_func_in_func(
            self.module,
            self.foreign.closure_environment,
            self.builder.func,
        );
        let environment_call = self
            .builder
            .ins()
            .call(environment_function, &[closure_pointer]);
        let environment = self.builder.inst_results(environment_call)[0];
        arguments.push(environment);
        for (expression, expected) in ordered.into_iter().zip(&signature.params) {
            let value = self.evaluate_with_expected(&expression, Some(*expected))?;
            if value.kome_type != *expected {
                return Err(CodegenError::at(
                    format!(
                        "closure argument expects {}, but found {}",
                        self.info.type_name(*expected),
                        self.info.type_name(value.kome_type)
                    ),
                    expression.span(),
                ));
            }
            arguments.push(value.expect_value(expression.span())?);
            typed_arguments.push(value);
        }
        let mut native_signature = self.module.make_signature();
        native_signature.params.push(AbiParam::new(types::I64));
        for parameter in &signature.params {
            native_signature.params.push(AbiParam::new(
                parameter
                    .cranelift()
                    .expect("closure parameters cannot have type Void"),
            ));
        }
        if let Some(result) = signature.ret.cranelift() {
            native_signature.returns.push(AbiParam::new(result));
        }
        let signature_ref = self.builder.import_signature(native_signature);
        let invocation = self
            .builder
            .ins()
            .call_indirect(signature_ref, code, &arguments);
        for argument in typed_arguments {
            self.release_owned_temporary(argument, call.span)?;
        }
        self.release_owned_temporary(closure, call.callee.span())?;
        if signature.ret == KomeType::Void {
            Ok(TypedValue::void())
        } else {
            Ok(TypedValue::some(
                self.builder.inst_results(invocation)[0],
                signature.ret,
            ))
        }
    }

    fn evaluate_component(
        &mut self,
        name: &str,
        arguments: &[CallArg],
        children: &[Expression],
        span: Span,
    ) -> CodegenResult<TypedValue> {
        let component =
            self.info.components.get(name).cloned().ok_or_else(|| {
                CodegenError::at(format!("component `{name}` was not found"), span)
            })?;
        let ordered = self.order_call_arguments(
            arguments,
            &component.param_names,
            &component.defaults,
            0,
            span,
        )?;
        let previous_moves = self.allow_managed_moves;
        self.allow_managed_moves = false;
        self.scopes.push(HashMap::new());
        let component_scope = self.scopes.len();
        for ((expression, expected), parameter_name) in ordered
            .iter()
            .zip(&component.param_types)
            .zip(&component.param_names)
        {
            let value = self.evaluate_with_expected(expression, Some(*expected))?;
            if value.kome_type != *expected {
                return Err(CodegenError::at(
                    format!(
                        "component `{name}` parameter expects {}, but found {}",
                        self.info.type_name(*expected),
                        self.info.type_name(value.kome_type)
                    ),
                    expression.span(),
                ));
            }
            let raw = value.expect_value(expression.span())?;
            self.declare_variable(parameter_name, raw, *expected, value.ownership)?;
        }
        if let Some(body) = &component.body {
            for member in body {
                match member {
                    ComponentMember::State(binding) | ComponentMember::Let(binding) => {
                        self.translate_statement(&Statement::Let(binding.as_ref().clone()))?;
                    }
                    ComponentMember::Function(function) => self.local_functions.push((
                        function.name.clone(),
                        component_scope,
                        function.clone(),
                    )),
                    ComponentMember::Recipe(_) => {}
                }
            }
        }
        if let Some(body) = &component.body {
            for member in body {
                let ComponentMember::Recipe(recipe) = member else {
                    continue;
                };
                if recipe
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "beforeChildren")
                {
                    self.translate_block(&recipe.body)?;
                }
            }
        }
        for child in children {
            let value = self.evaluate(child)?;
            self.release_owned_temporary(value, child.span())?;
        }
        if let Some(body) = &component.body {
            for member in body {
                let ComponentMember::Recipe(recipe) = member else {
                    continue;
                };
                let startup = recipe
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "startup");
                let child_hook = recipe.attributes.iter().any(|attribute| {
                    attribute.name == "beforeChildren" || attribute.name == "afterChildren"
                });
                if !child_hook && (recipe.name == "view" || startup) {
                    self.translate_block(&recipe.body)?;
                }
            }
        }
        if let Some(body) = &component.body {
            for member in body {
                let ComponentMember::Recipe(recipe) = member else {
                    continue;
                };
                if recipe
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "afterChildren")
                {
                    self.translate_block(&recipe.body)?;
                }
            }
        }
        let scope = self.scopes.pop().expect("component scope exists");
        for variable in scope.values() {
            if variable.kome_type.is_managed() && variable.owns_value {
                let value = self.builder.use_var(variable.variable);
                self.release_managed(value, variable.kome_type);
            }
        }
        self.closures
            .retain(|(_, depth, _)| *depth < component_scope);
        self.local_functions
            .retain(|(_, depth, _)| *depth < component_scope);
        self.allow_managed_moves = previous_moves;
        Ok(TypedValue::some(
            self.builder.ins().iconst(types::I8, 0),
            KomeType::Null,
        ))
    }

    fn evaluate_closure_call(
        &mut self,
        closure: &kome_ast::expressions::ClosureExpression,
        call: &CallExpression,
    ) -> CodegenResult<TypedValue> {
        let mut names = Vec::with_capacity(closure.params.len());
        let mut defaults = Vec::with_capacity(closure.params.len());
        for parameter in &closure.params {
            let kome_ast::patterns::Pattern::Ident(identifier) = parameter else {
                return Err(CodegenError::at(
                    "closure parameters require identifier patterns",
                    parameter.span(),
                ));
            };
            names.push(identifier.name.clone());
            defaults.push(identifier.default.as_deref().cloned());
        }
        let ordered = self.order_call_arguments(&call.args, &names, &defaults, 0, call.span)?;
        let mut evaluated = Vec::with_capacity(ordered.len());
        for (expression, parameter) in ordered.iter().zip(&closure.params) {
            let kome_ast::patterns::Pattern::Ident(identifier) = parameter else {
                unreachable!();
            };
            let expected = identifier
                .type_annotation
                .as_ref()
                .map(|annotation| {
                    type_from_annotation(
                        annotation,
                        &self.info.runtime_types,
                        &self.info.struct_ids,
                        &self.info.task_types,
                        &self.info.list_types,
                        &self.info.optional_types,
                        &self.info.closure_types,
                    )
                })
                .transpose()?;
            let value = self.evaluate_with_expected(expression, expected)?;
            if let Some(expected) = expected
                && value.kome_type != expected
            {
                return Err(CodegenError::at(
                    format!(
                        "closure parameter `{}` expects {}, but found {}",
                        identifier.name,
                        self.info.type_name(expected),
                        self.info.type_name(value.kome_type)
                    ),
                    expression.span(),
                ));
            }
            evaluated.push(value);
        }
        self.scopes.push(HashMap::new());
        for (parameter, value) in closure.params.iter().zip(evaluated) {
            let kome_ast::patterns::Pattern::Ident(identifier) = parameter else {
                unreachable!();
            };
            let raw = self.own_value(value, identifier.span)?;
            self.declare_variable(
                &identifier.name,
                raw,
                value.kome_type,
                ValueOwnership::Owned,
            )?;
        }
        let mut result = self.evaluate(&closure.body)?;
        if result.kome_type.is_managed() {
            let raw = self.own_value(result, closure.body.span())?;
            result = TypedValue::some(raw, result.kome_type);
        }
        let scope = self.scopes.pop().expect("closure call scope exists");
        for scoped in scope.values() {
            if scoped.kome_type.is_managed() && scoped.owns_value {
                let value = self.builder.use_var(scoped.variable);
                self.release_managed(value, scoped.kome_type);
            }
        }
        Ok(result)
    }

    fn evaluate_local_function_call(
        &mut self,
        declaration: &FunctionDeclaration,
        call: &CallExpression,
    ) -> CodegenResult<TypedValue> {
        if self
            .local_call_stack
            .iter()
            .any(|name| name == &declaration.name)
        {
            return Err(CodegenError::at(
                format!(
                    "recursive component function `{}` requires a runtime closure environment",
                    declaration.name
                ),
                call.span,
            ));
        }
        self.local_call_stack.push(declaration.name.clone());
        let result = (|| {
            let signature = analyze_signature(
                declaration,
                &self.info.runtime_types,
                &self.info.struct_ids,
                &self.info.task_types,
                &self.info.list_types,
                &self.info.optional_types,
                &self.info.closure_types,
                None,
            )?;
            let body = declaration.body.as_ref().ok_or_else(|| {
                CodegenError::at(
                    format!("component function `{}` has no body", declaration.name),
                    declaration.span,
                )
            })?;
            let ordered = self.order_call_arguments(
                &call.args,
                &signature.param_names,
                &signature.defaults,
                0,
                call.span,
            )?;
            let mut arguments = Vec::with_capacity(ordered.len());
            for (expression, expected) in ordered.iter().zip(&signature.params) {
                let value = self.evaluate_with_expected(expression, Some(*expected))?;
                if value.kome_type != *expected {
                    return Err(CodegenError::at(
                        format!(
                            "component function `{}` expects {}, but found {}",
                            declaration.name,
                            self.info.type_name(*expected),
                            self.info.type_name(value.kome_type)
                        ),
                        expression.span(),
                    ));
                }
                arguments.push(value);
            }

            let done = self.builder.create_block();
            if let Some(representation) = signature.ret.cranelift() {
                self.builder.append_block_param(done, representation);
            }
            let scope_depth = self.scopes.len();
            self.scopes.push(HashMap::new());
            for ((pattern, value), expected) in declaration
                .params
                .iter()
                .zip(arguments)
                .zip(&signature.params)
            {
                let kome_ast::patterns::Pattern::Ident(identifier) = pattern else {
                    return Err(CodegenError::at(
                        "component function parameters require identifier patterns",
                        pattern.span(),
                    ));
                };
                let raw = value.expect_value(identifier.span)?;
                self.declare_variable(&identifier.name, raw, *expected, value.ownership)?;
            }
            self.inline_returns.push(InlineReturnContext {
                target: done,
                return_type: signature.ret,
                scope_depth,
            });
            let outer_terminated = self.terminated;
            self.terminated = false;
            self.translate_block(body)?;
            if !self.terminated {
                self.release_owned_scopes_from(scope_depth);
                if signature.ret == KomeType::Void {
                    self.builder.ins().jump(done, &[]);
                } else {
                    let value = self.zero_value(signature.ret)?;
                    self.builder.ins().jump(done, &[ir::BlockArg::Value(value)]);
                }
            }
            self.inline_returns.pop();
            self.scopes.pop().expect("component function scope exists");
            self.builder.seal_block(done);
            self.builder.switch_to_block(done);
            self.terminated = outer_terminated;
            if signature.ret == KomeType::Void {
                Ok(TypedValue::void())
            } else {
                Ok(TypedValue::some(
                    self.builder.block_params(done)[0],
                    signature.ret,
                ))
            }
        })();
        self.local_call_stack.pop();
        result
    }

    fn evaluate_method_call(
        &mut self,
        member: &MemberExpression,
        call: &CallExpression,
    ) -> CodegenResult<TypedValue> {
        let static_target = if let Expression::Ident(identifier) = member.object.as_ref()
            && !self
                .scopes
                .iter()
                .any(|scope| scope.contains_key(&identifier.name))
        {
            self.info
                .struct_ids
                .get(&identifier.name)
                .copied()
                .map(KomeType::Struct)
                .or_else(|| self.info.runtime_types.get(&identifier.name).copied())
        } else {
            None
        };

        let mut evaluated = Vec::new();
        let target = if let Some(target) = static_target {
            target
        } else {
            let receiver = self.evaluate(&member.object)?;
            let target = receiver.kome_type;
            evaluated.push(receiver);
            target
        };
        if target == KomeType::Null && static_target.is_none() {
            for argument in &call.args {
                let expression = match argument {
                    CallArg::Positional(expression) => expression,
                    CallArg::Named { value, .. } => value,
                };
                if matches!(expression, Expression::DotIdent(_)) {
                    continue;
                }
                let value = self.evaluate(expression)?;
                self.release_owned_temporary(value, expression.span())?;
            }
            for value in evaluated {
                self.release_owned_temporary(value, member.object.span())?;
            }
            return Ok(TypedValue::some(
                self.builder.ins().iconst(types::I8, 0),
                KomeType::Null,
            ));
        }
        let (implementation, method, _) = self
            .info
            .implementation_member(target, &member.property)
            .ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "type `{}` has no method `{}`",
                        self.info.type_name(target),
                        member.property
                    ),
                    member.span,
                )
            })?;
        if implementation.trait_name.as_deref() == Some("Drop") {
            return Err(CodegenError::at(
                "destructor `drop` cannot be called directly",
                call.span,
            ));
        }
        let method = method.cloned().ok_or_else(|| {
            CodegenError::at(
                format!(
                    "`{}` is an associated constant and cannot be called",
                    member.property
                ),
                member.span,
            )
        })?;
        if method.has_self == static_target.is_some() {
            let kind = if method.has_self {
                "instance"
            } else {
                "static"
            };
            return Err(CodegenError::at(
                format!("method `{}` is {kind}", member.property),
                member.span,
            ));
        }
        let mut arguments = Vec::with_capacity(method.signature.params.len());
        if let Some(receiver) = evaluated.first() {
            arguments.push(receiver.expect_value(member.object.span())?);
        }
        let skip = usize::from(method.has_self);
        let ordered = self.order_call_arguments(
            &call.args,
            &method.signature.param_names,
            &method.signature.defaults,
            skip,
            call.span,
        )?;
        for (expression, expected) in ordered
            .into_iter()
            .zip(method.signature.params.iter().skip(skip))
        {
            let typed = self.evaluate_with_expected(&expression, Some(*expected))?;
            if typed.kome_type != *expected {
                return Err(CodegenError::at(
                    format!(
                        "argument for `{}` expects {}, but received {}",
                        member.property,
                        self.info.type_name(*expected),
                        self.info.type_name(typed.kome_type)
                    ),
                    expression.span(),
                ));
            }
            arguments.push(typed.expect_value(expression.span())?);
            evaluated.push(typed);
        }
        let result = self.emit_user_call(&method.function_key, &arguments, &method.signature)?;
        for value in evaluated {
            self.release_owned_temporary(value, call.span)?;
        }
        Ok(result)
    }

    fn order_call_arguments(
        &self,
        arguments: &[CallArg],
        parameter_names: &[String],
        defaults: &[Option<Expression>],
        skip: usize,
        span: Span,
    ) -> CodegenResult<Vec<Expression>> {
        let parameter_names = &parameter_names[skip..];
        let defaults = &defaults[skip..];
        let mut ordered = vec![None; parameter_names.len()];
        let mut next_positional = 0;
        for argument in arguments {
            let (index, expression, argument_span) = match argument {
                CallArg::Positional(expression) => {
                    while ordered.get(next_positional).is_some_and(Option::is_some) {
                        next_positional += 1;
                    }
                    (next_positional, expression.clone(), expression.span())
                }
                CallArg::Named { name, value, span } => {
                    let index = parameter_names
                        .iter()
                        .position(|parameter| parameter == name)
                        .ok_or_else(|| {
                            CodegenError::at(format!("unknown named argument `{name}`"), *span)
                        })?;
                    (index, value.as_ref().clone(), *span)
                }
            };
            let Some(slot) = ordered.get_mut(index) else {
                return Err(CodegenError::at("too many call arguments", argument_span));
            };
            if slot.is_some() {
                return Err(CodegenError::at(
                    format!(
                        "argument `{}` was supplied more than once",
                        parameter_names[index]
                    ),
                    argument_span,
                ));
            }
            *slot = Some(expression);
            if index == next_positional {
                next_positional += 1;
            }
        }
        for (index, slot) in ordered.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = defaults[index].clone();
            }
            if slot.is_none() {
                return Err(CodegenError::at(
                    format!("missing argument `{}`", parameter_names[index]),
                    span,
                ));
            }
        }
        Ok(ordered.into_iter().map(Option::unwrap).collect())
    }

    fn evaluate_assign(&mut self, assignment: &AssignmentExpression) -> CodegenResult<TypedValue> {
        if let Expression::Member(member) = assignment.target.as_ref() {
            return self.evaluate_member_assign(member, assignment);
        }
        if let Expression::Index(index) = assignment.target.as_ref() {
            return self.evaluate_index_assign(index, assignment);
        }
        let Expression::Ident(identifier) = assignment.target.as_ref() else {
            return Err(CodegenError::at(
                "assignment target must be an identifier",
                assignment.target.span(),
            ));
        };

        let Some(scope) = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(&identifier.name))
        else {
            return self.evaluate_global_assign(identifier, assignment);
        };

        let scoped = self.scopes[scope][&identifier.name];
        let typed = self.evaluate_with_expected(&assignment.value, Some(scoped.kome_type))?;

        if typed.kome_type != scoped.kome_type {
            return Err(CodegenError::at(
                format!(
                    "cannot assign {} to variable `{}` of type {}",
                    typed.kome_type.name(),
                    identifier.name,
                    scoped.kome_type.name(),
                ),
                assignment.value.span(),
            ));
        }

        match assignment.op {
            AssignOp::Assign => {
                let value = self.own_value(typed, assignment.value.span())?;

                if scoped.kome_type.is_managed() && self.scopes[scope][&identifier.name].owns_value
                {
                    let old = self.builder.use_var(scoped.variable);
                    self.release_managed(old, scoped.kome_type);
                }

                self.builder.def_var(scoped.variable, value);

                if scoped.kome_type.is_managed() {
                    self.scopes[scope]
                        .get_mut(&identifier.name)
                        .unwrap()
                        .owns_value = true;
                }

                Ok(TypedValue::borrowed(value, scoped.kome_type))
            }
            AssignOp::AddAssign => {
                let old = self.builder.use_var(scoped.variable);
                let value = self.assignment_value(old, typed, assignment)?;
                if scoped.kome_type.is_managed() && scoped.owns_value {
                    self.release_managed(old, scoped.kome_type);
                }
                self.builder.def_var(scoped.variable, value);
                if scoped.kome_type.is_managed() {
                    self.scopes[scope]
                        .get_mut(&identifier.name)
                        .unwrap()
                        .owns_value = true;
                }
                Ok(TypedValue::borrowed(value, scoped.kome_type))
            }
        }
    }

    fn evaluate_global_assign(
        &mut self,
        identifier: &IdentifierExpression,
        assignment: &AssignmentExpression,
    ) -> CodegenResult<TypedValue> {
        let global = self
            .info
            .globals
            .get(&identifier.name)
            .cloned()
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;
        if !global.binding.mutable {
            return Err(CodegenError::at(
                format!("cannot assign to immutable global `{}`", identifier.name),
                identifier.span,
            ));
        }
        let old = self.evaluate_global(identifier)?;
        let kome_type = old.kome_type;
        let old_value = old.expect_value(identifier.span)?;
        let right = self.evaluate_with_expected(&assignment.value, Some(kome_type))?;
        if right.kome_type != kome_type {
            return Err(CodegenError::at(
                format!(
                    "cannot assign {} to global `{}` of type {}",
                    self.info.type_name(right.kome_type),
                    identifier.name,
                    self.info.type_name(kome_type)
                ),
                assignment.value.span(),
            ));
        }
        let value = self.assignment_value(old_value, right, assignment)?;
        if kome_type.is_managed() {
            self.release_managed(old_value, kome_type);
        }
        let storage = self.global_storage[&identifier.name];
        let data = Module::declare_data_in_func(self.module, storage.value, self.builder.func);
        let pointer = self
            .builder
            .ins()
            .symbol_value(self.module.target_config().pointer_type(), data);
        self.builder
            .ins()
            .store(MachMemFlags::new(), value, pointer, 0);
        if kome_type.is_managed() {
            self.retain_managed(value, kome_type);
        }
        Ok(TypedValue::some(value, kome_type))
    }

    fn evaluate_member_assign(
        &mut self,
        member: &MemberExpression,
        assignment: &AssignmentExpression,
    ) -> CodegenResult<TypedValue> {
        let object = self.evaluate(&member.object)?;
        let KomeType::Struct(id) = object.kome_type else {
            return Err(CodegenError::at(
                "field assignment requires a struct value",
                member.object.span(),
            ));
        };
        let field = self
            .info
            .struct_info(id)
            .fields
            .iter()
            .find(|field| field.name == member.property)
            .cloned()
            .ok_or_else(|| {
                CodegenError::at(
                    format!(
                        "struct `{}` has no field `{}`",
                        self.info.struct_info(id).name,
                        member.property
                    ),
                    member.span,
                )
            })?;
        let pointer = object.expect_value(member.object.span())?;
        let old = self.builder.ins().load(
            field.kome_type.cranelift().expect("field representation"),
            MachMemFlags::new(),
            pointer,
            field.offset,
        );
        let right = self.evaluate_with_expected(&assignment.value, Some(field.kome_type))?;
        if right.kome_type != field.kome_type {
            return Err(CodegenError::at(
                format!(
                    "field `{}` expects {}, but found {}",
                    field.name,
                    self.info.type_name(field.kome_type),
                    self.info.type_name(right.kome_type)
                ),
                assignment.value.span(),
            ));
        }
        let value = self.assignment_value(old, right, assignment)?;
        if field.kome_type.is_managed() {
            self.release_managed(old, field.kome_type);
        }
        self.builder
            .ins()
            .store(MachMemFlags::new(), value, pointer, field.offset);
        if field.kome_type.is_managed() {
            self.retain_managed(value, field.kome_type);
        }
        self.release_owned_temporary(object, member.object.span())?;
        Ok(TypedValue::some(value, field.kome_type))
    }

    fn evaluate_index_assign(
        &mut self,
        index: &kome_ast::expressions::IndexExpression,
        assignment: &AssignmentExpression,
    ) -> CodegenResult<TypedValue> {
        let object = self.evaluate(&index.object)?;
        if let KomeType::Struct(id) = object.kome_type
            && self.info.struct_info(id).anonymous
        {
            let key = static_index_name(&index.index).ok_or_else(|| {
                CodegenError::at(
                    "structural object indices must be string or number literals",
                    index.index.span(),
                )
            })?;
            let layout = self.info.struct_info(id);
            let field = layout
                .fields
                .iter()
                .find(|field| field.name == key)
                .cloned()
                .ok_or_else(|| {
                    CodegenError::at(format!("object has no property `{key}`"), index.span)
                })?;
            let pointer = object.expect_value(index.object.span())?;
            let old = self.builder.ins().load(
                field.kome_type.cranelift().expect("field representation"),
                MachMemFlags::new(),
                pointer,
                field.offset,
            );
            let right = self.evaluate_with_expected(&assignment.value, Some(field.kome_type))?;
            if right.kome_type != field.kome_type {
                return Err(CodegenError::at(
                    format!(
                        "object property `{key}` expects {}, but found {}",
                        self.info.type_name(field.kome_type),
                        self.info.type_name(right.kome_type)
                    ),
                    assignment.value.span(),
                ));
            }
            let value = self.assignment_value(old, right, assignment)?;
            if field.kome_type.is_managed() {
                self.release_managed(old, field.kome_type);
            }
            self.builder
                .ins()
                .store(MachMemFlags::new(), value, pointer, field.offset);
            if field.kome_type.is_managed() {
                self.retain_managed(value, field.kome_type);
            }
            self.release_owned_temporary(object, index.object.span())?;
            return Ok(TypedValue::some(value, field.kome_type));
        }
        let KomeType::List(id) = object.kome_type else {
            return Err(CodegenError::at(
                "index assignment requires a List",
                index.object.span(),
            ));
        };
        let typed_index = self.evaluate(&index.index)?;
        let raw_index = typed_index.expect_value(index.index.span())?;
        let index_value = match typed_index.kome_type {
            KomeType::Number => {
                let convert = Module::declare_func_in_func(
                    self.module,
                    self.foreign.number_to_i64,
                    self.builder.func,
                );
                let call = self.builder.ins().call(convert, &[raw_index]);
                self.builder.inst_results(call)[0]
            }
            KomeType::I8 | KomeType::I16 | KomeType::I32 => {
                self.builder.ins().sextend(types::I64, raw_index)
            }
            KomeType::U8 | KomeType::U16 | KomeType::U32 => {
                self.builder.ins().uextend(types::I64, raw_index)
            }
            KomeType::I64 | KomeType::U64 => raw_index,
            other => {
                return Err(CodegenError::at(
                    format!(
                        "list index must be an integer, but found {}",
                        self.info.type_name(other)
                    ),
                    index.index.span(),
                ));
            }
        };
        let pointer = object.expect_value(index.object.span())?;
        let require = Module::declare_func_in_func(
            self.module,
            self.foreign.list_require_index,
            self.builder.func,
        );
        let call = self.builder.ins().call(require, &[pointer, index_value]);
        let checked_index = self.builder.inst_results(call)[0];
        let offset = self.builder.ins().imul_imm_u(checked_index, 8);
        let address = self.builder.ins().iadd(pointer, offset);
        let element_type = self.info.list_element(id);
        let old_slot = self
            .builder
            .ins()
            .load(types::I64, MachMemFlags::new(), address, 0);
        let old = self.task_slot_to_value(old_slot, element_type);
        let right = self.evaluate_with_expected(&assignment.value, Some(element_type))?;
        if right.kome_type != element_type {
            return Err(CodegenError::at(
                format!(
                    "list element expects {}, but found {}",
                    self.info.type_name(element_type),
                    self.info.type_name(right.kome_type)
                ),
                assignment.value.span(),
            ));
        }
        let value = self.assignment_value(old, right, assignment)?;
        if element_type.is_managed() {
            self.release_managed(old, element_type);
        }
        let slot = self.value_to_task_slot(value, element_type);
        self.builder
            .ins()
            .store(MachMemFlags::new(), slot, address, 0);
        if element_type.is_managed() {
            self.retain_managed(value, element_type);
        }
        self.release_owned_temporary(typed_index, index.index.span())?;
        self.release_owned_temporary(object, index.object.span())?;
        Ok(TypedValue::some(value, element_type))
    }

    fn assignment_value(
        &mut self,
        old: ir::Value,
        right: TypedValue,
        assignment: &AssignmentExpression,
    ) -> CodegenResult<ir::Value> {
        if assignment.op == AssignOp::Assign {
            return self.own_value(right, assignment.value.span());
        }
        let raw = right.expect_value(assignment.value.span())?;
        let value = match right.kome_type {
            KomeType::Number => self.emit_number_binary(self.foreign.number_add, old, raw),
            KomeType::String => self.emit_number_binary(self.foreign.string_concat, old, raw),
            KomeType::I8
            | KomeType::I16
            | KomeType::I32
            | KomeType::I64
            | KomeType::U8
            | KomeType::U16
            | KomeType::U32
            | KomeType::U64 => self.builder.ins().iadd(old, raw),
            KomeType::F32 | KomeType::F64 => self.builder.ins().fadd(old, raw),
            other => {
                return Err(CodegenError::at(
                    format!(
                        "compound assignment requires a numeric or String value, but found {}",
                        self.info.type_name(other)
                    ),
                    assignment.span,
                ));
            }
        };
        self.release_owned_temporary(right, assignment.value.span())?;
        Ok(value)
    }

    fn emit_user_call(
        &mut self,
        function_key: &str,
        arguments: &[ir::Value],
        signature: &FunctionSignature,
    ) -> CodegenResult<TypedValue> {
        let func_id = self.func_ids[function_key];

        let func_ref = Module::declare_func_in_func(self.module, func_id, self.builder.func);

        let call = self.builder.ins().call(func_ref, arguments);

        if signature.ret == KomeType::Void {
            return Ok(TypedValue::void());
        }

        Ok(TypedValue::some(
            self.builder.inst_results(call)[0],
            signature.ret,
        ))
    }

    /// Marshals arguments into `{ tag, payload }` slots and dispatches
    /// through `__kome_native_call`.
    fn emit_native_call(
        &mut self,
        symbol: &str,
        arguments: &[ir::Value],
        signature: &FunctionSignature,
    ) -> CodegenResult<TypedValue> {
        const SLOT_SIZE: i64 = 16;

        let slot = self.builder.create_sized_stack_slot(StackSlotData {
            kind: cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
            size: (SLOT_SIZE * arguments.len() as i64) as StackSize,
            align_shift: 3,
            key: None,
        });

        let buffer =
            self.builder
                .ins()
                .stack_addr(self.module.target_config().pointer_type(), slot, 0);

        for (index, (value, param_type)) in arguments.iter().zip(&signature.params).enumerate() {
            let offset = SLOT_SIZE * index as i64;

            let tag_address = self.builder.ins().iadd_imm_s(buffer, offset);
            let tag = self.builder.ins().iconst(types::I64, param_type.tag()?);

            self.builder
                .ins()
                .store(MachMemFlags::new(), tag, tag_address, 0);

            let payload = match param_type {
                KomeType::Number | KomeType::String | KomeType::Socket | KomeType::Listener => {
                    *value
                }

                KomeType::Boolean | KomeType::Null => {
                    self.builder.ins().uextend(types::I64, *value)
                }
                KomeType::I8 | KomeType::I16 | KomeType::I32 => {
                    self.builder.ins().sextend(types::I64, *value)
                }
                KomeType::U8 | KomeType::U16 | KomeType::U32 => {
                    self.builder.ins().uextend(types::I64, *value)
                }
                KomeType::I64 | KomeType::U64 | KomeType::Isize | KomeType::Usize => *value,
                KomeType::F32 => {
                    let bits = self
                        .builder
                        .ins()
                        .bitcast(types::I32, MachMemFlags::new(), *value);
                    self.builder.ins().uextend(types::I64, bits)
                }
                KomeType::F64 => {
                    self.builder
                        .ins()
                        .bitcast(types::I64, MachMemFlags::new(), *value)
                }

                KomeType::Void => {
                    return Err(CodegenError::new("parameters cannot have type Void", None));
                }

                KomeType::Pointer
                | KomeType::Struct(_)
                | KomeType::Task(_)
                | KomeType::List(_)
                | KomeType::Enum(_)
                | KomeType::Optional(_)
                | KomeType::Closure(_) => {
                    return Err(CodegenError::new(
                        "managed aggregate values cannot cross the native ABI",
                        None,
                    ));
                }
            };

            let payload_address = self.builder.ins().iadd_imm_s(buffer, offset + 8);

            self.builder
                .ins()
                .store(MachMemFlags::new(), payload, payload_address, 0);
        }

        let name_pointer = self.c_string_pointer(symbol)?;

        let argc = self
            .builder
            .ins()
            .iconst(types::I64, arguments.len() as i64);

        let return_tag = match signature.ret {
            KomeType::Optional(id) => kome_abi::optional_tag(self.info.optional_inner(id).tag()?),
            other => other.tag()?,
        };
        let ret_tag = self.builder.ins().iconst(types::I64, return_tag);

        let func_ref =
            Module::declare_func_in_func(self.module, self.foreign.native_call, self.builder.func);

        let call = self
            .builder
            .ins()
            .call(func_ref, &[name_pointer, argc, buffer, ret_tag]);

        let payload = self.builder.inst_results(call)[0];

        let value = match signature.ret {
            KomeType::Void => {
                return Ok(TypedValue::void());
            }

            KomeType::Number | KomeType::String | KomeType::Socket | KomeType::Listener => payload,

            KomeType::Boolean | KomeType::Null => self.builder.ins().ireduce(types::I8, payload),

            KomeType::I8 | KomeType::U8 => self.builder.ins().ireduce(types::I8, payload),
            KomeType::I16 | KomeType::U16 => self.builder.ins().ireduce(types::I16, payload),
            KomeType::I32 | KomeType::U32 => self.builder.ins().ireduce(types::I32, payload),
            KomeType::I64 | KomeType::U64 | KomeType::Isize | KomeType::Usize => payload,
            KomeType::F32 => {
                let bits = self.builder.ins().ireduce(types::I32, payload);
                self.builder
                    .ins()
                    .bitcast(types::F32, MachMemFlags::new(), bits)
            }
            KomeType::F64 => self
                .builder
                .ins()
                .bitcast(types::F64, MachMemFlags::new(), payload),

            KomeType::Optional(_) => payload,

            KomeType::Pointer
            | KomeType::Struct(_)
            | KomeType::Task(_)
            | KomeType::List(_)
            | KomeType::Enum(_)
            | KomeType::Closure(_) => {
                return Err(CodegenError::new(
                    "managed aggregate values cannot cross the native ABI",
                    None,
                ));
            }
        };

        Ok(TypedValue::some(value, signature.ret))
    }

    fn zero_value(&mut self, kome_type: KomeType) -> CodegenResult<ir::Value> {
        match kome_type {
            KomeType::Number => self
                .evaluate_literal(&LiteralExpression {
                    span: Span::new(0, 0),
                    kind: LiteralKind::Number(NumberLiteral("0".into())),
                })?
                .expect_value(Span::new(0, 0)),
            KomeType::String => self
                .evaluate_literal(&LiteralExpression {
                    span: Span::new(0, 0),
                    kind: LiteralKind::String(String::new()),
                })?
                .expect_value(Span::new(0, 0)),
            KomeType::Socket | KomeType::Listener => Err(CodegenError::new(
                "socket values cannot be used without an initializer",
                None,
            )),
            KomeType::F64 => Ok(self.builder.ins().f64const(0.0)),
            KomeType::F32 => Ok(self.builder.ins().f32const(0.0)),
            KomeType::Boolean | KomeType::I8 | KomeType::U8 | KomeType::Null => {
                Ok(self.builder.ins().iconst(types::I8, 0))
            }
            KomeType::I16 | KomeType::U16 => Ok(self.builder.ins().iconst(types::I16, 0)),
            KomeType::I32 | KomeType::U32 => Ok(self.builder.ins().iconst(types::I32, 0)),
            KomeType::I64
            | KomeType::U64
            | KomeType::Isize
            | KomeType::Usize
            | KomeType::Pointer => Ok(self.builder.ins().iconst(types::I64, 0)),
            KomeType::Void => Err(CodegenError::new(
                "internal error: Void has no zero value",
                None,
            )),
            KomeType::Struct(id) => {
                let layout = self.info.struct_info(id).clone();
                let size = self
                    .builder
                    .ins()
                    .iconst(types::I64, i64::from(layout.size));
                let alloc = Module::declare_func_in_func(
                    self.module,
                    self.foreign.struct_alloc,
                    self.builder.func,
                );
                let call = self.builder.ins().call(alloc, &[size]);
                let pointer = self.builder.inst_results(call)[0];
                for field in &layout.fields {
                    let value = self.zero_value(field.kome_type)?;
                    self.builder
                        .ins()
                        .store(MachMemFlags::new(), value, pointer, field.offset);
                }
                Ok(pointer)
            }
            KomeType::Task(_) => Err(CodegenError::new(
                "Task cannot be used without an initializer",
                None,
            )),
            KomeType::List(_) => {
                let length = self.builder.ins().iconst(types::I64, 0);
                let alloc = Module::declare_func_in_func(
                    self.module,
                    self.foreign.list_alloc,
                    self.builder.func,
                );
                let call = self.builder.ins().call(alloc, &[length]);
                Ok(self.builder.inst_results(call)[0])
            }
            KomeType::Enum(_) => Ok(self.builder.ins().iconst(types::I64, 0)),
            KomeType::Optional(_) => Ok(self.builder.ins().iconst(types::I64, 0)),
            KomeType::Closure(_) => Err(CodegenError::new(
                "closure values cannot be used without an initializer",
                None,
            )),
        }
    }

    #[allow(unused)]
    fn retain_number(&mut self, value: ir::Value) {
        let function = Module::declare_func_in_func(
            self.module,
            self.foreign.number_retain,
            self.builder.func,
        );

        self.builder.ins().call(function, &[value]);
    }

    fn own_value(&mut self, value: TypedValue, span: Span) -> CodegenResult<ir::Value> {
        let raw = value.expect_value(span)?;

        if value.kome_type.is_managed() {
            self.take_managed_ownership(raw, value.kome_type, value.ownership);
        }

        Ok(raw)
    }

    fn take_managed_ownership(
        &mut self,
        value: ir::Value,
        kome_type: KomeType,
        ownership: ValueOwnership,
    ) {
        match ownership {
            ValueOwnership::Owned => {}
            ValueOwnership::Borrowed => {
                self.retain_managed(value, kome_type);
            }
            ValueOwnership::BorrowedMovable { scope, variable } => {
                let scoped = self.scopes[scope]
                    .values_mut()
                    .find(|scoped| scoped.variable == variable)
                    .expect("movable variable must still exist");

                scoped.owns_value = false;
            }
        }
    }

    fn release_owned_managed(&mut self) {
        let values = self
            .scopes
            .iter()
            .flat_map(|scope| scope.values())
            .filter(|scoped| scoped.kome_type.is_managed() && scoped.owns_value)
            .map(|scoped| (self.builder.use_var(scoped.variable), scoped.kome_type))
            .collect::<Vec<_>>();

        for (value, kome_type) in values {
            self.release_managed(value, kome_type);
        }
    }

    fn retain_managed(&mut self, value: ir::Value, kome_type: KomeType) {
        let function = match kome_type {
            KomeType::Number => self.foreign.number_retain,
            KomeType::String => self.foreign.string_retain,
            KomeType::Socket | KomeType::Listener => self.foreign.socket_retain,
            KomeType::Struct(_) => self.foreign.struct_retain,
            KomeType::Closure(_) => self.foreign.closure_retain,
            KomeType::Task(_) => self.foreign.task_retain,
            KomeType::List(_) => self.foreign.list_retain,
            KomeType::Optional(_) => {
                self.retain_optional(value);
                return;
            }
            _ => return,
        };

        let function = Module::declare_func_in_func(self.module, function, self.builder.func);
        self.builder.ins().call(function, &[value]);
    }

    fn release_managed(&mut self, value: ir::Value, kome_type: KomeType) {
        let function = match kome_type {
            KomeType::Number => self.foreign.number_release,
            KomeType::String => self.foreign.string_release,
            KomeType::Socket | KomeType::Listener => self.foreign.socket_release,
            KomeType::Struct(id) => {
                self.release_struct(value, id);
                return;
            }
            KomeType::Closure(_) => {
                let function = Module::declare_func_in_func(
                    self.module,
                    self.foreign.closure_release,
                    self.builder.func,
                );
                self.builder.ins().call(function, &[value]);
                return;
            }
            KomeType::Task(id) => {
                self.release_task(value, id);
                return;
            }
            KomeType::List(id) => {
                self.release_list(value, id);
                return;
            }
            KomeType::Optional(id) => {
                self.release_optional(value, id);
                return;
            }
            _ => return,
        };

        let function = Module::declare_func_in_func(self.module, function, self.builder.func);
        self.builder.ins().call(function, &[value]);
    }

    fn release_struct(&mut self, value: ir::Value, id: usize) {
        let release = Module::declare_func_in_func(
            self.module,
            self.foreign.struct_release,
            self.builder.func,
        );
        let call = self.builder.ins().call(release, &[value]);
        let is_last = self.builder.inst_results(call)[0];
        let destroy = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.ins().brif(is_last, destroy, &[], done, &[]);
        self.builder.switch_to_block(destroy);
        self.builder.seal_block(destroy);
        if let Some(method) = self.info.drop_method(KomeType::Struct(id)).cloned() {
            let function = Module::declare_func_in_func(
                self.module,
                self.func_ids[&method.function_key],
                self.builder.func,
            );
            self.builder.ins().call(function, &[value]);
        }
        let layout = self.info.struct_info(id).clone();
        for field in &layout.fields {
            if field.kome_type.is_managed() {
                let representation = field
                    .kome_type
                    .cranelift()
                    .expect("field has representation");
                let field_value = self.builder.ins().load(
                    representation,
                    MachMemFlags::new(),
                    value,
                    field.offset,
                );
                self.release_managed(field_value, field.kome_type);
            }
        }
        let size = self
            .builder
            .ins()
            .iconst(types::I64, i64::from(layout.size));
        let dealloc = Module::declare_func_in_func(
            self.module,
            self.foreign.struct_dealloc,
            self.builder.func,
        );
        self.builder.ins().call(dealloc, &[value, size]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }

    fn retain_optional(&mut self, value: ir::Value) {
        let retain = self.builder.create_block();
        let done = self.builder.create_block();
        let is_present = self.builder.ins().icmp_imm_u(IntCC::NotEqual, value, 0);
        self.builder.ins().brif(is_present, retain, &[], done, &[]);
        self.builder.switch_to_block(retain);
        self.builder.seal_block(retain);
        let function = Module::declare_func_in_func(
            self.module,
            self.foreign.struct_retain,
            self.builder.func,
        );
        self.builder.ins().call(function, &[value]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }

    fn release_optional(&mut self, value: ir::Value, id: usize) {
        let release = self.builder.create_block();
        let done = self.builder.create_block();
        let is_present = self.builder.ins().icmp_imm_u(IntCC::NotEqual, value, 0);
        self.builder.ins().brif(is_present, release, &[], done, &[]);
        self.builder.switch_to_block(release);
        self.builder.seal_block(release);

        let release_function = Module::declare_func_in_func(
            self.module,
            self.foreign.struct_release,
            self.builder.func,
        );
        let call = self.builder.ins().call(release_function, &[value]);
        let is_last = self.builder.inst_results(call)[0];
        let destroy = self.builder.create_block();
        self.builder.ins().brif(is_last, destroy, &[], done, &[]);
        self.builder.switch_to_block(destroy);
        self.builder.seal_block(destroy);

        let inner = self.info.optional_inner(id);
        if inner.is_managed() {
            let slot = self
                .builder
                .ins()
                .load(types::I64, MachMemFlags::new(), value, 0);
            let payload = self.task_slot_to_value(slot, inner);
            self.release_managed(payload, inner);
        }
        let size = self.builder.ins().iconst(types::I64, 8);
        let dealloc = Module::declare_func_in_func(
            self.module,
            self.foreign.struct_dealloc,
            self.builder.func,
        );
        self.builder.ins().call(dealloc, &[value, size]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }

    fn release_task(&mut self, value: ir::Value, id: usize) {
        let release =
            Module::declare_func_in_func(self.module, self.foreign.task_release, self.builder.func);
        let call = self.builder.ins().call(release, &[value]);
        let is_last = self.builder.inst_results(call)[0];
        let destroy = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.ins().brif(is_last, destroy, &[], done, &[]);
        self.builder.switch_to_block(destroy);
        self.builder.seal_block(destroy);

        let wait =
            Module::declare_func_in_func(self.module, self.foreign.task_wait, self.builder.func);
        let wait_call = self.builder.ins().call(wait, &[value]);
        let state = self.builder.inst_results(wait_call)[0];
        let completed = self.builder.ins().icmp_imm_u(IntCC::Equal, state, 2);
        let release_result = self.builder.create_block();
        let deallocate = self.builder.create_block();
        self.builder
            .ins()
            .brif(completed, release_result, &[], deallocate, &[]);
        self.builder.switch_to_block(release_result);
        self.builder.seal_block(release_result);
        let result_type = self.info.task_result(id);
        if result_type.is_managed() {
            let result = Module::declare_func_in_func(
                self.module,
                self.foreign.task_result,
                self.builder.func,
            );
            let result_call = self.builder.ins().call(result, &[value]);
            let result =
                self.task_slot_to_value(self.builder.inst_results(result_call)[0], result_type);
            self.release_managed(result, result_type);
        }
        self.builder.ins().jump(deallocate, &[]);
        self.builder.switch_to_block(deallocate);
        self.builder.seal_block(deallocate);
        let dealloc =
            Module::declare_func_in_func(self.module, self.foreign.task_dealloc, self.builder.func);
        self.builder.ins().call(dealloc, &[value]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }

    fn release_list(&mut self, value: ir::Value, id: usize) {
        let release =
            Module::declare_func_in_func(self.module, self.foreign.list_release, self.builder.func);
        let call = self.builder.ins().call(release, &[value]);
        let is_last = self.builder.inst_results(call)[0];
        let destroy = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.ins().brif(is_last, destroy, &[], done, &[]);
        self.builder.switch_to_block(destroy);
        self.builder.seal_block(destroy);
        let element_type = self.info.list_element(id);
        if element_type.is_managed() {
            let len_fn =
                Module::declare_func_in_func(self.module, self.foreign.list_len, self.builder.func);
            let len_call = self.builder.ins().call(len_fn, &[value]);
            let length = self.builder.inst_results(len_call)[0];
            let loop_block = self.builder.create_block();
            let release_element = self.builder.create_block();
            let deallocate = self.builder.create_block();
            self.builder.append_block_param(loop_block, types::I64);
            let zero = self.builder.ins().iconst(types::I64, 0);
            self.builder
                .ins()
                .jump(loop_block, &[ir::BlockArg::Value(zero)]);
            self.builder.switch_to_block(loop_block);
            let index = self.builder.block_params(loop_block)[0];
            let more = self
                .builder
                .ins()
                .icmp(IntCC::UnsignedLessThan, index, length);
            self.builder
                .ins()
                .brif(more, release_element, &[], deallocate, &[]);
            self.builder.switch_to_block(release_element);
            self.builder.seal_block(release_element);
            let offset = self.builder.ins().imul_imm_u(index, 8);
            let address = self.builder.ins().iadd(value, offset);
            let slot = self
                .builder
                .ins()
                .load(types::I64, MachMemFlags::new(), address, 0);
            let element = self.task_slot_to_value(slot, element_type);
            self.release_managed(element, element_type);
            let next = self.builder.ins().iadd_imm_u(index, 1);
            self.builder
                .ins()
                .jump(loop_block, &[ir::BlockArg::Value(next)]);
            self.builder.seal_block(loop_block);
            self.builder.switch_to_block(deallocate);
            self.builder.seal_block(deallocate);
        }
        let dealloc =
            Module::declare_func_in_func(self.module, self.foreign.list_dealloc, self.builder.func);
        self.builder.ins().call(dealloc, &[value]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
        self.builder.seal_block(done);
    }
}

fn unsupported_statement(kind: &str, statement: &Statement) -> CodegenError {
    CodegenError::at(
        format!("statement `{kind}` is not supported yet"),
        statement.span(),
    )
}

#[cfg(test)]
mod tests {
    use super::mangled_name;

    #[test]
    fn mangles_qualified_names_without_native_punctuation() {
        let symbol = mangled_name("std::io::println");

        assert_eq!(symbol, "kome_q7374643a3a696f3a3a7072696e746c6e");
        assert!(
            symbol
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        );
    }

    #[test]
    fn preserves_simple_function_symbols() {
        assert_eq!(mangled_name("main"), "kome_main");
    }
}
