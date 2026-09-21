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
use kome_ast::declarations::{Declaration, FunctionDeclaration, Module as KomeModule};
use kome_ast::expressions::{
    AssignOp, AssignmentExpression, BinaryExpression, BinaryOp, CallArg, CallExpression,
    Expression, GroupExpression, IdentifierExpression, LiteralExpression, LiteralKind,
    NumberLiteral,
};
use kome_ast::statements::{BlockStatement, Statement};
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
pub enum FunctionKind<'a> {
    Native {
        signature: FunctionSignature,

        /// The symbol registered in the native runtime's [`NativeRegistry`](kome_native_rt::NativeRegistry).
        symbol: &'a str,
    },

    User {
        declaration: &'a FunctionDeclaration,
        signature: FunctionSignature,
    },
}

impl FunctionKind<'_> {
    pub fn signature(&self) -> &FunctionSignature {
        match self {
            Self::Native { signature, .. } => signature,
            Self::User { signature, .. } => signature,
        }
    }
}

/// Static information about a module, gathered before code generation.
#[derive(Debug)]
pub struct ModuleInfo<'a> {
    functions: HashMap<String, FunctionKind<'a>>,
}

impl<'a> ModuleInfo<'a> {
    pub fn get(&self, name: &str) -> Option<&FunctionKind<'a>> {
        self.functions.get(name)
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
            } => Some((name.as_str(), *declaration, signature)),
            FunctionKind::Native { .. } => None,
        })
    }
}

