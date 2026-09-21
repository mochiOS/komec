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
}

impl ForeignFunctions {
    fn declare<M: Module>(module: &mut M) -> CodegenResult<Self> {
        let native_call = declare_foreign(
            module,
            "__kome_native_call",
            &[types::I64, types::I64, types::I64, types::I64],
            types::I64,
        )?;

        Ok(Self { native_call })
    }
}

fn declare_foreign<M: Module>(
    module: &mut M,
    name: &str,
    params: &[types::Type],
    ret: types::Type,
) -> CodegenResult<FuncId> {
    let mut signature = module.make_signature();

    signature
        .params
        .extend(params.iter().map(|ty| AbiParam::new(*ty)));
    signature.returns.push(AbiParam::new(ret));

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

/// The result of evaluating an expression: its native value and type.
///
/// The value is `None` exactly when the type is `Void`.
#[derive(Debug, Clone, Copy)]
struct TypedValue {
    value: Option<ir::Value>,
    kome_type: KomeType,
}

impl TypedValue {
    fn some(value: ir::Value, kome_type: KomeType) -> Self {
        Self {
            value: Some(value),
            kome_type,
        }
    }

    fn void() -> Self {
        Self {
            value: None,
            kome_type: KomeType::Void,
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

            self.declare_variable(&identifier.name, value, *param_type)?;
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

        self.scopes.pop();

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
                    Some(expression) => self.evaluate(expression)?,

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

                self.declare_variable(&pattern.name, value, typed.kome_type)?;
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
    ) -> CodegenResult<()> {
        let representation = kome_type.cranelift().ok_or_else(|| {
            CodegenError::new("internal error: Void cannot be stored in a variable", None)
        })?;

        let variable = self.builder.declare_var(representation);

        self.builder.def_var(variable, value);

        let scope = self.scopes.last_mut().expect("scope stack is never empty");

        scope.insert(
            name.to_owned(),
            ScopedVariable {
                variable,
                kome_type,
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
                let parsed = number.0.parse::<f64>().map_err(|_| {
                    CodegenError::at(
                        format!("`{}` is not a valid number", number.0),
                        literal.span,
                    )
                })?;

                Ok(TypedValue::some(
                    self.builder.ins().f64const(parsed),
                    KomeType::Number,
                ))
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

        Ok(TypedValue::some(
            self.builder.use_var(scoped.variable),
            scoped.kome_type,
        ))
    }

    fn evaluate_group(&mut self, group: &GroupExpression) -> CodegenResult<TypedValue> {
        self.evaluate(&group.expression)
    }

    fn c_string_pointer(&mut self, symbol: &str) -> CodegenResult<ir::Value> {
        let data_id = self.native_symbols.intern(self.module, symbol)?;
        let global = Module::declare_data_in_func(self.module, data_id, self.builder.func);

        Ok(self
            .builder
            .ins()
            .symbol_value(self.module.target_config().pointer_type(), global))
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
            BinaryOp::Add => match (left.kome_type, right.kome_type) {
                (KomeType::Number, KomeType::Number) => {
                    let (l, r) = (left.expect_value(span)?, right.expect_value(span)?);

                    Ok(TypedValue::some(
                        self.builder.ins().fadd(l, r),
                        KomeType::Number,
                    ))
                }

                _ => Err(invalid_operands(left, right)),
            },

            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                if left.kome_type != KomeType::Number || right.kome_type != KomeType::Number {
                    return Err(invalid_operands(left, right));
                }

                let l = left.expect_value(span)?;
                let r = right.expect_value(span)?;

                let value = match binary.op {
                    BinaryOp::Sub => self.builder.ins().fsub(l, r),
                    BinaryOp::Mul => self.builder.ins().fmul(l, r),
                    BinaryOp::Div => self.builder.ins().fdiv(l, r),
                    _ => unreachable!("only arithmetic operations are handled here"),
                };

                Ok(TypedValue::some(value, KomeType::Number))
            }

            BinaryOp::Eq | BinaryOp::NotEq => {
                let equal = binary.op == BinaryOp::Eq;

                match (left.kome_type, right.kome_type) {
                    (KomeType::Number, KomeType::Number) => {
                        let (l, r) = (left.expect_value(span)?, right.expect_value(span)?);

                        let flag = self.builder.ins().fcmp(
                            if equal {
                                FloatCC::Equal
                            } else {
                                FloatCC::NotEqual
                            },
                            l,
                            r,
                        );

                        Ok(TypedValue::some(flag, KomeType::Boolean))
                    }

                    (KomeType::Boolean, KomeType::Boolean) | (KomeType::Null, KomeType::Null) => {
                        let (l, r) = (left.expect_value(span)?, right.expect_value(span)?);

                        let flag = self.builder.ins().icmp(
                            if equal { IntCC::Equal } else { IntCC::NotEqual },
                            l,
                            r,
                        );

                        Ok(TypedValue::some(flag, KomeType::Boolean))
                    }

                    _ => Err(invalid_operands(left, right)),
                }
            }

            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                if left.kome_type != KomeType::Number || right.kome_type != KomeType::Number {
                    return Err(invalid_operands(left, right));
                }

                let (l, r) = (left.expect_value(span)?, right.expect_value(span)?);

                let condition = match binary.op {
                    BinaryOp::Lt => FloatCC::LessThan,
                    BinaryOp::Lte => FloatCC::LessThanOrEqual,
                    BinaryOp::Gt => FloatCC::GreaterThan,
                    BinaryOp::Gte => FloatCC::GreaterThanOrEqual,
                    _ => unreachable!("only ordering operations are handled here"),
                };

                Ok(TypedValue::some(
                    self.builder.ins().fcmp(condition, l, r),
                    KomeType::Boolean,
                ))
            }

            BinaryOp::And | BinaryOp::Or => {
                if left.kome_type != KomeType::Boolean || right.kome_type != KomeType::Boolean {
                    return Err(invalid_operands(left, right));
                }

                let (l, r) = (left.expect_value(span)?, right.expect_value(span)?);

                let flag = if binary.op == BinaryOp::And {
                    self.builder.ins().band(l, r)
                } else {
                    self.builder.ins().bor(l, r)
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
            let tag = self.builder.ins().iconst(types::I64, param_type.tag());
            self.builder
                .ins()
                .store(MachMemFlags::new(), tag, tag_address, 0);

            let payload = match param_type {
                KomeType::Number => {
                    self.builder
                        .ins()
                        .bitcast(types::I64, MachMemFlags::new(), *value)
                }
                KomeType::Boolean | KomeType::Null => {
                    self.builder.ins().uextend(types::I64, *value)
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

        let ret_tag = self.builder.ins().iconst(types::I64, signature.ret.tag());

        let func_ref =
            Module::declare_func_in_func(self.module, self.foreign.native_call, self.builder.func);

        let call = self
            .builder
            .ins()
            .call(func_ref, &[name_pointer, argc, buffer, ret_tag]);

        let payload = self.builder.inst_results(call)[0];

        let value = match signature.ret {
            KomeType::Void => return Ok(TypedValue::void()),
            KomeType::Number => {
                self.builder
                    .ins()
                    .bitcast(types::F64, MachMemFlags::new(), payload)
            }
            KomeType::Boolean | KomeType::Null => self.builder.ins().ireduce(types::I8, payload),
        };

        Ok(TypedValue::some(value, signature.ret))
    }

    fn zero_value(&mut self, kome_type: KomeType) -> CodegenResult<ir::Value> {
        match kome_type {
            KomeType::Number => Ok(self.builder.ins().f64const(0.0)),

            KomeType::Boolean | KomeType::Null => Ok(self.builder.ins().iconst(types::I8, 0)),

            KomeType::Void => Err(CodegenError::new(
                "internal error: Void has no zero value",
                None,
            )),
        }
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
