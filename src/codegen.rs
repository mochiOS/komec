use crate::ast;
use crate::ast::{Expr, Op, Stmt, parse_stmt};
use crate::library::LibraryManager;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicValue, PointerValue, ValueKind};
use log::debug;
use pest::Parser;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct VarArgForwarder {
    target: String,
    suffix: Option<String>,
}

fn resolve_std_module_file(std_root: &std::path::Path, path_parts: &[String]) -> Option<std::path::PathBuf> {
    let rel = path_parts.join("/");
    let direct = std_root.join(format!("{rel}.kome"));
    if direct.exists() {
        return Some(direct);
    }
    let module = std_root.join(rel).join("module.kome");
    if module.exists() {
        return Some(module);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::resolve_std_module_file;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn resolves_module_kome_when_direct_file_missing() {
        let root = std::env::temp_dir().join("kome_std_resolve_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("std/io")).unwrap();
        fs::write(root.join("std/io/module.kome"), "fn hello() {}").unwrap();

        let parts = vec!["std".to_string(), "io".to_string()];
        let resolved = resolve_std_module_file(&root, &parts).unwrap();
        assert_eq!(resolved, PathBuf::from(root.join("std/io/module.kome")));

        let _ = fs::remove_dir_all(&root);
    }
}

/// ASTからLLVM IRを生成するコンテキスト
pub struct CodegenContext<'a, 'ctx> {
    pub context: &'ctx Context,
    pub builder: &'a Builder<'ctx>,
    pub module: &'a Module<'ctx>,
    pub variables: HashMap<String, VariableInfo<'ctx>>,
    pub library_manager: &'a LibraryManager,
    pub current_dir: std::path::PathBuf,
    pub current_module_prefix: Option<String>,
    pub allowed_externs: HashSet<String>,
    pub current_fn_is_variadic: bool,
    pub register_fn: Option<inkwell::values::FunctionValue<'ctx>>,
    pub vararg_forwarders: HashMap<String, VarArgForwarder>,
}

/// 変数の情報
#[derive(Clone, Debug)]
#[allow(unused)]
pub struct VariableInfo<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub is_state: bool,
    pub kind: VariableKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableKind {
    I32,
    Ptr,
}

impl<'a, 'ctx> CodegenContext<'a, 'ctx> {
    fn ensure_register_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        if let Some(f) = self.register_fn {
            return f;
        }
        if let Some(existing) = self.module.get_function("__kome_register") {
            self.register_fn = Some(existing);
            return existing;
        }

        let void_t = self.context.void_type();
        let fn_ty = void_t.fn_type(&[], false);
        let f = self.module.add_function("__kome_register", fn_ty, None);
        let entry = self.context.append_basic_block(f, "entry");

        // 何も登録しない場合も正しく終了できるように ret を置く
        let prev = self.builder.get_insert_block();
        self.builder.position_at_end(entry);
        self.builder.build_return(None).ok();
        if let Some(prev) = prev {
            self.builder.position_at_end(prev);
        } else {
            self.builder.clear_insertion_position();
        }

        self.register_fn = Some(f);
        f
    }
    fn try_register_vararg_forwarder(&mut self, llvm_name: &str, params: &[ast::FnParam], body: &[Stmt]) -> bool {
        if body.len() != 1 {
            return false;
        }
        let Stmt::ExprStmt(Expr::CallChain { head, tails }) = &body[0] else {
            return false;
        };
        if head != "printf" {
            return false;
        }
        let Some(ast::Accessor::Method(args, trailing)) = tails.first() else {
            return false;
        };
        if trailing.is_some() {
            return false;
        }
        if args.len() < 2 || !matches!(args.last(), Some(Expr::VarArgs)) {
            return false;
        }
        // 最初の引数が `args`（第1引数）もしくは `args with "..."` の形かを見る
        let Some(first_param) = params.first() else {
            return false;
        };
        let suffix = match &args[0] {
            Expr::Ident(name) if name == &first_param.name => None,
            Expr::BinaryOp { left, op: Op::With, right } => {
                if matches!(&**left, Expr::Ident(n) if n == &first_param.name) {
                    if let Expr::String(s) = &**right {
                        Some(s.clone())
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            _ => return false,
        };

        self.vararg_forwarders.insert(
            llvm_name.to_string(),
            VarArgForwarder {
                target: "printf".to_string(),
                suffix,
            },
        );
        true
    }
    /// 複数の文（Statements）を順番に LLVM 命令に変換する
    pub fn compile_statements(
        &mut self,
        statements: &[Stmt],
    ) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!(
            "DEBUG: compile_statements called with {} statements",
            statements.len()
        );
        for (i, stmt) in statements.iter().enumerate() {
            eprintln!(
                "DEBUG:   stmt[{}] type: {}",
                i,
                match stmt {
                    Stmt::CInclude(_) => "CInclude",
                    Stmt::EnumDecl { .. } => "EnumDecl",
                    Stmt::Declaration { .. } => "Declaration",
                    Stmt::Assignment { .. } => "Assignment",
                    Stmt::ExprStmt(_) => "ExprStmt",
                    Stmt::FnDecl { .. } => "FnDecl",
                    Stmt::Bundle { .. } => "Bundle",
                    Stmt::Import { .. } => "Import",
                    Stmt::Recipe { .. } => "Recipe",
                    Stmt::If { .. } => "If",
                    Stmt::While { .. } => "While",
                    Stmt::For { .. } => "For",
                    Stmt::Decorator { .. } => "Decorator",
                    Stmt::Return(_) => "Return",
                    Stmt::Block(_) => "Block",
                }
            );
        }

        let i32_type = self.context.i32_type();
        for stmt in statements {
            if let Stmt::Declaration { is_state, name, .. } = stmt {
                if *is_state {
                    let global_var = match self.module.get_global(name) {
                        Some(g) => g,
                        None => {
                            let g = self.module.add_global(i32_type, None, name);
                            g.set_initializer(&i32_type.const_int(0, false));
                            g
                        }
                    };

                    self.variables.insert(
                        name.clone(),
                        VariableInfo {
                            ptr: global_var.as_pointer_value(),
                            is_state: true,
                            kind: VariableKind::I32,
                        },
                    );
                }
            }
        }

        for stmt in statements {
            match stmt {
                Stmt::CInclude(path) => {
                    // Resolve relative includes against the current directory of the file being compiled.
                    let mut p = std::path::PathBuf::from(path);
                    if p.is_relative() {
                        p = self.current_dir.join(p);
                    }
                    let p = std::fs::canonicalize(&p).unwrap_or(p);
                    if let Some(names) = self.library_manager.load_c_header_collect(
                        p.to_string_lossy().as_ref(),
                        self.context,
                        &self.module,
                    ) {
                        if self.current_module_prefix.is_some() {
                            for n in names {
                                self.allowed_externs.insert(n);
                            }
                        }
                    }
                }
                Stmt::EnumDecl { .. } => {
                    // TODO
                }
                #[allow(unused)]
                Stmt::Import(path_parts) => {
                    let full_path = path_parts.join(".");
                    let module_prefix = path_parts.last().cloned().unwrap_or_default();

                    if full_path.starts_with("libc.") {
                        if let Some(names) = self.library_manager.load_c_header_collect(
                            &full_path,
                            self.context,
                            &self.module,
                        ) {
                            if self.current_module_prefix.is_some() {
                                for n in names {
                                    self.allowed_externs.insert(n);
                                }
                            }
                        }
                        continue;
                    }

                    let std_root =
                        std::env::var("KOME_STD_PATH").unwrap_or_else(|_| "./".to_owned());
                    let std_root = std::path::PathBuf::from(std_root);
                    let kome_file_path = resolve_std_module_file(&std_root, path_parts)
                        .unwrap_or_else(|| {
                            // Keep a stable, actionable error message for users.
                            let rel = path_parts.join("/");
                            panic!(
                                "Standard library not found at: {:?} or {:?}",
                                std_root.join(format!("{rel}.kome")),
                                std_root.join(rel).join("module.kome")
                            );
                        });

                    let source = std::fs::read_to_string(&kome_file_path).map_err(|_| {
                        format!("Failed to read standard library: {:?}", kome_file_path)
                    })?;

                    if let Some(parent) = kome_file_path.parent() {
                        self.current_dir = parent.to_path_buf();
                    }

                    let mut std_ast: Vec<Stmt> = Vec::new();

                    let pairs = match crate::KomeParser::parse(crate::Rule::program, &source) {
                        Ok(p) => p,
                        Err(e) => {
                            panic!(
                                "Failed to parse standard library file {:?}: {}",
                                kome_file_path, e
                            );
                        }
                    };

                    for pair in pairs {
                        match pair.as_rule() {
                            crate::Rule::program => {
                                for inner_pair in pair.into_inner() {
                                    if inner_pair.as_rule() == crate::Rule::stmt {
                                        let stmt = parse_stmt(inner_pair);
                                        std_ast.push(stmt);
                                    }
                                }
                            }
                            crate::Rule::stmt => {
                                let stmt = parse_stmt(pair);
                                std_ast.push(stmt);
                            }
                            crate::Rule::EOI => {}
                            _ => {}
                        }
                    }

                    // Namespace std modules by prefixing their function names with the module name.
                    // Example: `use std.io.keyboard` makes `fn scan(...)` available as `keyboard.scan(...)`
                    // by compiling it as `keyboard_scan`.
                    if !module_prefix.is_empty() {
                        for stmt in std_ast.iter_mut() {
                            if let Stmt::FnDecl { name, .. } = stmt {
                                if !name.contains('.')
                                    && !name.starts_with(&format!("{module_prefix}_"))
                                {
                                    *name = format!("{module_prefix}_{name}");
                                }
                            }
                        }
                    }

                    // Compile std module in an isolated "extern allowlist" scope so std code can
                    // only call C functions declared via `cinclude`/`use libc.*` inside that module.
                    let prev_prefix = self.current_module_prefix.take();
                    let prev_allowed = std::mem::take(&mut self.allowed_externs);
                    self.current_module_prefix = Some(module_prefix.clone());
                    self.allowed_externs = HashSet::new();

                    self.compile_statements(&std_ast)?;

                    self.current_module_prefix = prev_prefix;
                    self.allowed_externs = prev_allowed;
                }

                Stmt::FnDecl {
                    is_public: _,
                    name,
                    params,
                    is_variadic,
                    body,
                } => {
                    eprintln!(
                        "DEBUG compile_statements: FnDecl '{}' with {} body statements",
                        name,
                        body.len()
                    );
                    let previous_block = self.builder.get_insert_block();
                    let prev_variadic = self.current_fn_is_variadic;
                    self.current_fn_is_variadic = *is_variadic;

                    let void_type = self.context.void_type();
                    let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                    let i32_t = self.context.i32_type();

                    let mut llvm_param_types = Vec::new();
                    let mut param_kinds = Vec::new();
                    for p in params {
                        let ty = p.ty.as_str();
                        let (llvm_ty, kind) = match ty {
                            "Ptr" | "Any" | "String" => (ptr_t.into(), VariableKind::Ptr),
                            "Int" | "i32" => (i32_t.into(), VariableKind::I32),
                            _ => (i32_t.into(), VariableKind::I32),
                        };
                        llvm_param_types.push(llvm_ty);
                        param_kinds.push(kind);
                    }

                    let llvm_name = name.replace('.', "_");

                    // 同じ std モジュールを複数回 import した場合など、同名関数を再定義しない。
                    if let Some(existing) = self.module.get_function(&llvm_name) {
                        if existing.count_basic_blocks() > 0 {
                            debug!(
                                "Codegen: skipping duplicate function definition '{}'",
                                llvm_name
                            );
                            self.current_fn_is_variadic = prev_variadic;
                            if previous_block.is_none() {
                                self.builder.clear_insertion_position();
                            }
                            continue;
                        }
                    }

                    // `fn f(x: Any, ...) { printf(x, ...) }` のような「薄いラッパ」は
                    // 呼び出し側へ展開してしまえば、LLVM 側で va_list を扱わずに済む。
                    if *is_variadic && self.try_register_vararg_forwarder(&llvm_name, params, body)
                    {
                        debug!(
                            "Codegen: registered variadic forwarder for function '{}'",
                            llvm_name
                        );
                        // 関数本体は生成しない（呼び出し側で直接 target を呼ぶ）
                        self.current_fn_is_variadic = prev_variadic;
                        if previous_block.is_none() {
                            self.builder.clear_insertion_position();
                        }
                        continue;
                    }

                    let fn_type = void_type.fn_type(&llvm_param_types, *is_variadic);
                    let function = self.module.add_function(&llvm_name, fn_type, None);
                    let entry_block = self.context.append_basic_block(function, "entry");

                    self.variables.retain(|_, v| v.is_state);
                    self.builder.position_at_end(entry_block);

                    for (idx, p) in params.iter().enumerate() {
                        let kind = param_kinds.get(idx).copied().unwrap_or(VariableKind::I32);
                        let alloca = match kind {
                            VariableKind::I32 => self
                                .builder
                                .build_alloca(i32_t, &p.name)
                                .expect("alloca i32"),
                            VariableKind::Ptr => self
                                .builder
                                .build_alloca(ptr_t, &p.name)
                                .expect("alloca ptr"),
                        };
                        let arg = function.get_nth_param(idx as u32).expect("param");
                        self.builder.build_store(alloca, arg).expect("store param");
                        self.variables.insert(
                            p.name.clone(),
                            VariableInfo {
                                ptr: alloca,
                                is_state: false,
                                kind,
                            },
                        );
                    }

                    self.compile_statements(body)?;
                    self.current_fn_is_variadic = prev_variadic;

                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        self.builder
                            .build_return(None)
                            .expect("Failed to build void return");
                    }

                    if let Some(prev) = previous_block {
                        self.builder.position_at_end(prev);
                    } else {
                        debug!(
                            "Codegen: No previous block to return to after compiling function '{}'",
                            name
                        );
                        // トップレベル（挿入先ブロック無し）から関数を生成した場合、
                        // ここで挿入位置をクリアしておかないと以降のコード生成が
                        // 直前に生成した関数の末尾へ誤って挿入されてしまう。
                        self.builder.clear_insertion_position();
                    }
                }

                Stmt::ExprStmt(expr) => {
                    self.compile_expr(expr);
                }
                Stmt::Return(ret_expr) => {
                    if let Some(bb) = self.builder.get_insert_block() {
                        if bb.get_terminator().is_some() {
                            return Ok(());
                        }
                    }
                    if let Some(e) = ret_expr {
                        let _ = self.compile_expr(e);
                    }
                    self.builder.build_return(None).ok();
                }

                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let condition = self.compile_expr(condition);
                    let parent_func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    let then_bb = self.context.append_basic_block(parent_func, "then");
                    let else_bb = self.context.append_basic_block(parent_func, "else");
                    let merge_bb = self.context.append_basic_block(parent_func, "ifcont");

                    // 条件に応じて分岐
                    self.builder
                        .build_conditional_branch(condition.into_int_value(), then_bb, else_bb)
                        .expect("Failed to build conditional branch");

                    // thenブロックの構築
                    self.builder.position_at_end(then_bb);

                    if let Stmt::Bundle { body, .. } = &**then_body {
                        self.compile_statements(body)
                            .expect("Failed to compile then block statements");
                    } else {
                        self.compile_statements(&[*then_body.clone()])
                            .expect("Failed to compile then block statements");
                    }

                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(merge_bb)
                            .expect("Failed to build unconditional branch");
                    }

                    // elseブロックの構築
                    self.builder.position_at_end(else_bb);
                    if let Some(else_stmt_box) = else_body {
                        if let Stmt::Bundle { body, .. } = &**else_stmt_box {
                            self.compile_statements(body)
                                .expect("Failed to compile else block statements");
                        } else {
                            self.compile_statements(&[*else_stmt_box.clone()])
                                .expect("Failed to compile else block statements");
                        }
                    }

                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(merge_bb)
                            .expect("Failed to build unconditional branch");
                    }

                    // 合流
                    self.builder.position_at_end(merge_bb);
                }

                Stmt::Bundle {
                    name: bundle_name,
                    body,
                } => {
                    /* compile bundle body first */
                    self.compile_statements(body)?;

                    /* Then auto-generate run() method for every bundle */
                    let run_fn_name = format!("{}_run", bundle_name);
                    if self.module.get_function(&run_fn_name).is_none() {
                        let void_type = self.context.void_type();
                        let fn_type = void_type.fn_type(&[], false);
                        let run_function = self.module.add_function(&run_fn_name, fn_type, None);
                        let entry_block = self.context.append_basic_block(run_function, "entry");

                        let prev_block = self.builder.get_insert_block();
                        self.builder.position_at_end(entry_block);

                        /* Call the runtime start loop */
                        if let Some(loop_fn) = self.module.get_function("__kome_runtime_start_loop")
                        {
                            self.builder
                                .build_call(loop_fn, &[], "call_start_loop")
                                .ok();
                        }

                        self.builder.build_return(None).ok();

                        if let Some(prev) = prev_block {
                            self.builder.position_at_end(prev);
                        }

                        debug!("Codegen: Auto-generated run() for bundle '{}'", bundle_name);
                    }
                }

                #[allow(unused)]
                Stmt::Recipe {
                    is_public,
                    name,
                    state_deps,
                    body,
                } => {
                    debug!(
                        "Codegen: Compiling recipe '{}' (deps: {:?})",
                        name, state_deps
                    );

                    // レシピ登録（subscribe）は実行時に必要なので、トップレベルでも安全に
                    // コード生成できるよう `__kome_register` 関数へ集約する。
                    let register_fn = self.ensure_register_fn();
                    let register_entry = register_fn
                        .get_first_basic_block()
                        .expect("__kome_register entry");
                    let previous_block = self.builder.get_insert_block();

                    self.variables.retain(|_, v| v.is_state);

                    // 関数名は bundle 名とレシピ名を組み合わせて一意にする
                    let void_type = self.context.void_type();
                    let recipe_fn_type = void_type.fn_type(&[], false);
                    let recipe_fn_name = format!("App_recipe_{}", name);
                    let recipe_function =
                        self.module
                            .add_function(&recipe_fn_name, recipe_fn_type, None);

                    let recipe_entry_bb = self.context.append_basic_block(recipe_function, "entry");

                    self.builder.position_at_end(recipe_entry_bb);

                    let recipe_stmts: Vec<Stmt> = vec![Stmt::ExprStmt(body.clone())];
                    self.compile_statements(&recipe_stmts)?;

                    if recipe_entry_bb.get_terminator().is_none() {
                        self.builder
                            .build_return(None)
                            .expect("Failed to build return for recipe function");
                    }

                    if let Some(prev) = previous_block {
                        self.builder.position_at_end(prev);

                        // 元の挿入位置は一旦戻す（後で復元）
                    } else {
                        // トップレベルから呼ばれている場合は挿入位置が無いので後で復元しない
                    }

                    // __kome_register の末尾（ret の直前）に subscribe 呼び出しを挿入する
                    self.builder.position_at_end(register_entry);
                    if let Some(term) = register_entry.get_terminator() {
                        self.builder.position_before(&term);
                    }

                    let subscribe_fn = match self.module.get_function("__kome_runtime_subscribe") {
                        Some(f) => f,
                        None => {
                            let address_space = inkwell::AddressSpace::from(0);
                            let generic_ptr_type = self.context.ptr_type(address_space);
                            let sub_fn_type =
                                void_type.fn_type(&[generic_ptr_type.into(), generic_ptr_type.into()], false);
                            self.module
                                .add_function("__kome_runtime_subscribe", sub_fn_type, None)
                        }
                    };

                    for dep_var in state_deps {
                        let dep_var_global = self
                            .builder
                            .build_global_string_ptr(dep_var, "dep_var_name")
                            .expect("Failed to generate global string ptr");
                        let recipe_fn_ptr = recipe_function.as_global_value().as_pointer_value();
                        self.builder
                            .build_call(
                                subscribe_fn,
                                &[
                                    dep_var_global.as_pointer_value().into(),
                                    recipe_fn_ptr.into(),
                                ],
                                "subscribe_call",
                            )
                            .expect("Failed to build runtime subscribe call");
                        debug!(
                            "Codegen: Registered '{}' to look at state '{}'",
                            recipe_fn_name, dep_var
                        );
                    }

                    // もとの挿入位置に戻す
                    if let Some(prev) = previous_block {
                        self.builder.position_at_end(prev);
                    } else {
                        self.builder.clear_insertion_position();
                    }
                }

                Stmt::Assignment {
                    is_default: _,
                    name,
                    value,
                } => {
                    let short_name = name.split('.').last().unwrap().to_string();

                    let (ptr, is_state_target) = if let Some(var_info) = self.variables.get(name) {
                        (var_info.ptr, var_info.is_state)
                    } else if let Some(var_info) = self.variables.get(&short_name) {
                        (var_info.ptr, var_info.is_state)
                    } else if let Some(global_var) = self
                        .module
                        .get_global(name)
                        .or_else(|| self.module.get_global(&short_name))
                    {
                        (global_var.as_pointer_value(), false)
                    } else {
                        panic!(
                            "Undefined variable for assignment: {} (short: {})",
                            name, short_name
                        );
                    };

                    let rhs_val = self.compile_expr(value).into_int_value();
                    self.builder
                        .build_store(ptr, rhs_val)
                        .expect("Failed to store");

                    if is_state_target {
                        let void_t = self.context.void_type();
                        let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                        let emit_fn = match self.module.get_function("__kome_runtime_emit") {
                            Some(f) => f,
                            None => {
                                let fn_ty = void_t.fn_type(&[ptr_t.into()], false);
                                self.module.add_function("__kome_runtime_emit", fn_ty, None)
                            }
                        };

                        let var_name = self
                            .builder
                            .build_global_string_ptr(&short_name, "state_name")
                            .expect("state name");
                        self.builder
                            .build_call(
                                emit_fn,
                                &[var_name.as_pointer_value().into()],
                                "emit_state_change",
                            )
                            .expect("emit");
                    }
                }

                Stmt::While { condition, body } => {
                    let parent_func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    let cond_bb = self.context.append_basic_block(parent_func, "while_cond");
                    let body_bb = self.context.append_basic_block(parent_func, "while_body");
                    let end_bb = self.context.append_basic_block(parent_func, "while_end");

                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .expect("Failed to branch to while condition");

                    self.builder.position_at_end(cond_bb);
                    let cond_val = self.compile_expr(condition);
                    self.builder
                        .build_conditional_branch(cond_val.into_int_value(), body_bb, end_bb)
                        .expect("Failed to build while conditional branch");

                    self.builder.position_at_end(body_bb);
                    if let Stmt::Bundle {
                        body: body_stmts, ..
                    } = &**body
                    {
                        self.compile_statements(body_stmts)
                            .expect("Failed to compile while body statements");
                    } else {
                        self.compile_statements(&[*body.clone()])
                            .expect("Failed to compile while body statements");
                    }

                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .expect("Failed to loop back to while condition");

                    self.builder.position_at_end(end_bb);
                }

                Stmt::For {
                    init,
                    condition,
                    update,
                    body,
                } => {
                    let parent_func = self
                        .builder
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    // 条件式（i < end）の左辺からループ変数名（"i" など）を特定する
                    let loop_var_name = if let Expr::BinaryOp { left, .. } = condition {
                        if let Expr::Ident(name) = &**left {
                            name.clone()
                        } else {
                            panic!(
                                "Codegen Error: For loop condition left-hand side must be an identifier"
                            );
                        }
                    } else {
                        panic!("Codegen Error: Invalid for loop condition structure");
                    };

                    // 修正: VariableInfoの参照ではなく、内部の ptr (PointerValue) を直接コピーして受け取る
                    let loop_var_ptr = if let Some(info) = self.variables.get(&loop_var_name) {
                        info.ptr // コピーされるため、ここで self の借用は終わる
                    } else {
                        let i32_type = self.context.i32_type();
                        let new_alloc = self
                            .builder
                            .build_alloca(i32_type, &loop_var_name)
                            .expect("Failed to allocate for-loop variable");

                        self.variables.insert(
                            loop_var_name.clone(),
                            VariableInfo {
                                ptr: new_alloc,
                                is_state: false,
                                kind: VariableKind::I32,
                            },
                        );
                        new_alloc
                    };

                    // 修正: alloc_ptr.ptr ではなく、上で取り出した loop_var_ptr を使う
                    let start_val = self.compile_expr(init);
                    self.builder
                        .build_store(loop_var_ptr, start_val)
                        .expect("Failed to store for-loop init value");

                    let cond_bb = self.context.append_basic_block(parent_func, "for_cond");
                    let body_bb = self.context.append_basic_block(parent_func, "for_body");
                    let end_bb = self.context.append_basic_block(parent_func, "for_end");

                    // 条件チェックへジャンプ
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .expect("Failed to branch to for condition");

                    // 条件チェック
                    self.builder.position_at_end(cond_bb);
                    let cond_val = self.compile_expr(condition);
                    self.builder
                        .build_conditional_branch(cond_val.into_int_value(), body_bb, end_bb)
                        .expect("Failed to build for conditional branch");

                    // ループ本体
                    self.builder.position_at_end(body_bb);

                    let stmts_to_compile: Vec<Stmt> = match &**body {
                        Stmt::Bundle {
                            body: body_stmts, ..
                        } => body_stmts.clone(),
                        other_stmt => vec![other_stmt.clone()],
                    };

                    // これでもうどこからも self は借用されていないので、安全に呼び出せます！
                    self.compile_statements(&stmts_to_compile)
                        .expect("Failed to compile for loop body statements");
                    // インクリメント
                    if let Some(update_expr) = update {
                        let next_val = self.compile_expr(update_expr);
                        self.builder
                            .build_store(loop_var_ptr, next_val)
                            .expect("Failed to update for-loop counter");
                    }

                    // 条件チェックブロックへ戻る
                    self.builder
                        .build_unconditional_branch(cond_bb)
                        .expect("Failed to loop back to for condition");

                    self.builder.position_at_end(end_bb);

                    self.variables.remove(&loop_var_name);
                }

                _ => debug!("Codegen: Unknown statement: {:?}", stmt),
            }
        }
        Ok(())
    }

    /// 式（Expression）を評価し、LLVM の値（BasicValueEnum）を返す
    fn compile_expr(&mut self, expr: &Expr) -> inkwell::values::BasicValueEnum<'ctx> {
        match expr {
            Expr::Integer(val) => {
                // 整数リテラルをLLVMのi32に変換
                // TODO: ここも型推論
                let i32_type = self.context.i32_type();
                i32_type.const_int(*val as u64, false).as_basic_value_enum()
            }
            Expr::Ident(name) => {
                // Special case: "any" is a placeholder for event handlers
                if name == "any" {
                    let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                    return ptr_t.const_null().as_basic_value_enum();
                }

                let ptr = if let Some(var_info) = self.variables.get(name) {
                    var_info.ptr
                } else if let Some(global_var) = self.module.get_global(name) {
                    global_var.as_pointer_value()
                } else if let Some(short_name) = name.split('.').last() {
                    if let Some(global_var) = self.module.get_global(short_name) {
                        global_var.as_pointer_value()
                    } else {
                        panic!(
                            "Undefined variable: {} (tried short name: {})",
                            name, short_name
                        );
                    }
                } else {
                    panic!("Undefined variable: {}", name);
                };

                if let Some(var_info) = self.variables.get(name) {
                    match var_info.kind {
                        VariableKind::I32 => self
                            .builder
                            .build_load(self.context.i32_type(), ptr, name)
                            .expect("Failed to load variable"),
                        VariableKind::Ptr => {
                            let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                            self.builder
                                .build_load(ptr_t, ptr, name)
                                .expect("Failed to load ptr variable")
                        }
                    }
                } else {
                    self.builder
                        .build_load(self.context.i32_type(), ptr, name)
                        .expect("Failed to load variable")
                }
            }
            Expr::BinaryOp { left, op, right } => {
                // 左辺と右辺をそれぞれLLVM IRにする
                let left_val = self.compile_expr(left);
                let right_val = self.compile_expr(right);

                match op {
                    Op::Add => self
                        .builder
                        .build_int_add(
                            left_val.into_int_value(),
                            right_val.into_int_value(),
                            "addtmp",
                        )
                        .expect("Failed to build add instruction")
                        .as_basic_value_enum(),
                    Op::Sub => self
                        .builder
                        .build_int_sub(
                            left_val.into_int_value(),
                            right_val.into_int_value(),
                            "subtmp",
                        )
                        .expect("Failed to build sub instruction")
                        .as_basic_value_enum(),
                    Op::Mul => self
                        .builder
                        .build_int_mul(
                            left_val.into_int_value(),
                            right_val.into_int_value(),
                            "multmp",
                        )
                        .expect("Failed to build mul instruction")
                        .as_basic_value_enum(),
                    Op::Div => self
                        .builder
                        .build_int_signed_div(
                            left_val.into_int_value(),
                            right_val.into_int_value(),
                            "divtmp",
                        )
                        .expect("Failed to build div instruction")
                        .as_basic_value_enum(),
                    Op::Eq => {
                        // ==
                        self.builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "eqtmp",
                            )
                            .expect("Failed to build icmp eq instruction")
                            .as_basic_value_enum()
                    }
                    Op::Neq => {
                        // !=
                        self.builder
                            .build_int_compare(
                                IntPredicate::NE,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "netmp",
                            )
                            .expect("Failed to build icmp ne instruction")
                            .as_basic_value_enum()
                    }
                    Op::Lt => {
                        // <
                        self.builder
                            .build_int_compare(
                                IntPredicate::SLT,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "lttmp",
                            )
                            .expect("Failed to build icmp slt instruction")
                            .as_basic_value_enum()
                    }
                    Op::Gt => {
                        // >
                        self.builder
                            .build_int_compare(
                                IntPredicate::SGT,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "gttmp",
                            )
                            .expect("Failed to build icmp sgt instruction")
                            .as_basic_value_enum()
                    }
                    Op::Le => {
                        // <=
                        self.builder
                            .build_int_compare(
                                IntPredicate::SLE,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "letmp",
                            )
                            .expect("Failed to build icmp sle instruction")
                            .as_basic_value_enum()
                    }
                    Op::Ge => {
                        // >=
                        self.builder
                            .build_int_compare(
                                IntPredicate::SGE,
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "getmp",
                            )
                            .expect("Failed to build icmp sge instruction")
                            .as_basic_value_enum()
                    }
                    Op::And => {
                        // &&
                        self.builder
                            .build_and(
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "andtmp",
                            )
                            .expect("Failed to build logical and instruction")
                            .as_basic_value_enum()
                    }
                    Op::Or => {
                        // ||
                        self.builder
                            .build_or(
                                left_val.into_int_value(),
                                right_val.into_int_value(),
                                "ortmp",
                            )
                            .expect("Failed to build logical or instruction")
                            .as_basic_value_enum()
                    }
                    Op::In => {
                        // TODO: 実装
                        todo!("Codegen: 'in' operator is not yet implemented.")
                    }
                    Op::Question => {
                        // TODO: Null合体演算子実装
                        todo!("Codegen: '??' operator is not yet implemented.")
                    }
                    Op::With => {
                        // stdlib では `args with "\n"` のような「末尾付与」のために使っている。
                        // 文字列の動的結合は未実装なので、現状は左辺をそのまま返す（no-op）。
                        left_val
                    }
                }
            }
            Expr::Block(stmts) => {
                self.compile_statements(&*stmts.clone())
                    .expect("Failed to compile block statements");
                self.context
                    .i32_type()
                    .const_int(0, false)
                    .as_basic_value_enum()
            }
            Expr::CallChain { head, tails } => {
                /* Handle bundle.method() calls (e.g., App.run()) */
                eprintln!(
                    "DEBUG: compile_expr CallChain head={}, tails.len={}",
                    head,
                    tails.len()
                );
                for (i, tail) in tails.iter().enumerate() {
                    eprintln!("DEBUG:   tail[{}] = {:?}", i, tail);
                }

                // ここでは `print` や `io.print` のような特別扱い（ハードコード）はしない。
                // 可変長引数の転送（`...`）は、通常の関数呼び出しとして処理できるように
                // `printf(fmt, ...)` を `vprintf(fmt, va_list)` へ lower する形で実装する。

                if tails.len() >= 2 {
                    eprintln!("DEBUG: Found CallChain with {} tails", tails.len());
                    if let (
                        ast::Accessor::Property(method_name),
                        ast::Accessor::Method(args, trailing_closure),
                    ) = (&tails[0], &tails[1])
                    {
                        eprintln!("DEBUG: Property + Method found: {}.{}()", head, method_name);
                        let bundle_name = head;
                        let fn_name = format!("{}_{}", bundle_name, method_name);
                        eprintln!("DEBUG: Looking for function: {}", fn_name);
                        // 変数長引数ラッパ（例: `io_println`）は、呼び出し側で target（例: `printf`）へ展開する
                        if let Some(fwd) = self.vararg_forwarders.get(&fn_name).cloned() {
                            let target_fn = self
                                .module
                                .get_function(&fwd.target)
                                .unwrap_or_else(|| {
                                    let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                                    let fn_ty =
                                        self.context.i32_type().fn_type(&[ptr_t.into()], true);
                                    self.module.add_function(&fwd.target, fn_ty, None)
                                });

                            let mut lowered: Vec<
                                inkwell::values::BasicMetadataValueEnum<'ctx>,
                            > = Vec::new();

                            if !args.is_empty() {
                                if let Some(suf) = &fwd.suffix {
                                    if let Expr::String(s) = &args[0] {
                                        lowered.push(
                                            self.compile_expr(&Expr::String(format!("{s}{suf}")))
                                                .into(),
                                        );
                                    } else {
                                        lowered.push(self.compile_expr(&args[0]).into());
                                    }
                                } else {
                                    lowered.push(self.compile_expr(&args[0]).into());
                                }
                                for a in args.iter().skip(1) {
                                    lowered.push(self.compile_expr(a).into());
                                }
                            }

                            let call = self
                                .builder
                                .build_call(target_fn, &lowered, "call_forwarded")
                                .expect("call forwarder target");

                            if let Some(suf) = &fwd.suffix {
                                if !matches!(args.get(0), Some(Expr::String(_))) {
                                    let suf_val = self.compile_expr(&Expr::String(suf.clone()));
                                    let _ = self
                                        .builder
                                        .build_call(
                                            target_fn,
                                            &[inkwell::values::BasicMetadataValueEnum::from(
                                                suf_val,
                                            )],
                                            "call_suffix",
                                        )
                                        .ok();
                                }
                            }

                            return match call.try_as_basic_value() {
                                ValueKind::Basic(val) => val,
                                ValueKind::Instruction(_) => self
                                    .context
                                    .i32_type()
                                    .const_int(0, false)
                                    .as_basic_value_enum(),
                            };
                        }

                        if let Some(function) = self.module.get_function(&fn_name) {
                            eprintln!("DEBUG: Found function: {}", fn_name);
                            if function.count_basic_blocks() == 0 {
                                if self.current_module_prefix.is_some()
                                    && !self.allowed_externs.contains(&fn_name)
                                {
                                    panic!(
                                        "Std module tried to call C function '{}' without declaring it via `cinclude` (or `use libc.*`) in that std file.",
                                        fn_name
                                    );
                                }
                            }
                            let mut llvm_args = Vec::new();
                            for arg in args {
                                let val = self.compile_expr(arg);
                                llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(val));
                            }

                            // If there is a trailing closure, compile it and add closure pointer as argument
                            if let Some(block_stmts) = trailing_closure {
                                let mut i = 0;
                                let closure_name = loop {
                                    let name = format!("__kome_anon_closure_{}", i);
                                    if self.module.get_function(&name).is_none() {
                                        break name;
                                    }
                                    i += 1;
                                };

                                let void_type = self.context.void_type();
                                let closure_fn_type = void_type.fn_type(&[], false);
                                let closure_function =
                                    self.module
                                        .add_function(&closure_name, closure_fn_type, None);

                                let entry_bb =
                                    self.context.append_basic_block(closure_function, "entry");
                                let current_bb = self.builder.get_insert_block().unwrap();
                                self.builder.position_at_end(entry_bb);

                                self.compile_statements(block_stmts)
                                    .expect("Failed to compile trailing closure body");

                                self.builder
                                    .build_return(None)
                                    .expect("Failed to build return for closure");
                                self.builder.position_at_end(current_bb);

                                let closure_ptr =
                                    closure_function.as_global_value().as_pointer_value();
                                llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(
                                    closure_ptr,
                                ));
                            }

                            let call = self
                                .builder
                                .build_call(function, &llvm_args, "calltmp")
                                .expect("Codegen: Failed to build function call");

                            match call.try_as_basic_value() {
                                ValueKind::Basic(val) => {
                                    return val;
                                }
                                ValueKind::Instruction(_) => {
                                    return self
                                        .context
                                        .i32_type()
                                        .const_int(0, false)
                                        .as_basic_value_enum();
                                }
                            }
                        }
                    }
                }

                if let Some(ast::Accessor::Method(args, trailing_closure)) = tails.first() {
                    if args.last().is_some_and(|e| matches!(e, Expr::VarArgs)) {
                        panic!("`...` の転送は現在サポートしていません。");
                    }

                    let mut fn_name = head.clone();
                    let lookup_name = fn_name.replace('.', "_");

                    if self.module.get_function(&lookup_name).is_none() {
                        if let Some(method_name) = head.split('.').last() {
                            let fallback_name = format!("__bundle_{}", method_name);
                            if self.module.get_function(&fallback_name).is_some() {
                                fn_name = fallback_name;
                            } else {
                                fn_name = lookup_name;
                            }
                        } else {
                            fn_name = lookup_name;
                        }
                    } else {
                        fn_name = lookup_name;
                    }

                    // 変数長引数ラッパ（例: `io_println`）は、呼び出し側で target（例: `printf`）へ展開する
                    if let Some(fwd) = self.vararg_forwarders.get(&fn_name).cloned() {
                        let target_fn = self
                            .module
                            .get_function(&fwd.target)
                            .unwrap_or_else(|| {
                                let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                                let fn_ty = self.context.i32_type().fn_type(&[ptr_t.into()], true);
                                self.module.add_function(&fwd.target, fn_ty, None)
                            });

                        let mut lowered: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                            Vec::new();

                        if !args.is_empty() {
                            if let Some(suf) = &fwd.suffix {
                                if let Expr::String(s) = &args[0] {
                                    lowered.push(
                                        self.compile_expr(&Expr::String(format!("{s}{suf}")))
                                            .into(),
                                    );
                                } else {
                                    lowered.push(self.compile_expr(&args[0]).into());
                                }
                            } else {
                                lowered.push(self.compile_expr(&args[0]).into());
                            }
                            for a in args.iter().skip(1) {
                                lowered.push(self.compile_expr(a).into());
                            }
                        }

                        let call = self
                            .builder
                            .build_call(target_fn, &lowered, "call_forwarded")
                            .expect("call forwarder target");

                        if let Some(suf) = &fwd.suffix {
                            if !matches!(args.get(0), Some(Expr::String(_))) {
                                let suf_val = self.compile_expr(&Expr::String(suf.clone()));
                                let _ = self
                                    .builder
                                    .build_call(
                                        target_fn,
                                        &[inkwell::values::BasicMetadataValueEnum::from(suf_val)],
                                        "call_suffix",
                                    )
                                    .ok();
                            }
                        }

                        return match call.try_as_basic_value() {
                            ValueKind::Basic(val) => val,
                            ValueKind::Instruction(_) => self
                                .context
                                .i32_type()
                                .const_int(0, false)
                                .as_basic_value_enum(),
                        };
                    }

                    let function = self
                        .module
                        .get_function(&fn_name)
                        .expect(&format!("Undefined function: {}", head));
                    if function.count_basic_blocks() == 0 {
                        if self.current_module_prefix.is_some()
                            && !self.allowed_externs.contains(&fn_name)
                        {
                            panic!(
                                "Std module tried to call C function '{}' without declaring it via `cinclude` (or `use libc.*`) in that std file.",
                                fn_name
                            );
                        }
                    }

                    let mut llvm_args = Vec::new();

                    for arg in args {
                        let val = self.compile_expr(arg);
                        llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(val));
                    }

                    if let Some(block_stmts) = trailing_closure {
                        let mut i = 0;
                        let closure_name = loop {
                            let name = format!("__kome_anon_closure_{}", i);
                            if self.module.get_function(&name).is_none() {
                                break name;
                            }
                            i += 1;
                        };

                        let void_type = self.context.void_type();
                        let closure_fn_type = void_type.fn_type(&[], false);
                        let closure_function =
                            self.module
                                .add_function(&closure_name, closure_fn_type, None);

                        let entry_bb = self.context.append_basic_block(closure_function, "entry");
                        let current_bb = self.builder.get_insert_block().unwrap();
                        self.builder.position_at_end(entry_bb);

                        self.compile_statements(block_stmts)
                            .expect("Failed to compile trailing closure body");

                        self.builder
                            .build_return(None)
                            .expect("Failed to build return for closure");
                        self.builder.position_at_end(current_bb);

                        let closure_ptr = closure_function.as_global_value().as_pointer_value();
                        llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(closure_ptr));
                    }

                    let call = self
                        .builder
                        .build_call(function, &llvm_args, "calltmp")
                        .expect("Codegen: Failed to build function call");

                    match call.try_as_basic_value() {
                        ValueKind::Basic(val) => val,
                        ValueKind::Instruction(_) => self
                            .context
                            .i32_type()
                            .const_int(0, false)
                            .as_basic_value_enum(),
                    }
                } else {
                    // 未定義の関数は panic させず、警告を出して 0 を返す。
                    eprintln!(
                        "Codegen: Undefined function when resolving callchain: {}. Returning 0.",
                        head
                    );
                    return self
                        .context
                        .i32_type()
                        .const_int(0, false)
                        .as_basic_value_enum();
                }
            }

            Expr::String(s) => {
                let unescaped = s.replace("\\n", "\n");
                let global_str_ptr = self
                    .builder
                    .build_global_string_ptr(&unescaped, "str_literal")
                    .expect("Codegen: Failed to get global string");

                global_str_ptr.as_basic_value_enum()
            } // TODO: 文字列などはまた実装
            Expr::VarArgs => {
                // 可変長引数（`...`）の「転送」は未実装だが、stdlib をパース/コンパイルできるよう
                // AST としては受け付ける。値としてはダミーを返す。
                self.context
                    .i32_type()
                    .const_int(0, false)
                    .as_basic_value_enum()
            }
        }
    }

    /// 関数をコンパイルする
    #[allow(unused)]
    fn compile_function(&mut self, name: &str, body: &[Stmt]) {
        let func =
            self.module
                .add_function(name, self.context.void_type().fn_type(&[], false), None);
        let bb = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(bb);

        self.compile_statements(body).unwrap();

        self.builder
            .build_return(Some(&self.context.i32_type().const_int(0, false)))
            .unwrap();
    }
}
