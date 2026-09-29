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
    Binding, Declaration, ForDeclaration, FunctionDeclaration, Module as KomeModule,
    StructDeclaration, TraitDeclaration, TypeMember,
};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, BinaryExpression, BinaryOp, CallArg, CallExpression,
    Expression, GroupExpression, IdentifierExpression, LiteralExpression, LiteralKind,
    MemberExpression, NumberLiteral, PropertyKey, StructExpression, TaskExpression, WaitExpression,
};
use kome_ast::statements::{BlockStatement, Statement};
use std::cell::RefCell;
use std::collections::HashMap;

/// The compiled signature of a Kome function.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub params: Vec<KomeType>,
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

    User {
        declaration: FunctionDeclaration,
        signature: FunctionSignature,
    },
}

impl FunctionKind {
    pub fn signature(&self) -> &FunctionSignature {
        match self {
            Self::Native { signature, .. } => signature,
            Self::User { signature, .. } => signature,
        }
    }
}

/// Static information about a module, gathered before code generation.
#[derive(Debug)]
pub struct ModuleInfo {
    functions: HashMap<String, FunctionKind>,
    runtime_types: HashMap<String, KomeType>,
    structs: Vec<StructInfo>,
    struct_ids: HashMap<String, usize>,
    implementations: Vec<TypeImplementation>,
    task_types: RefCell<Vec<KomeType>>,
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
    pub fn get(&self, name: &str) -> Option<&FunctionKind> {
        self.functions.get(name)
    }

    pub fn runtime_type(&self, name: &str) -> Option<KomeType> {
        self.runtime_types.get(name).copied()
    }

    fn struct_info(&self, id: usize) -> &StructInfo {
        &self.structs[id]
    }

    fn type_name(&self, ty: KomeType) -> String {
        match ty {
            KomeType::Struct(id) => self.structs[id].name.clone(),
            KomeType::Task(id) => format!("Task<{}>", self.type_name(self.task_result(id))),
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

    /// The entry point's signature, validated for direct invocation.
    pub fn entry_signature(&self, entry: &str) -> CodegenResult<&FunctionSignature> {
        match self.get(entry) {
            None => Err(CodegenError::new(
                format!("entry function `{entry}` was not found"),
                None,
            )),

            Some(FunctionKind::Native { .. }) => Err(CodegenError::new(
                format!("entry function `{entry}` must not be a @native declaration"),
                None,
            )),

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

    pub fn user_functions(
        &self,
    ) -> impl Iterator<Item = (&str, &FunctionDeclaration, &FunctionSignature)> {
        self.functions.iter().filter_map(|(name, kind)| match kind {
            FunctionKind::User {
                declaration,
                signature,
            } => Some((name.as_str(), declaration, signature)),
            FunctionKind::Native { .. } => None,
        })
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
            let kome_type =
                type_from_annotation(&field.type_, &runtime_types, &struct_ids, &task_types)?;
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
        });
    }
    let structs = structs.into_iter().map(Option::unwrap).collect::<Vec<_>>();

    for declaration in &module.declarations {
        let Declaration::Function(function) = declaration else {
            continue;
        };

        let signature =
            analyze_signature(function, &runtime_types, &struct_ids, &task_types, None)?;

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
                    let kome_type =
                        type_from_annotation(annotation, &runtime_types, &struct_ids, &task_types)?;
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
            )?;
        }
        implementations.push(TypeImplementation {
            target,
            trait_name,
            methods,
            constants,
        });
    }