/// Collects function declarations and computes their signatures.
pub fn analyze_module(module: &KomeModule) -> CodegenResult<ModuleInfo<'_>> {
    let mut functions = HashMap::new();

    for declaration in &module.declarations {
        let Declaration::Function(function) = declaration else {
            continue;
        };

        let signature = analyze_signature(function)?;

        let kind = match native_symbol(function)? {
            Some(symbol) => FunctionKind::Native { signature, symbol },

            None => FunctionKind::User {
                declaration: function,
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

    Ok(ModuleInfo { functions })
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
                return_type: signature.ret,
                terminated: false,
            };

            translator.translate_function(declaration, signature)?;
        }

        module
            .define_function(func_id, &mut context)
            .map_err(|error| {
                CodegenError::at(
                    format!("failed to compile function `{name}`: {error}"),
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

        Ok(Self {
            native_call,
            number_parse,
            number_retain,
            number_release,
            number_add,
            number_sub,
            number_mul,
            number_compare,
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

fn analyze_signature(function: &FunctionDeclaration) -> CodegenResult<FunctionSignature> {
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

        let Some(annotation) = &identifier.type_annotation else {
            return Err(CodegenError::at(
                format!(
                    "parameter `{}` of function `{}` requires a type annotation",
                    identifier.name, function.name
                ),
                identifier.span,
            ));
        };

        let param_type = KomeType::from_annotation(annotation)?;

        if param_type == KomeType::Void {
            return Err(CodegenError::at(
                "parameters cannot have type Void",
                identifier.span,
            ));
        }

        params.push(param_type);
    }

    let ret = match &function.return_type {
        Some(annotation) => KomeType::from_annotation(annotation)?,
        None => KomeType::Void,
    };

    Ok(FunctionSignature { params, ret })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueOwnership {
    Borrowed,
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

struct FunctionTranslator<'b, 'c, 'a, M: Module> {
    builder: FunctionBuilder<'c>,
    module: &'b mut M,
    info: &'b ModuleInfo<'a>,
    func_ids: &'b HashMap<String, FuncId>,
    foreign: &'b ForeignFunctions,
    native_symbols: &'b mut NativeSymbolPool,
    scopes: Vec<HashMap<String, ScopedVariable>>,
    return_type: KomeType,
    terminated: bool,
}

#[derive(Debug, Clone, Copy)]
struct ScopedVariable {
    variable: Variable,
    kome_type: KomeType,
    owns_value: bool,
}

impl<'b, 'c, 'a, M: Module> FunctionTranslator<'b, 'c, 'a, M> {
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
                if scoped.kome_type == KomeType::Number && scoped.owns_value {
                    let value = self.builder.use_var(scoped.variable);

                    self.release_number(value);
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

                        typed.expect_value(expression.span())?
                    }

                    None => match self.return_type {
                        KomeType::Void => {
                            self.builder.ins().return_(&[]);
                            self.terminated = true;
                            return Ok(());
                        }

                        return_type => self.zero_value(return_type)?,
                    },
                };

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
                    .map(KomeType::from_annotation)
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

        let owns_value = if kome_type == KomeType::Number {
            if ownership == ValueOwnership::Borrowed {
                self.retain_number(value);
            }

            true
        } else {
            false
        };

        let variable = self.builder.declare_var(representation);

        self.builder.def_var(variable, value);

        let scope = self.scopes.last_mut().expect("scope stack is never empty");

        scope.insert(
            name.to_owned(),
            ScopedVariable {
                variable,
                kome_type,
                owns_value,
            },
        );

        Ok(())
    }

    fn evaluate(&mut self, expression: &Expression) -> CodegenResult<TypedValue> {
        match expression {
            Expression::Literal(literal) => self.evaluate_literal(literal),

            Expression::Ident(identifier) => self.evaluate_identifier(identifier),

            Expression::Group(group) => self.evaluate_group(group),

            Expression::Binary(binary) => self.evaluate_binary(binary),

            Expression::Call(call) => self.evaluate_call(call),

            Expression::Assign(assign) => self.evaluate_assign(assign),

            other => Err(CodegenError::at(
                format!(
                    "expression `{}` is not supported yet",
                    expression_kind(other)
                ),
                other.span(),
            )),
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

            LiteralKind::Boolean(flag) => Ok(TypedValue::some(
                self.builder.ins().iconst(types::I8, i64::from(*flag)),
                KomeType::Boolean,
            )),

            LiteralKind::Null => Ok(TypedValue::some(
                self.builder.ins().iconst(types::I8, 0),
                KomeType::Null,
            )),

            LiteralKind::String(_) => Err(CodegenError::at(
                "string literals are not supported yet",
                literal.span,
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
        let scoped = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&identifier.name))
            .copied()
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;

        Ok(TypedValue::borrowed(
            self.builder.use_var(scoped.variable),
            scoped.kome_type,
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

            KomeType::F64 | KomeType::Number => {
                let value = number.0.parse::<f64>().map_err(|_| {
                    CodegenError::at(format!("`{}` is not a valid number", number.0), span)
                })?;

                Ok(TypedValue::some(
                    self.builder.ins().f64const(value),
                    expected,
                ))
            }

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
        }

        match plan {
            CalleePlan::User => self.emit_user_call(callee, &arguments, &signature),

            CalleePlan::Native { symbol } => self.emit_native_call(&symbol, &arguments, &signature),
        }
    }

    fn evaluate_assign(&mut self, assignment: &AssignmentExpression) -> CodegenResult<TypedValue> {
        let Expression::Ident(identifier) = assignment.target.as_ref() else {
            return Err(CodegenError::at(
                "assignment target must be an identifier",
                assignment.target.span(),
            ));
        };

        let scoped = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&identifier.name))
            .copied()
            .ok_or_else(|| {
                CodegenError::at(
                    format!("variable `{}` is not defined", identifier.name),
                    identifier.span,
                )
            })?;

        let value = self.evaluate(&assignment.value)?;

        if value.kome_type != scoped.kome_type {
            return Err(CodegenError::at(
                format!(
                    "cannot assign {} to variable `{}` of type {}",
                    value.kome_type.name(),
                    identifier.name,
                    scoped.kome_type.name(),
                ),
                assignment.value.span(),
            ));
        }

        let value = value.expect_value(assignment.value.span())?;

        match assignment.op {
            AssignOp::Assign => {
                self.builder.def_var(scoped.variable, value);
            }

            _ => {
                return Err(CodegenError::at(
                    "compound assignment is not supported yet",
                    assignment.span,
                ));
            }
        }

        Ok(TypedValue::some(value, scoped.kome_type))
    }

    fn emit_user_call(
        &mut self,
        callee: &IdentifierExpression,
        arguments: &[ir::Value],
        signature: &FunctionSignature,
    ) -> CodegenResult<TypedValue> {
        let func_id = self.func_ids[&callee.name];

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
                KomeType::Number => *value,

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

            KomeType::Number => payload,

            KomeType::Boolean | KomeType::Null => self.builder.ins().ireduce(types::I8, payload),

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
        }
    }

    fn retain_number(&mut self, value: ir::Value) {
        let function = Module::declare_func_in_func(
            self.module,
            self.foreign.number_retain,
            self.builder.func,
        );

        self.builder.ins().call(function, &[value]);
    }

    fn release_number(&mut self, value: ir::Value) {
        let function = Module::declare_func_in_func(
            self.module,
            self.foreign.number_release,
            self.builder.func,
        );

        self.builder.ins().call(function, &[value]);
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
        Expression::Binary(_) => "binary",
        Expression::Call(_) => "call",
        Expression::Member(_) => "member access",
        Expression::Index(_) => "index",
        Expression::Assign(_) => "assignment",
        Expression::Group(_) => "group",
        Expression::Block(_) => "block",
        Expression::List(_) => "list",
        Expression::Object(_) => "object",
        Expression::Template(_) => "template",
        Expression::Closure(_) => "closure",
        Expression::DotIdent(_) => "dot identifier",
        Expression::Is(_) => "is",
        Expression::Component(_) => "component",
    }
}