    Ok(ModuleInfo {
        functions,
        runtime_types,
        structs,
        struct_ids,
        implementations,
        task_types,
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
) -> CodegenResult<()> {
    for required in &trait_decl.functions {
        let expected = analyze_signature(
            required,
            runtime_types,
            struct_ids,
            task_types,
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

/// The mangled symbol for a user function.
pub fn mangled_name(name: &str) -> String {
    format!("kome_{name}")
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

    for (name, _, signature) in info.user_functions() {
        let cranelift_signature = build_signature(module, signature)?;

        let func_id = module
            .declare_function(&mangled_name(name), Linkage::Export, &cranelift_signature)
            .map_err(|error| CodegenError::new(error.to_string(), None))?;

        func_ids.insert(name.to_owned(), func_id);
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
                foreign: &foreign,
                native_symbols: &mut native_symbols,
                scopes: vec![HashMap::new()],
                remaining_reads: HashMap::new(),
                next_binding_id: 0,
                return_type: signature.ret,
                terminated: false,
            };

            translator.translate_function(declaration, signature)?;
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
    number_compare: FuncId,
    string_create: FuncId,
    string_retain: FuncId,
    string_release: FuncId,
    struct_alloc: FuncId,
    struct_retain: FuncId,
    struct_release: FuncId,
    struct_dealloc: FuncId,
    task_create: FuncId,
    task_start: FuncId,
    task_complete: FuncId,
    task_wait: FuncId,
    task_result: FuncId,
    task_retain: FuncId,
    task_release: FuncId,
    task_dealloc: FuncId,
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

        let number_compare = declare_foreign(
            module,
            "__kome_number_compare",
            &[types::I64, types::I64],
            Some(types::I32),
        )?;

        let string_create = declare_foreign(
            module,
            "__kome_string_create",
            &[types::I64, types::I64],
            Some(types::I64),
        )?;

        let string_retain = declare_foreign(module, "__kome_string_retain", &[types::I64], None)?;
        let string_release = declare_foreign(module, "__kome_string_release", &[types::I64], None)?;
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
        let task_create = declare_foreign(module, "__kome_task_create", &[], Some(types::I64))?;
        let task_start = declare_foreign(module, "__kome_task_start", &[types::I64], None)?;
        let task_complete = declare_foreign(
            module,
            "__kome_task_complete",
            &[types::I64, types::I64],
            None,
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

        Ok(Self {
            native_call,
            number_parse,
            number_retain,
            number_release,
            number_add,
            number_sub,
            number_mul,
            number_compare,
            string_create,
            string_retain,
            string_release,
            struct_alloc,
            struct_retain,
            struct_release,
            struct_dealloc,
            task_create,
            task_start,
            task_complete,
            task_wait,
            task_result,
            task_retain,
            task_release,
            task_dealloc,
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
    self_type: Option<KomeType>,
) -> CodegenResult<FunctionSignature> {
    let mut params = Vec::with_capacity(function.params.len());

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
            type_from_annotation(annotation, runtime_types, struct_ids, task_types)?
        };

        if param_type == KomeType::Void {
            return Err(CodegenError::at(
                "parameters cannot have type Void",
                identifier.span,
            ));
        }

        params.push(param_type);
    }

    let ret = match &function.return_type {
        Some(annotation) => {
            type_from_annotation(annotation, runtime_types, struct_ids, task_types)?
        }
        None => KomeType::Void,
    };

    Ok(FunctionSignature { params, ret })
}

fn type_from_annotation(
    annotation: &kome_ast::types::Type,
    runtime_types: &HashMap<String, KomeType>,
    struct_ids: &HashMap<String, usize>,
    task_types: &RefCell<Vec<KomeType>>,
) -> CodegenResult<KomeType> {
    if let kome_ast::types::Type::Named(named) = annotation {
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

    KomeType::from_annotation(annotation)
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

    Native { symbol: String },
}

struct FunctionTranslator<'b, 'c, M: Module> {
    builder: FunctionBuilder<'c>,
    module: &'b mut M,
    info: &'b ModuleInfo,
    func_ids: &'b HashMap<String, FuncId>,
    foreign: &'b ForeignFunctions,
    native_symbols: &'b mut NativeSymbolPool,
    scopes: Vec<HashMap<String, ScopedVariable>>,
    remaining_reads: HashMap<usize, usize>,
    next_binding_id: usize,
    return_type: KomeType,
    terminated: bool,
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

            _ => {}
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

            Expression::Task(task) => self.visit_expression(&task.argument),

            Expression::Wait(wait) => self.visit_expression(&wait.argument),

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
                self.visit_expression(&assignment.value);
            }

            Expression::Member(member) => self.visit_expression(&member.object),

            Expression::Struct(struct_) => {
                for field in &struct_.fields {
                    self.visit_expression(&field.value);
                }
            }

            _ => {}
        }
    }
}

impl<'b, 'c, M: Module> FunctionTranslator<'b, 'c, M> {
    fn translate_function(
        mut self,
        declaration: &FunctionDeclaration,
        signature: &FunctionSignature,
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

            self.declare_variable(
                &identifier.name,
                value,
                *param_type,
                ValueOwnership::Borrowed,
            )?;
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

        Ok(())
    }

    fn translate_statement(&mut self, statement: &Statement) -> CodegenResult<()> {
        match statement {
            Statement::Empty(_) => {}

            Statement::Expression(statement) => {
                self.evaluate(&statement.expression)?;
            }

            Statement::Return(statement) => {
                let value = match &statement.argument {
                    Some(expression) => {
                        let typed = self.evaluate(expression)?;

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

            Statement::If(_) => return Err(unsupported_statement("if", statement)),

            Statement::While(_) => return Err(unsupported_statement("while", statement)),

            Statement::ForIn(_) => return Err(unsupported_statement("for", statement)),

            Statement::Break(_) => return Err(unsupported_statement("break", statement)),

            Statement::Continue(_) => return Err(unsupported_statement("continue", statement)),

            Statement::Is(_) => return Err(unsupported_statement("is", statement)),

            Statement::Declaration(_) => {
                return Err(unsupported_statement("nested declaration", statement));
            }
        }

        Ok(())
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

    fn evaluate(&mut self, expression: &Expression) -> CodegenResult<TypedValue> {
        match expression {
            Expression::Literal(literal) => self.evaluate_literal(literal),

            Expression::Ident(identifier) => self.evaluate_identifier(identifier),

            Expression::Group(group) => self.evaluate_group(group),

            Expression::Task(task) => self.evaluate_task(task, None),

            Expression::Wait(wait) => self.evaluate_wait(wait),

            Expression::Binary(binary) => self.evaluate_binary(binary),

            Expression::Call(call) => self.evaluate_call(call),

            Expression::Assign(assign) => self.evaluate_assign(assign),

            Expression::Struct(struct_) => self.evaluate_struct(struct_),

            Expression::Member(member) => self.evaluate_member(member),

            other => Err(CodegenError::at(
                format!(
                    "expression `{}` is not supported yet",
                    expression_kind(other)
                ),
                other.span(),
            )),
        }
    }

    fn evaluate_task(
        &mut self,
        task: &TaskExpression,
        expected_result: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        let create =
            Module::declare_func_in_func(self.module, self.foreign.task_create, self.builder.func);
        let call = self.builder.ins().call(create, &[]);
        let handle = self.builder.inst_results(call)[0];
        let start =
            Module::declare_func_in_func(self.module, self.foreign.task_start, self.builder.func);
        self.builder.ins().call(start, &[handle]);

        let result = self.evaluate_with_expected(&task.argument, expected_result)?;
        if result.kome_type == KomeType::Void {
            return Err(CodegenError::at(
                "task expressions returning Void are not supported yet",
                task.span,
            ));
        }
        let value = result.expect_value(task.argument.span())?;
        self.take_managed_ownership(value, result.kome_type, result.ownership);
        let slot = self.value_to_task_slot(value, result.kome_type);
        let complete = Module::declare_func_in_func(
            self.module,
            self.foreign.task_complete,
            self.builder.func,
        );
        self.builder.ins().call(complete, &[handle, slot]);

        Ok(TypedValue::some(
            handle,
            self.info.task_type(result.kome_type),
        ))
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
        self.builder.ins().call(wait_fn, &[handle]);
        let result_fn =
            Module::declare_func_in_func(self.module, self.foreign.task_result, self.builder.func);
        let call = self.builder.ins().call(result_fn, &[handle]);
        let result_type = self.info.task_result(id);
        let value = self.task_slot_to_value(self.builder.inst_results(call)[0], result_type);
        if result_type.is_managed() {
            self.retain_managed(value, result_type);
        }
        self.release_owned_temporary(task, wait.argument.span())?;
        Ok(TypedValue::some(value, result_type))
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

            LiteralKind::Percent(_) => Err(CodegenError::at(
                "percent literals are not supported yet",
                literal.span,
            )),
        }
    }

    fn evaluate_identifier(
        &mut self,
        identifier: &IdentifierExpression,
    ) -> CodegenResult<TypedValue> {
        let scope = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(&identifier.name))
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;

        let scoped = self.scopes[scope][&identifier.name];

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

        let ownership = if scoped.kome_type.is_managed() && scoped.owns_value && *remaining == 0 {
            ValueOwnership::BorrowedMovable {
                scope,
                variable: scoped.variable,
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

    fn evaluate_group(&mut self, group: &GroupExpression) -> CodegenResult<TypedValue> {
        self.evaluate(&group.expression)
    }

    fn evaluate_with_expected(
        &mut self,
        expression: &Expression,
        expected: Option<KomeType>,
    ) -> CodegenResult<TypedValue> {
        if let Expression::Task(task) = expression
            && let Some(KomeType::Task(id)) = expected
        {
            return self.evaluate_task(task, Some(self.info.task_result(id)));
        }
        if let Expression::Literal(literal) = expression {
            if let LiteralKind::Number(number) = &literal.kind {
                if let Some(expected) = expected {
                    return self.evaluate_numeric_literal(number, literal.span, expected);
                }
            }
        }

        self.evaluate(expression)
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
        let layout = self.info.struct_info(id).clone();
        if expression.fields.len() != layout.fields.len() {
            return Err(CodegenError::at(
                format!(
                    "struct `{}` expects {} field(s), but received {}",
                    expression.name,
                    layout.fields.len(),
                    expression.fields.len()
                ),
                expression.span,
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
            let property = expression
                .fields
                .iter()
                .find(|property| match &property.key {
                    PropertyKey::Ident { name, .. } | PropertyKey::String { value: name, .. } => {
                        name == &field.name
                    }
                    PropertyKey::Number { .. } | PropertyKey::Computed { .. } => false,
                })
                .ok_or_else(|| {
                    CodegenError::at(
                        format!("missing field `{}` in `{}`", field.name, expression.name),
                        expression.span,
                    )
                })?;
            let duplicates = expression
                .fields
                .iter()
                .filter(|candidate| match &candidate.key {
                    PropertyKey::Ident { name, .. } | PropertyKey::String { value: name, .. } => {
                        name == &field.name
                    }
                    PropertyKey::Number { .. } | PropertyKey::Computed { .. } => false,
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
        for property in &expression.fields {
            let name = match &property.key {
                PropertyKey::Ident { name, .. } | PropertyKey::String { value: name, .. } => name,
                PropertyKey::Number { .. } | PropertyKey::Computed { .. } => {
                    return Err(CodegenError::at(
                        "struct field names must be identifiers",
                        property.span,
                    ));
                }
            };
            if !layout.fields.iter().any(|field| field.name == *name) {
                return Err(CodegenError::at(
                    format!("struct `{}` has no field `{name}`", expression.name),
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
        let left = self.evaluate(&binary.left)?;
        let right = self.evaluate(&binary.right)?;

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
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                if left.kome_type != KomeType::Number || right.kome_type != KomeType::Number {
                    return Err(invalid_operands(left, right));
                }

                let left_value = left.expect_value(span)?;
                let right_value = right.expect_value(span)?;

                let function = match binary.op {
                    BinaryOp::Add => self.foreign.number_add,
                    BinaryOp::Sub => self.foreign.number_sub,
                    BinaryOp::Mul => self.foreign.number_mul,
                    _ => unreachable!("only Number arithmetic operations are handled here"),
                };

                let value = self.emit_number_binary(function, left_value, right_value);

                self.release_owned_temporary(left, span)?;
                self.release_owned_temporary(right, span)?;

                Ok(TypedValue::some(value, KomeType::Number))
            }

            BinaryOp::Div => Err(CodegenError::at(
                "Number division semantics are not defined yet",
                binary.span,
            )),

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

                    _ => Err(invalid_operands(left, right)),
                }
            }

            BinaryOp::And | BinaryOp::Or => {
                if left.kome_type != KomeType::Boolean || right.kome_type != KomeType::Boolean {
                    return Err(invalid_operands(left, right));
                }

                let left_value = left.expect_value(span)?;
                let right_value = right.expect_value(span)?;

                let flag = if binary.op == BinaryOp::And {
                    self.builder.ins().band(left_value, right_value)
                } else {
                    self.builder.ins().bor(left_value, right_value)
                };

                Ok(TypedValue::some(flag, KomeType::Boolean))
            }
        }
    }

    fn evaluate_call(&mut self, call: &CallExpression) -> CodegenResult<TypedValue> {
        if let Expression::Member(member) = call.callee.as_ref() {
            return self.evaluate_method_call(member, call);
        }

        let Expression::Ident(callee) = call.callee.as_ref() else {
            return Err(CodegenError::at(
                "calling non-identifier expressions is not supported yet",
                call.callee.span(),
            ));
        };

        let plan = match self.info.get(&callee.name) {
            None => {
                return Err(CodegenError::at(
                    format!("function `{}` was not found", callee.name),
                    callee.span,
                ));
            }

            Some(FunctionKind::User { signature, .. }) => {
                if call.args.len() != signature.params.len() {
                    return Err(CodegenError::at(
                        format!(
                            "function `{}` expects {} argument(s), but received {}",
                            callee.name,
                            signature.params.len(),
                            call.args.len()
                        ),
                        call.span,
                    ));
                }

                CalleePlan::User
            }

            Some(FunctionKind::Native { signature, symbol }) => {
                if call.args.len() != signature.params.len() {
                    return Err(CodegenError::at(
                        format!(
                            "function `{}` expects {} argument(s), but received {}",
                            callee.name,
                            signature.params.len(),
                            call.args.len()
                        ),
                        call.span,
                    ));
                }

                CalleePlan::Native {
                    symbol: (*symbol).to_owned(),
                }
            }
        };

        // Fetch the signature again through a cloned snapshot so that no
        // borrow of `self.info` survives into argument evaluation.
        let signature = self
            .info
            .get(&callee.name)
            .expect("callee existence was checked above")
            .signature()
            .clone();

        let mut arguments = Vec::with_capacity(call.args.len());
        let mut argument_values = Vec::with_capacity(call.args.len());

        for (argument, param_type) in call.args.iter().zip(&signature.params) {
            let expression = match argument {
                CallArg::Positional(expression) => expression,

                CallArg::Named { span, .. } => {
                    return Err(CodegenError::at(
                        "named arguments are not supported yet",
                        *span,
                    ));
                }
            };

            let typed = self.evaluate(expression)?;

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
            CalleePlan::User => self.emit_user_call(&callee.name, &arguments, &signature)?,
            CalleePlan::Native { symbol } => {
                self.emit_native_call(&symbol, &arguments, &signature)?
            }
        };

        for argument in argument_values {
            self.release_owned_temporary(argument, call.span)?;
        }

        Ok(result)
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
        let (_, method, _) = self
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
        let supplied = call.args.len() + usize::from(method.has_self);
        if supplied != method.signature.params.len() {
            return Err(CodegenError::at(
                format!(
                    "method `{}` expects {} argument(s), but received {}",
                    member.property,
                    method.signature.params.len() - usize::from(method.has_self),
                    call.args.len()
                ),
                call.span,
            ));
        }
        let mut arguments = Vec::with_capacity(supplied);
        if let Some(receiver) = evaluated.first() {
            arguments.push(receiver.expect_value(member.object.span())?);
        }
        let skip = usize::from(method.has_self);
        for (argument, expected) in call
            .args
            .iter()
            .zip(method.signature.params.iter().skip(skip))
        {
            let expression = match argument {
                CallArg::Positional(expression) => expression,
                CallArg::Named { span, .. } => {
                    return Err(CodegenError::at(
                        "named arguments are not supported yet",
                        *span,
                    ));
                }
            };
            let typed = self.evaluate_with_expected(expression, Some(*expected))?;
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

    fn evaluate_assign(&mut self, assignment: &AssignmentExpression) -> CodegenResult<TypedValue> {
        let Expression::Ident(identifier) = assignment.target.as_ref() else {
            return Err(CodegenError::at(
                "assignment target must be an identifier",
                assignment.target.span(),
            ));
        };

        let scope = self
            .scopes
            .iter()
            .rposition(|scope| scope.contains_key(&identifier.name))
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;

        let scoped = self.scopes[scope][&identifier.name];
        let typed = self.evaluate(&assignment.value)?;

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
            _ => Err(CodegenError::at(
                "compound assignment is not supported yet",
                assignment.span,
            )),
        }
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
                KomeType::Number | KomeType::String => *value,

                KomeType::Boolean | KomeType::Null => {
                    self.builder.ins().uextend(types::I64, *value)
                }

                KomeType::I8
                | KomeType::I16
                | KomeType::I32
                | KomeType::I64
                | KomeType::U8
                | KomeType::U16
                | KomeType::U32
                | KomeType::U64
                | KomeType::F32
                | KomeType::F64 => {
                    return Err(CodegenError::new(
                        "fixed-width numeric types are not supported by the native ABI yet",
                        None,
                    ));
                }

                KomeType::Void => {
                    return Err(CodegenError::new("parameters cannot have type Void", None));
                }

                KomeType::Struct(_) | KomeType::Task(_) => {
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

        let ret_tag = self.builder.ins().iconst(types::I64, signature.ret.tag()?);

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

            KomeType::Number | KomeType::String => payload,

            KomeType::Boolean | KomeType::Null => self.builder.ins().ireduce(types::I8, payload),

            KomeType::Struct(_) | KomeType::Task(_) => {
                return Err(CodegenError::new(
                    "managed aggregate values cannot cross the native ABI",
                    None,
                ));
            }

            KomeType::I8
            | KomeType::I16
            | KomeType::I32
            | KomeType::I64
            | KomeType::U8
            | KomeType::U16
            | KomeType::U32
            | KomeType::U64
            | KomeType::F32
            | KomeType::F64 => {
                return Err(CodegenError::new(
                    "fixed-width numeric types are not supported by the native ABI yet",
                    None,
                ));
            }
        };

        Ok(TypedValue::some(value, signature.ret))
    }

    fn zero_value(&mut self, kome_type: KomeType) -> CodegenResult<ir::Value> {
        match kome_type {
            KomeType::Number => Err(CodegenError::new(
                "Number cannot be zero-initialized without constructing a runtime value",
                None,
            )),
            KomeType::String => Err(CodegenError::new(
                "String cannot be zero-initialized without constructing a runtime value",
                None,
            )),
            KomeType::F64 => Ok(self.builder.ins().f64const(0.0)),
            KomeType::F32 => Ok(self.builder.ins().f32const(0.0)),
            KomeType::Boolean | KomeType::I8 | KomeType::U8 | KomeType::Null => {
                Ok(self.builder.ins().iconst(types::I8, 0))
            }
            KomeType::I16 | KomeType::U16 => Ok(self.builder.ins().iconst(types::I16, 0)),
            KomeType::I32 | KomeType::U32 => Ok(self.builder.ins().iconst(types::I32, 0)),
            KomeType::I64 | KomeType::U64 => Ok(self.builder.ins().iconst(types::I64, 0)),
            KomeType::Void => Err(CodegenError::new(
                "internal error: Void has no zero value",
                None,
            )),
            KomeType::Struct(id) => Err(CodegenError::new(
                format!(
                    "{} cannot be used without an initializer",
                    self.info.struct_info(id).name
                ),
                None,
            )),
            KomeType::Task(_) => Err(CodegenError::new(
                "Task cannot be used without an initializer",
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
            KomeType::Struct(_) => self.foreign.struct_retain,
            KomeType::Task(_) => self.foreign.task_retain,
            _ => return,
        };

        let function = Module::declare_func_in_func(self.module, function, self.builder.func);
        self.builder.ins().call(function, &[value]);
    }

    fn release_managed(&mut self, value: ir::Value, kome_type: KomeType) {
        let function = match kome_type {
            KomeType::Number => self.foreign.number_release,
            KomeType::String => self.foreign.string_release,
            KomeType::Struct(id) => {
                self.release_struct(value, id);
                return;
            }
            KomeType::Task(id) => {
                self.release_task(value, id);
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
}

fn unsupported_statement(kind: &str, statement: &Statement) -> CodegenError {
    CodegenError::at(
        format!("statement `{kind}` is not supported yet"),
        statement.span(),
    )
}

fn expression_kind(expression: &Expression) -> &'static str {
    match expression {
        Expression::Literal(_) => "literal",
        Expression::Ident(_) => "identifier",
        Expression::Unary(_) => "unary",
        Expression::Task(_) => "task",
        Expression::Wait(_) => "wait",
        Expression::Binary(_) => "binary",
        Expression::Call(_) => "call",
        Expression::Member(_) => "member access",
        Expression::Index(_) => "index",
        Expression::Assign(_) => "assignment",
        Expression::Group(_) => "group",
        Expression::Block(_) => "block",
        Expression::List(_) => "list",
        Expression::Object(_) => "object",
        Expression::Struct(_) => "struct construction",
        Expression::Template(_) => "template",
        Expression::Closure(_) => "closure",
        Expression::DotIdent(_) => "dot identifier",
        Expression::Is(_) => "is",
        Expression::Component(_) => "component",
    }
}
