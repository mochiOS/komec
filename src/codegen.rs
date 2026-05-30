use crate::ast;
use crate::ast::{Expr, Op, Stmt, parse_stmt};
use crate::library::LibraryManager;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, PointerValue, ValueKind};
use log::debug;
use pest::Parser;
use std::collections::HashMap;
use std::collections::HashSet;

/// `!default` のためのスコープ情報
#[derive(Clone, Debug, Default)]
pub(crate) struct DefaultScope<'ctx> {
    /// default 値の一時置き場（`!default` 実行時に評価して格納する）
    default_slots: HashMap<String, PointerValue<'ctx>>,
    /// `!default` が実行されたか（分岐内でも成立させる）
    active_flags: HashMap<String, PointerValue<'ctx>>,
    /// 通常代入が発生したか（制御フローを跨ぐため i1 の alloca を使う）
    assigned_flags: HashMap<String, PointerValue<'ctx>>,
}

fn resolve_module_file(root: &std::path::Path, path_parts: &[String]) -> Option<std::path::PathBuf> {
    let rel = path_parts.join("/");
    let direct = root.join(format!("{rel}.kome"));
    if direct.exists() {
        return Some(direct);
    }
    let module = root.join(rel).join("module.kome");
    if module.exists() {
        return Some(module);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::resolve_module_file;
    use super::CodegenContext;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn resolves_module_kome_when_direct_file_missing() {
        let root = std::env::temp_dir().join("kome_std_resolve_test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("std/io")).unwrap();
        fs::write(root.join("std/io/module.kome"), "fn hello() {}").unwrap();

        let parts = vec!["std".to_string(), "io".to_string()];
        let resolved = resolve_module_file(&root, &parts).unwrap();
        assert_eq!(resolved, PathBuf::from(root.join("std/io/module.kome")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unescape_string_literal_supports_quotes_and_backslash() {
        assert_eq!(CodegenContext::unescape_string_literal("\\\""), "\"");
        assert_eq!(CodegenContext::unescape_string_literal("\\\\"), "\\");
        assert_eq!(CodegenContext::unescape_string_literal("a\\nb"), "a\nb");
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
    pub register_fn: Option<inkwell::values::FunctionValue<'ctx>>,
    pub fn_params: HashMap<String, Vec<ast::FnParam>>,
    pub current_return: Option<ReturnKind>,
    /// `!default` の適用単位（関数/クロージャ）をスタックで管理
    pub(crate) default_scopes: Vec<DefaultScope<'ctx>>,
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
    Bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnKind {
    Void,
    I32,
    Ptr,
    Bool,
    OptI32,
    OptPtr,
}

impl<'a, 'ctx> CodegenContext<'a, 'ctx> {
    fn unescape_string_literal(raw: &str) -> String {
        // 最低限のエスケープだけ扱う（ViewKit の JSON を書けるのが目的）
        let mut out = String::with_capacity(raw.len());
        let mut it = raw.chars().peekable();
        while let Some(ch) = it.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    // 未定義はそのまま（例: \u は未対応）
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        out
    }

    fn push_default_scope(&mut self) {
        self.default_scopes.push(DefaultScope::default());
    }

    fn pop_default_scope_apply(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(scope) = self.default_scopes.pop() else {
            return Ok(());
        };

        // すでに return 等でブロックが閉じている場合、末尾到達しないので適用しない
        let Some(bb) = self.builder.get_insert_block() else {
            return Ok(());
        };
        if bb.get_terminator().is_some() {
            return Ok(());
        }

        // スコープ末尾で「!default が実行され」「通常代入が無かった変数」にだけ default を適用する
        for (name, slot_ptr) in scope.default_slots {
            let Some(active_ptr) = scope.active_flags.get(&name).copied() else {
                continue;
            };
            let Some(assigned_ptr) = scope.assigned_flags.get(&name).copied() else {
                continue;
            };

            let active = self
                .builder
                .build_load(self.context.bool_type(), active_ptr, "default_active")
                .expect("load default active")
                .into_int_value();
            let assigned = self
                .builder
                .build_load(self.context.bool_type(), assigned_ptr, "default_assigned")
                .expect("load default assigned")
                .into_int_value();
            let not_assigned = self
                .builder
                .build_not(assigned, "default_not_assigned")
                .expect("not assigned");
            let should_apply = self
                .builder
                .build_and(active, not_assigned, "default_should_apply")
                .expect("and default");

            let parent = self
                .builder
                .get_insert_block()
                .expect("default apply requires insert block")
                .get_parent()
                .expect("default apply requires parent func");

            let then_bb = self.context.append_basic_block(parent, "default_apply");
            let cont_bb = self.context.append_basic_block(parent, "default_cont");

            self.builder
                .build_conditional_branch(should_apply, then_bb, cont_bb)
                .expect("default br");

            self.builder.position_at_end(then_bb);
            // 型に合わせた load を行うため、専用ヘルパーに任せる
            self.codegen_assignment_store_from_slot(&name, slot_ptr)?;
            if self
                .builder
                .get_insert_block()
                .and_then(|bb| bb.get_terminator())
                .is_none()
            {
                self.builder
                    .build_unconditional_branch(cont_bb)
                    .expect("default br cont");
            }

            self.builder.position_at_end(cont_bb);
        }

        Ok(())
    }

    /// 変数名を「代入が解決される名前」に正規化する（短縮名フォールバックを含む）
    fn canonical_var_name(&self, name: &str) -> String {
        if self.variables.contains_key(name) || self.module.get_global(name).is_some() {
            return name.to_string();
        }
        let short = name.split('.').last().unwrap_or(name);
        if self.variables.contains_key(short) || self.module.get_global(short).is_some() {
            return short.to_string();
        }
        // 解決不能でも、エラーメッセージを分かりやすくするため原文を返す
        name.to_string()
    }

    /// エントリブロックに alloca を作る（分岐内で作ると後で支配関係で詰む）
    fn build_alloca_in_entry<T: BasicType<'ctx>>(
        &self,
        ty: T,
        name: &str,
    ) -> PointerValue<'ctx> {
        let entry = self
            .builder
            .get_insert_block()
            .expect("alloca requires insert block")
            .get_parent()
            .expect("alloca requires parent func")
            .get_first_basic_block()
            .expect("function has entry");

        let tmp = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(inst) => tmp.position_before(&inst),
            None => tmp.position_at_end(entry),
        }
        tmp.build_alloca(ty, name).expect("alloca in entry")
    }

    fn build_bool_alloca_in_entry(&self, name: &str, init: bool) -> PointerValue<'ctx> {
        let p = self.build_alloca_in_entry(self.context.bool_type(), name);
        let entry = self
            .builder
            .get_insert_block()
            .expect("init requires insert block")
            .get_parent()
            .expect("init requires parent func")
            .get_first_basic_block()
            .expect("function has entry");
        let tmp = self.context.create_builder();
        tmp.position_at_end(entry);
        if let Some(term) = entry.get_terminator() {
            tmp.position_before(&term);
        }
        let v = self
            .context
            .bool_type()
            .const_int(if init { 1 } else { 0 }, false);
        tmp.build_store(p, v).expect("init bool flag");
        p
    }

    fn resolve_assignment_target(
        &self,
        name: &str,
    ) -> (PointerValue<'ctx>, bool, VariableKind, String) {
        let short_name = name.split('.').last().unwrap_or(name).to_string();

        if let Some(info) = self.variables.get(name) {
            return (info.ptr, info.is_state, info.kind, short_name);
        }
        if let Some(info) = self.variables.get(&short_name) {
            return (info.ptr, info.is_state, info.kind, short_name);
        }
        if let Some(g) = self.module.get_global(name).or_else(|| self.module.get_global(&short_name)) {
            return (g.as_pointer_value(), false, VariableKind::I32, short_name);
        }

        panic!("Undefined variable for assignment: {} (short: {})", name, short_name);
    }

    fn normalize_value_for_kind(
        &self,
        kind: VariableKind,
        v: inkwell::values::BasicValueEnum<'ctx>,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        match kind {
            VariableKind::I32 => {
                let iv = v.into_int_value();
                if iv.get_type().get_bit_width() == 1 {
                    let z = self
                        .builder
                        .build_int_z_extend(iv, self.context.i32_type(), "bool_to_i32")
                        .expect("zext");
                    z.as_basic_value_enum()
                } else {
                    iv.as_basic_value_enum()
                }
            }
            VariableKind::Bool => self.to_bool(v).as_basic_value_enum(),
            VariableKind::Ptr => v.into_pointer_value().as_basic_value_enum(),
        }
    }

    fn codegen_assignment_store_value(
        &mut self,
        canonical_name: &str,
        value: inkwell::values::BasicValueEnum<'ctx>,
        mark_assigned: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (ptr, is_state_target, kind, short_name) = self.resolve_assignment_target(canonical_name);
        let v = self.normalize_value_for_kind(kind, value);

        match kind {
            VariableKind::I32 | VariableKind::Bool => {
                self.builder
                    .build_store(ptr, v.into_int_value())
                    .expect("store assignment");
            }
            VariableKind::Ptr => {
                self.builder
                    .build_store(ptr, v.into_pointer_value())
                    .expect("store assignment");
            }
        }

        if mark_assigned {
            if let Some(scope) = self.default_scopes.last() {
                if let Some(flag_ptr) = scope.assigned_flags.get(canonical_name).copied() {
                    self.builder
                        .build_store(flag_ptr, self.context.bool_type().const_int(1, false))
                        .expect("mark default assigned");
                }
            }
        }

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

        Ok(())
    }

    fn codegen_assignment_store_from_slot(
        &mut self,
        canonical_name: &str,
        slot_ptr: PointerValue<'ctx>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_, _, kind, _) = self.resolve_assignment_target(canonical_name);
        let loaded = match kind {
            VariableKind::I32 => self
                .builder
                .build_load(self.context.i32_type(), slot_ptr, "default_load_i32")
                .expect("load default i32")
                .as_basic_value_enum(),
            VariableKind::Bool => self
                .builder
                .build_load(self.context.bool_type(), slot_ptr, "default_load_bool")
                .expect("load default bool")
                .as_basic_value_enum(),
            VariableKind::Ptr => self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::from(0)),
                    slot_ptr,
                    "default_load_ptr",
                )
                .expect("load default ptr")
                .as_basic_value_enum(),
        };
        self.codegen_assignment_store_value(canonical_name, loaded, false)
    }

    fn parse_return_kind(name: Option<&str>) -> ReturnKind {
        let s = name.unwrap_or("none").trim();
        if let Some(inner) = s.strip_suffix('?') {
            return match inner.trim() {
                "Int" | "i32" | "int" => ReturnKind::OptI32,
                "Ptr" | "Any" | "String" | "string" => ReturnKind::OptPtr,
                _ => ReturnKind::Void,
            };
        }
        match s {
            "none" | "None" | "Void" => ReturnKind::Void,
            "Int" | "i32" | "int" => ReturnKind::I32,
            "Ptr" | "Any" | "String" | "string" => ReturnKind::Ptr,
            "Bool" | "bool" => ReturnKind::Bool,
            _ => ReturnKind::Void,
        }
    }

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
    // ここでは「C の可変長引数への転送」は扱わない（Kome の可変長だけを正式機能にする方針）。
    /// 複数の文（Statements）を順番に LLVM 命令に変換する
    pub fn compile_statements(
        &mut self,
        statements: &[Stmt],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // デバッグ出力は環境変数で明示的に有効化する
        if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
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
                        Stmt::Match { .. } => "Match",
                        Stmt::Is { .. } => "Is",
                    }
                );
            }
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
                Stmt::Declaration {
                    is_public: _,
                    is_state,
                    is_mut: _,
                    name,
                    value,
                    range: _,
                } => {
                    // state はグローバル、通常の let はローカル変数として扱う
                    if *is_state {
                        let i32_t = self.context.i32_type();
                        let global = match self.module.get_global(name) {
                            Some(g) => g,
                            None => {
                                let g = self.module.add_global(i32_t, None, name);
                                g.set_initializer(&i32_t.const_int(0, false));
                                g
                            }
                        };
                        self.variables.insert(
                            name.clone(),
                            VariableInfo {
                                ptr: global.as_pointer_value(),
                                is_state: true,
                                kind: VariableKind::I32,
                            },
                        );

                        // 初期値が整数リテラルなら initializer に反映
                        if let Expr::Integer(v) = value {
                            global.set_initializer(&i32_t.const_int(*v as u64, false));
                        } else if self.builder.get_insert_block().is_some() {
                            // 実行時に代入する（式の評価が必要）
                            let rhs = self.compile_expr(value).into_int_value();
                            self.builder
                                .build_store(global.as_pointer_value(), rhs)
                                .expect("store state init");
                        }
                    } else {
                        // ローカル変数: 現在の挿入ブロックが必要
                        let Some(_) = self.builder.get_insert_block() else {
                            panic!("ローカル変数 '{}' は関数の中で宣言してください。", name);
                        };

                        let init = self.compile_expr(value);
                        match init {
                            inkwell::values::BasicValueEnum::IntValue(iv) => {
                                if iv.get_type().get_bit_width() == 1 {
                                    let alloca = self
                                        .builder
                                        .build_alloca(self.context.bool_type(), name)
                                        .expect("alloca local bool");
                                    self.builder.build_store(alloca, iv).expect("store local");
                                    self.variables.insert(
                                        name.clone(),
                                        VariableInfo {
                                            ptr: alloca,
                                            is_state: false,
                                            kind: VariableKind::Bool,
                                        },
                                    );
                                    continue;
                                }
                                let alloca = self
                                    .builder
                                    .build_alloca(self.context.i32_type(), name)
                                    .expect("alloca local i32");
                                self.builder.build_store(alloca, iv).expect("store local");
                                self.variables.insert(
                                    name.clone(),
                                    VariableInfo {
                                        ptr: alloca,
                                        is_state: false,
                                        kind: VariableKind::I32,
                                    },
                                );
                            }
                            inkwell::values::BasicValueEnum::PointerValue(pv) => {
                                let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                                let alloca = self
                                    .builder
                                    .build_alloca(ptr_t, name)
                                    .expect("alloca local ptr");
                                self.builder.build_store(alloca, pv).expect("store local");
                                self.variables.insert(
                                    name.clone(),
                                    VariableInfo {
                                        ptr: alloca,
                                        is_state: false,
                                        kind: VariableKind::Ptr,
                                    },
                                );
                            }
                            _ => {
                                panic!("未対応の初期化式です: {}", name);
                            }
                        }
                    }
                }
                #[allow(unused)]
                Stmt::Import(path_parts) => {
                    let full_path = path_parts.join(".");
                    // `viewKit.window.*` のように「親モジュール名を含む名前」で呼びたいケースがある。
                    // - `use viewKit` 単体は prelude 扱いで prefix を付けない（後段の特例）
                    // - `use viewKit.window` は `viewKit_window_*` を生成して `viewKit.window.*` と一致させる
                    let mut module_prefix = path_parts.last().cloned().unwrap_or_default();
                    if path_parts.len() >= 2 && path_parts[0] == "viewKit" && path_parts[1] == "window" {
                        module_prefix = "viewKit_window".to_string();
                    }
                    if path_parts.len() >= 2 && path_parts[0] == "viewKit" && path_parts[1] == "handler" {
                        module_prefix = "viewKit_handler".to_string();
                    }

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

                    // std 以外のモジュールは lib 配下も探索する（例: `use viewKit`）
                    let lib_root =
                        std::env::var("KOME_LIB_PATH").unwrap_or_else(|_| "./lib".to_owned());
                    let lib_root = std::path::PathBuf::from(lib_root);

                    // lib/* は標準ライブラリと同じ形式で解決する（lib/viewKit/module.kome など）
                    if full_path.starts_with("lib.") {
                        let parts = &path_parts[1..];
                        let lib_root =
                            std::env::var("KOME_LIB_PATH").unwrap_or_else(|_| "./lib".to_owned());
                        let lib_root = std::path::PathBuf::from(lib_root);
                        let kome_file_path = resolve_module_file(&lib_root, parts)
                            .unwrap_or_else(|| {
                                let rel = parts.join("/");
                                panic!(
                                    "Library module not found at: {:?} or {:?}",
                                    lib_root.join(format!("{rel}.kome")),
                                    lib_root.join(rel).join("module.kome")
                                );
                            });

                        let source = std::fs::read_to_string(&kome_file_path).map_err(|_| {
                            format!("Failed to read library module: {:?}", kome_file_path)
                        })?;

                        if let Some(parent) = kome_file_path.parent() {
                            self.current_dir = parent.to_path_buf();
                        }

                        let mut lib_ast: Vec<Stmt> = Vec::new();
                        let pairs = match crate::KomeParser::parse(crate::Rule::program, &source) {
                            Ok(p) => p,
                            Err(e) => {
                                panic!(
                                    "Failed to parse library module file {:?}: {}",
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
                                            lib_ast.push(stmt);
                                        }
                                    }
                                }
                                crate::Rule::stmt => {
                                    let stmt = parse_stmt(pair);
                                    lib_ast.push(stmt);
                                }
                                crate::Rule::EOI => {}
                                _ => {}
                            }
                        }

                        // import されたモジュール名で関数を名前空間化する
                        if !module_prefix.is_empty() {
                            for stmt in lib_ast.iter_mut() {
                                if let Stmt::FnDecl { name, .. } = stmt {
                                    if !name.contains('.')
                                        && !name.starts_with(&format!("{module_prefix}_"))
                                    {
                                        *name = format!("{module_prefix}_{name}");
                                    }
                                }
                            }
                        }

                        // lib モジュールも「C 呼び出しの明示」を守る
                        let prev_prefix = self.current_module_prefix.take();
                        let prev_allowed = std::mem::take(&mut self.allowed_externs);
                        self.current_module_prefix = Some(module_prefix.clone());
                        self.allowed_externs = HashSet::new();

                        if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                            eprintln!("DEBUG: compiling lib module {:?}", kome_file_path);
                        }
                        self.compile_statements(&lib_ast)?;
                        if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                            eprintln!("DEBUG: finished lib module {:?}", kome_file_path);
                        }

                        self.current_module_prefix = prev_prefix;
                        self.allowed_externs = prev_allowed;

                        continue;
                    }

                    // `use viewKit` / `use toml` のように lib 直下を名前で参照できるようにする
                    if !full_path.starts_with("std.") && !full_path.starts_with("libc.") {
                        if let Some(kome_file_path) = resolve_module_file(&lib_root, path_parts) {
                            let source = std::fs::read_to_string(&kome_file_path).map_err(|_| {
                                format!("Failed to read library module: {:?}", kome_file_path)
                            })?;

                            if let Some(parent) = kome_file_path.parent() {
                                self.current_dir = parent.to_path_buf();
                            }

                            let mut lib_ast: Vec<Stmt> = Vec::new();
                            let pairs =
                                match crate::KomeParser::parse(crate::Rule::program, &source) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        panic!(
                                            "Failed to parse library module file {:?}: {}",
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
                                                lib_ast.push(stmt);
                                            }
                                        }
                                    }
                                    crate::Rule::stmt => {
                                        let stmt = parse_stmt(pair);
                                        lib_ast.push(stmt);
                                    }
                                    crate::Rule::EOI => {}
                                    _ => {}
                                }
                            }

                            // lib 直下のトップレベル import は「そのままの名前」で公開したい
                            // 例: `use viewKit` で `window.create()` や `card()` が見える前提
                            // なので、このケースは prefix を付けない。
                            if !module_prefix.is_empty() && path_parts.len() != 1 {
                                for stmt in lib_ast.iter_mut() {
                                    if let Stmt::FnDecl { name, .. } = stmt {
                                        if !name.contains('.')
                                            && !name.starts_with(&format!("{module_prefix}_"))
                                        {
                                            *name = format!("{module_prefix}_{name}");
                                        }
                                    }
                                }
                            }

                            let prev_prefix = self.current_module_prefix.take();
                            let prev_allowed = std::mem::take(&mut self.allowed_externs);
                            self.current_module_prefix = Some(module_prefix.clone());
                            self.allowed_externs = HashSet::new();

                            if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                                eprintln!("DEBUG: compiling lib module {:?}", kome_file_path);
                            }
                            self.compile_statements(&lib_ast)?;
                            if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                                eprintln!("DEBUG: finished lib module {:?}", kome_file_path);
                            }

                            self.current_module_prefix = prev_prefix;
                            self.allowed_externs = prev_allowed;
                            continue;
                        }
                    }

                    let std_root =
                        std::env::var("KOME_STD_PATH").unwrap_or_else(|_| "./".to_owned());
                    let std_root = std::path::PathBuf::from(std_root);
                    let kome_file_path = resolve_module_file(&std_root, path_parts)
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

                    if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                        eprintln!("DEBUG: compiling std module {:?}", kome_file_path);
                    }
                    self.compile_statements(&std_ast)?;
                    if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                        eprintln!("DEBUG: finished std module {:?}", kome_file_path);
                    }

                    self.current_module_prefix = prev_prefix;
                    self.allowed_externs = prev_allowed;
                }

                Stmt::FnDecl {
                    is_public: _,
                    name,
                    params,
                    return_ty,
                    body,
                } => {
                    if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                        eprintln!(
                            "DEBUG compile_statements: FnDecl '{}' with {} body statements",
                            name,
                            body.len()
                        );
                    }
                    let previous_block = self.builder.get_insert_block();
                    let fn_is_variadic = params.iter().any(|p| p.is_variadic);

                    // シグネチャ情報を保存（呼び出し側で variadic を pack するため）
                    let llvm_name = name.replace('.', "_");
                    self.fn_params.insert(llvm_name.clone(), params.clone());

                    let void_type = self.context.void_type();
                    let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                    let i32_t = self.context.i32_type();
                    let i1_t = self.context.bool_type();
                    let i64_t = self.context.i64_type();
                    let ret_kind = Self::parse_return_kind(return_ty.as_deref());

                    let mut llvm_param_types = Vec::new();
                    let mut param_kinds = Vec::new();
                    for p in params {
                        if p.is_variadic {
                            // variadic param 自体は (ptr data, i32 len) に変換するため、ここでは追加しない
                            continue;
                        }
                        let ty = p.ty.as_str();
                        let (llvm_ty, kind) = match ty {
                            "Ptr" | "Any" | "String" => (ptr_t.into(), VariableKind::Ptr),
                            "Int" | "i32" => (i32_t.into(), VariableKind::I32),
                            "Bool" | "bool" => (self.context.bool_type().into(), VariableKind::Bool),
                            _ => (i32_t.into(), VariableKind::I32),
                        };
                        llvm_param_types.push(llvm_ty);
                        param_kinds.push(kind);
                    }

                    // 同じ std モジュールを複数回 import した場合など、同名関数を再定義しない。
                    if let Some(existing) = self.module.get_function(&llvm_name) {
                        if existing.count_basic_blocks() > 0 {
                            debug!(
                                "Codegen: skipping duplicate function definition '{}'",
                                llvm_name
                            );
                            if previous_block.is_none() {
                                self.builder.clear_insertion_position();
                            }
                            continue;
                        }
                    }

                    // Kome の variadic は (ptr data, i32 len) として受け取る
                    if fn_is_variadic {
                        // variadic param は最後の1つだけ許可
                        let last_is_variadic = params.last().is_some_and(|p| p.is_variadic);
                        let variadic_count = params.iter().filter(|p| p.is_variadic).count();
                        if variadic_count != 1 || !last_is_variadic {
                            panic!("可変長引数は最後の1つだけ指定できます。");
                        }
                        // 追加で (ptr, i32) を受け取る
                        llvm_param_types.push(ptr_t.into());
                        llvm_param_types.push(i32_t.into());
                    }

                    let fn_type = match ret_kind {
                        ReturnKind::Void => void_type.fn_type(&llvm_param_types, false),
                        ReturnKind::I32 => i32_t.fn_type(&llvm_param_types, false),
                        ReturnKind::Ptr => ptr_t.fn_type(&llvm_param_types, false),
                        ReturnKind::Bool => i1_t.fn_type(&llvm_param_types, false),
                        ReturnKind::OptI32 => i64_t.fn_type(&llvm_param_types, false),
                        ReturnKind::OptPtr => ptr_t.fn_type(&llvm_param_types, false),
                    };
                    let function = self.module.add_function(&llvm_name, fn_type, None);
                    let entry_block = self.context.append_basic_block(function, "entry");

                    self.variables.retain(|_, v| v.is_state);
                    self.builder.position_at_end(entry_block);
                    let prev_ret = self.current_return;
                    self.current_return = Some(ret_kind);

                    let mut arg_index: u32 = 0;
                    let mut kind_index: usize = 0;
                    for p in params.iter() {
                        if p.is_variadic {
                            // 可変長引数: (ptr data, i32 len)
                            let data_idx = arg_index;
                            let len_idx = arg_index + 1;
                            let data_alloca = self
                                .builder
                                .build_alloca(ptr_t, &format!("{}_data", p.name))
                                .expect("alloca varargs data");
                            let len_alloca = self
                                .builder
                                .build_alloca(i32_t, &format!("{}_len", p.name))
                                .expect("alloca varargs len");
                            let data_arg = function.get_nth_param(data_idx).expect("param");
                            let len_arg = function.get_nth_param(len_idx).expect("param");
                            self.builder
                                .build_store(data_alloca, data_arg)
                                .expect("store varargs data");
                            self.builder
                                .build_store(len_alloca, len_arg)
                                .expect("store varargs len");
                            self.variables.insert(
                                format!("{}_data", p.name),
                                VariableInfo {
                                    ptr: data_alloca,
                                    is_state: false,
                                    kind: VariableKind::Ptr,
                                },
                            );
                            self.variables.insert(
                                format!("{}_len", p.name),
                                VariableInfo {
                                    ptr: len_alloca,
                                    is_state: false,
                                    kind: VariableKind::I32,
                                },
                            );
                            arg_index += 2;
                            continue;
                        }

                        let kind = param_kinds
                            .get(kind_index)
                            .copied()
                            .unwrap_or(VariableKind::I32);
                        let alloca = match kind {
                            VariableKind::I32 => self
                                .builder
                                .build_alloca(i32_t, &p.name)
                                .expect("alloca i32"),
                            VariableKind::Ptr => self
                                .builder
                                .build_alloca(ptr_t, &p.name)
                                .expect("alloca ptr"),
                            VariableKind::Bool => self
                                .builder
                                .build_alloca(self.context.bool_type(), &p.name)
                                .expect("alloca bool"),
                        };
                        let arg = function.get_nth_param(arg_index).expect("param");
                        self.builder.build_store(alloca, arg).expect("store param");
                        self.variables.insert(
                            p.name.clone(),
                            VariableInfo {
                                ptr: alloca,
                                is_state: false,
                                kind,
                            },
                        );
                        arg_index += 1;
                        kind_index += 1;
                    }

                    // 戻り値がある場合、末尾の式は暗黙 return 側で評価する（短絡/SSA崩れ防止）
                    let (body_prefix, body_last) = if ret_kind != ReturnKind::Void {
                        match body.split_last() {
                            Some((last, prefix)) => (prefix, Some(last)),
                            None => (body.as_slice(), None),
                        }
                    } else {
                        (body.as_slice(), None)
                    };

                    // `!default` は「この関数の末尾で条件適用」なので、関数ボディ単位で管理する
                    self.push_default_scope();
                    self.compile_statements(body_prefix)?;
                    // 暗黙 return の評価前に default を確定させる（最後の式が state を読むため）
                    self.pop_default_scope_apply()?;

                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        match ret_kind {
                            ReturnKind::Void => {
                                self.builder
                                    .build_return(None)
                                    .expect("Failed to build void return");
                            }
                            ReturnKind::I32 | ReturnKind::Ptr => {
                                // 仕様: `return` は任意で、最後の式が戻り値になる
                                // 最後の文が ExprStmt のときだけ暗黙 return する
                                match body_last {
                                    Some(Stmt::ExprStmt(e)) => {
                                        let v = self.compile_expr(e);
                                        self.builder.build_return(Some(&v)).expect("return");
                                    }
                                    Some(Stmt::If { condition, then_body, else_body }) => {
                                        let v = self.compile_if_stmt_expr(condition, then_body, else_body);
                                        self.builder.build_return(Some(&v)).expect("return");
                                    }
                                    _ => {
                                        panic!(
                                            "戻り値のある関数 '{}' は最後の式か return が必要です。",
                                            name
                                        );
                                    }
                                }
                            }
                            ReturnKind::Bool => {
                                match body_last {
                                    Some(Stmt::ExprStmt(e)) => {
                                        let tmp = self.compile_expr(e);
                                        let v = self.to_bool(tmp);
                                        self.builder
                                            .build_return(Some(&v))
                                            .expect("Failed to build implicit return");
                                    }
                                    _ => {
                                        panic!(
                                            "戻り値のある関数 '{}' は最後の式か return が必要です。",
                                            name
                                        );
                                    }
                                }
                            }
                            ReturnKind::OptI32 | ReturnKind::OptPtr => {
                                match body_last {
                                    Some(Stmt::ExprStmt(e)) => {
                                        let v = self.compile_expr(e);
                                        let out = match ret_kind {
                                            ReturnKind::OptI32 => self.encode_opt_i32(v),
                                            ReturnKind::OptPtr => self.encode_opt_ptr(v),
                                            _ => v,
                                        };
                                        self.builder
                                            .build_return(Some(&out))
                                            .expect("Failed to build implicit return");
                                    }
                                    _ => {
                                        panic!(
                                            "戻り値のある関数 '{}' は最後の式か return が必要です。",
                                            name
                                        );
                                    }
                                }
                            }
                        }
                    }

                    self.current_return = prev_ret;
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
                    let kind = self.current_return.unwrap_or(ReturnKind::Void);
                    match (kind, ret_expr) {
                        (ReturnKind::Void, None) => {
                            self.builder.build_return(None).ok();
                        }
                        (ReturnKind::Void, Some(_)) => {
                            panic!("Void 関数で値を return しています。");
                        }
                        (ReturnKind::I32, Some(e)) | (ReturnKind::Ptr, Some(e)) => {
                            let v = self.compile_expr(e);
                            self.builder.build_return(Some(&v)).ok();
                        }
                        (ReturnKind::Bool, Some(e)) => {
                            let tmp = self.compile_expr(e);
                            let v = self.to_bool(tmp);
                            self.builder.build_return(Some(&v)).ok();
                        }
                        (ReturnKind::I32, None) | (ReturnKind::Ptr, None) => {
                            panic!("戻り値のある関数で return の値がありません。");
                        }
                        (ReturnKind::Bool, None) => {
                            panic!("戻り値のある関数で return の値がありません。");
                        }
                        (ReturnKind::OptI32, Some(e)) | (ReturnKind::OptPtr, Some(e)) => {
                            let v = self.compile_expr(e);
                            let out = match kind {
                                ReturnKind::OptI32 => self.encode_opt_i32(v),
                                ReturnKind::OptPtr => self.encode_opt_ptr(v),
                                _ => v,
                            };
                            self.builder.build_return(Some(&out)).ok();
                        }
                        (ReturnKind::OptI32, None) | (ReturnKind::OptPtr, None) => {
                            panic!("戻り値のある関数で return の値がありません。");
                        }
                    }
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
                Stmt::Is { value, pat, body } => {
                    // is は「一致したら body を実行する」だけの簡易分岐
                    let parent_func = self
                        .builder
                        .get_insert_block()
                        .expect("is requires insert block")
                        .get_parent()
                        .expect("is requires parent func");

                    let then_bb = self.context.append_basic_block(parent_func, "is_then");
                    let merge_bb = self.context.append_basic_block(parent_func, "is_merge");

                    let v = self.compile_expr(value);
                    let cond = self.build_match_pat_cond(v, pat);
                    self.builder
                        .build_conditional_branch(cond, then_bb, merge_bb)
                        .expect("is branch");

                    self.builder.position_at_end(then_bb);
                    let tmp = body.as_ref().clone();
                    self.compile_statements(std::slice::from_ref(&tmp))?;
                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
                        self.builder
                            .build_unconditional_branch(merge_bb)
                            .expect("is br merge");
                    }

                    self.builder.position_at_end(merge_bb);
                }
                Stmt::Match { value, arms } => {
                    // match は上から順に評価して最初に一致した arm を実行する
                    let parent_func = self
                        .builder
                        .get_insert_block()
                        .expect("match requires insert block")
                        .get_parent()
                        .expect("match requires parent func");

                    let merge_bb = self.context.append_basic_block(parent_func, "match_merge");
                    let mut next_bb = self
                        .builder
                        .get_insert_block()
                        .expect("match insert block");

                    let v = self.compile_expr(value);

                    for (i, (pat, body)) in arms.iter().enumerate() {
                        let arm_bb =
                            self.context.append_basic_block(parent_func, &format!("match_arm_{i}"));
                        let cont_bb =
                            self.context.append_basic_block(parent_func, &format!("match_cont_{i}"));

                        self.builder.position_at_end(next_bb);
                        let cond = self.build_match_pat_cond(v, pat);
                        self.builder
                            .build_conditional_branch(cond, arm_bb, cont_bb)
                            .expect("match branch");

                        self.builder.position_at_end(arm_bb);
                        let tmp = body.as_ref().clone();
                        self.compile_statements(std::slice::from_ref(&tmp))?;
                        if self
                            .builder
                            .get_insert_block()
                            .and_then(|bb| bb.get_terminator())
                            .is_none()
                        {
                            self.builder
                                .build_unconditional_branch(merge_bb)
                                .expect("arm br merge");
                        }

                        next_bb = cont_bb;
                    }

                    // どの arm にも一致しなかった場合
                    self.builder.position_at_end(next_bb);
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .expect("match default br");

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

                    // entry ブロックは最初の分岐で終端していることがあるので、
                    // 「現在の挿入ブロック」が終端しているかで判定する。
                    if self
                        .builder
                        .get_insert_block()
                        .and_then(|bb| bb.get_terminator())
                        .is_none()
                    {
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
                    is_default,
                    name,
                    value,
                } => {
                    let canonical = self.canonical_var_name(name);

                    if *is_default {
                        // 先に必要な情報を確定（`last_mut()` と self メソッド呼び出しの同時借用を避ける）
                        let (_, _, kind, _) = self.resolve_assignment_target(&canonical);
                        let new_slot_ptr = match kind {
                            VariableKind::I32 => self
                                .build_alloca_in_entry(self.context.i32_type(), "default_slot_i32"),
                            VariableKind::Bool => self
                                .build_alloca_in_entry(self.context.bool_type(), "default_slot_bool"),
                            VariableKind::Ptr => self.build_alloca_in_entry(
                                self.context.ptr_type(AddressSpace::from(0)),
                                "default_slot_ptr",
                            ),
                        };
                        let new_active_ptr =
                            self.build_bool_alloca_in_entry("default_active", false);
                        let new_assigned_ptr =
                            self.build_bool_alloca_in_entry("default_assigned", false);

                        let Some(scope) = self.default_scopes.last_mut() else {
                            panic!("!default は関数/クロージャ内で使用してください。");
                        };

                        // 変数型に合わせた slot / flag を用意
                        let slot_ptr = if let Some(p) = scope.default_slots.get(&canonical).copied() {
                            p
                        } else {
                            scope.default_slots.insert(canonical.clone(), new_slot_ptr);
                            new_slot_ptr
                        };

                        let active_ptr = if let Some(p) = scope.active_flags.get(&canonical).copied() {
                            p
                        } else {
                            scope.active_flags.insert(canonical.clone(), new_active_ptr);
                            new_active_ptr
                        };

                        if !scope.assigned_flags.contains_key(&canonical) {
                            scope.assigned_flags.insert(canonical.clone(), new_assigned_ptr);
                        }

                        // `!default` 実行時に評価して slot に保存（分岐内でも正しく動く）
                        let rhs = self.compile_expr(value);
                        let rhs = self.normalize_value_for_kind(kind, rhs);
                        match kind {
                            VariableKind::I32 | VariableKind::Bool => {
                                self.builder.build_store(slot_ptr, rhs.into_int_value()).expect("store default slot");
                            }
                            VariableKind::Ptr => {
                                self.builder.build_store(slot_ptr, rhs.into_pointer_value()).expect("store default slot");
                            }
                        }
                        self.builder
                            .build_store(active_ptr, self.context.bool_type().const_int(1, false))
                            .expect("activate default");
                    } else {
                        let rhs = self.compile_expr(value);
                        self.codegen_assignment_store_value(&canonical, rhs, true)?;
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
            Expr::Bool(v) => {
                let i1_t = self.context.bool_type();
                i1_t.const_int(if *v { 1 } else { 0 }, false)
                    .as_basic_value_enum()
            }
            Expr::None => {
                // none は null 相当（今は Ptr 専用の ?? のため）
                let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                ptr_t.const_null().as_basic_value_enum()
            }
            Expr::Ident(name) => {
                // Special case: "any" / "_" はイベントハンドラ用のプレースホルダ
                if name == "any" || name == "_" {
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
                        VariableKind::Bool => self
                            .builder
                            .build_load(self.context.bool_type(), ptr, name)
                            .expect("Failed to load bool variable"),
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

                match op {
                    Op::Add => self
                        .builder
                        .build_int_add(
                            left_val.into_int_value(),
                            self.compile_expr(right).into_int_value(),
                            "addtmp",
                        )
                        .expect("Failed to build add instruction")
                        .as_basic_value_enum(),
                    Op::Sub => self
                        .builder
                        .build_int_sub(
                            left_val.into_int_value(),
                            self.compile_expr(right).into_int_value(),
                            "subtmp",
                        )
                        .expect("Failed to build sub instruction")
                        .as_basic_value_enum(),
                    Op::Mul => self
                        .builder
                        .build_int_mul(
                            left_val.into_int_value(),
                            self.compile_expr(right).into_int_value(),
                            "multmp",
                        )
                        .expect("Failed to build mul instruction")
                        .as_basic_value_enum(),
                    Op::Div => self
                        .builder
                        .build_int_signed_div(
                            left_val.into_int_value(),
                            self.compile_expr(right).into_int_value(),
                            "divtmp",
                        )
                        .expect("Failed to build div instruction")
                        .as_basic_value_enum(),
                    Op::Eq => {
                        // ==
                        let right_val = self.compile_expr(right);
                        if left_val.is_pointer_value() || right_val.is_pointer_value() {
                            if !left_val.is_pointer_value() || !right_val.is_pointer_value() {
                                panic!("ptr と int/bool は比較できません。");
                            }
                            let lp = left_val.into_pointer_value();
                            let rp = right_val.into_pointer_value();
                            // null 比較は専用命令を使う
                            if rp.is_null() {
                                self.builder
                                    .build_is_null(lp, "peq_null")
                                    .expect("isnull")
                                    .as_basic_value_enum()
                            } else if lp.is_null() {
                                self.builder
                                    .build_is_null(rp, "peq_null")
                                    .expect("isnull")
                                    .as_basic_value_enum()
                            } else {
                                let i64_t = self.context.i64_type();
                                let li = self
                                    .builder
                                    .build_ptr_to_int(lp, i64_t, "peq_li")
                                    .expect("ptrtoint");
                                let ri = self
                                    .builder
                                    .build_ptr_to_int(rp, i64_t, "peq_ri")
                                    .expect("ptrtoint");
                                self.builder
                                    .build_int_compare(IntPredicate::EQ, li, ri, "peq")
                                    .expect("icmp")
                                    .as_basic_value_enum()
                            }
                        } else {
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
                    }
                    Op::Neq => {
                        // !=
                        let right_val = self.compile_expr(right);
                        if left_val.is_pointer_value() || right_val.is_pointer_value() {
                            if !left_val.is_pointer_value() || !right_val.is_pointer_value() {
                                panic!("ptr と int/bool は比較できません。");
                            }
                            let lp = left_val.into_pointer_value();
                            let rp = right_val.into_pointer_value();
                            if rp.is_null() {
                                self.builder
                                    .build_is_not_null(lp, "pne_null")
                                    .expect("isnotnull")
                                    .as_basic_value_enum()
                            } else if lp.is_null() {
                                self.builder
                                    .build_is_not_null(rp, "pne_null")
                                    .expect("isnotnull")
                                    .as_basic_value_enum()
                            } else {
                                let i64_t = self.context.i64_type();
                                let li = self
                                    .builder
                                    .build_ptr_to_int(lp, i64_t, "pne_li")
                                    .expect("ptrtoint");
                                let ri = self
                                    .builder
                                    .build_ptr_to_int(rp, i64_t, "pne_ri")
                                    .expect("ptrtoint");
                                self.builder
                                    .build_int_compare(IntPredicate::NE, li, ri, "pne")
                                    .expect("icmp")
                                    .as_basic_value_enum()
                            }
                        } else {
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
                    }
                    Op::Lt => {
                        // <
                        let right_val = self.compile_expr(right);
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
                        let right_val = self.compile_expr(right);
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
                        let right_val = self.compile_expr(right);
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
                        let right_val = self.compile_expr(right);
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
                        // &&（短絡評価）
                        let left_bb = self
                            .builder
                            .get_insert_block()
                            .expect("and requires insert block");
                        let left_i1 = self.to_bool(left_val);

                        let parent_func = self
                            .builder
                            .get_insert_block()
                            .expect("and requires insert block")
                            .get_parent()
                            .expect("and requires parent func");

                        let rhs_bb = self.context.append_basic_block(parent_func, "and_rhs");
                        let merge_bb = self.context.append_basic_block(parent_func, "and_merge");

                        // left が true のときだけ rhs を評価する
                        self.builder
                            .build_conditional_branch(left_i1, rhs_bb, merge_bb)
                            .expect("and branch");

                        // rhs
                        self.builder.position_at_end(rhs_bb);
                        let rhs_tmp = self.compile_expr(right);
                        let rhs_v = self.to_bool(rhs_tmp);
                        let rhs_end = self.builder.get_insert_block().expect("rhs end");
                        if rhs_end.get_terminator().is_none() {
                            self.builder
                                .build_unconditional_branch(merge_bb)
                                .expect("rhs br merge");
                        }

                        // merge
                        self.builder.position_at_end(merge_bb);
                        let phi = self
                            .builder
                            .build_phi(left_i1.get_type(), "andtmp")
                            .expect("phi and");
                        let false_v = left_i1.get_type().const_int(0, false);
                        phi.add_incoming(&[(&false_v, left_bb), (&rhs_v, rhs_end)]);
                        phi.as_basic_value()
                    }
                    Op::Or => {
                        // ||（短絡評価）
                        let left_bb = self
                            .builder
                            .get_insert_block()
                            .expect("or requires insert block");
                        let left_i1 = self.to_bool(left_val);

                        let parent_func = self
                            .builder
                            .get_insert_block()
                            .expect("or requires insert block")
                            .get_parent()
                            .expect("or requires parent func");

                        let rhs_bb = self.context.append_basic_block(parent_func, "or_rhs");
                        let merge_bb = self.context.append_basic_block(parent_func, "or_merge");

                        // left が false のときだけ rhs を評価する
                        self.builder
                            .build_conditional_branch(left_i1, merge_bb, rhs_bb)
                            .expect("or branch");

                        // rhs
                        self.builder.position_at_end(rhs_bb);
                        let rhs_tmp = self.compile_expr(right);
                        let rhs_v = self.to_bool(rhs_tmp);
                        let rhs_end = self.builder.get_insert_block().expect("rhs end");
                        if rhs_end.get_terminator().is_none() {
                            self.builder
                                .build_unconditional_branch(merge_bb)
                                .expect("rhs br merge");
                        }

                        // merge
                        self.builder.position_at_end(merge_bb);
                        let phi = self
                            .builder
                            .build_phi(left_i1.get_type(), "ortmp")
                            .expect("phi or");
                        let true_v = left_i1.get_type().const_int(1, false);
                        phi.add_incoming(&[(&true_v, left_bb), (&rhs_v, rhs_end)]);
                        phi.as_basic_value()
                    }
                    Op::In => {
                        // TODO: 実装
                        todo!("Codegen: 'in' operator is not yet implemented.")
                    }
                    Op::Question => {
                        // ??（null/none 合体）
                        // - ptr? は null を none として扱う
                        // - int? は i64 の 0=none, (x+1)=some(x) として扱う

                        // `none ?? rhs` は rhs を返す（左は評価不要）
                        if matches!(&**left, Expr::None) {
                            return self.compile_expr(right);
                        }

                        let left_bb = self
                            .builder
                            .get_insert_block()
                            .expect("?? requires insert block");
                        let parent_func = left_bb.get_parent().expect("parent func");
                        let rhs_bb = self.context.append_basic_block(parent_func, "coalesce_rhs");
                        let merge_bb = self.context.append_basic_block(parent_func, "coalesce_merge");

                        if left_val.is_pointer_value() {
                            // ptr?
                            let left_p = left_val.into_pointer_value();
                            let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                            let is_nonnull = self
                                .builder
                                .build_is_not_null(left_p, "nonnull")
                                .expect("isnotnull");

                            self.builder
                                .build_conditional_branch(is_nonnull, merge_bb, rhs_bb)
                                .expect("coalesce branch");

                            // rhs
                            self.builder.position_at_end(rhs_bb);
                            let rhs_v = self.compile_expr(right);
                            if !rhs_v.is_pointer_value() {
                                panic!("?? の右辺はポインタ型である必要があります。");
                            }
                            let rhs_p = rhs_v.into_pointer_value();
                            let rhs_end = self.builder.get_insert_block().expect("rhs end");
                            if rhs_end.get_terminator().is_none() {
                                self.builder
                                    .build_unconditional_branch(merge_bb)
                                    .expect("rhs br merge");
                            }

                            // merge
                            self.builder.position_at_end(merge_bb);
                            if let Some(first) = merge_bb.get_first_instruction() {
                                self.builder.position_before(&first);
                            }
                            let phi = self
                                .builder
                                .build_phi(ptr_t, "coalesce")
                                .expect("phi ??");
                            phi.add_incoming(&[(&left_p, left_bb), (&rhs_p, rhs_end)]);
                            phi.as_basic_value()
                        } else if left_val.is_int_value()
                            && left_val.into_int_value().get_type().get_bit_width() == 64
                        {
                            // int?（i64 エンコード）
                            let left_i64 = left_val.into_int_value();
                            let i64_t = self.context.i64_type();
                            let is_some = self
                                .builder
                                .build_int_compare(
                                    IntPredicate::NE,
                                    left_i64,
                                    i64_t.const_int(0, false),
                                    "hassome",
                                )
                                .expect("icmp hassome");

                            let some_bb =
                                self.context.append_basic_block(parent_func, "coalesce_some");

                            self.builder
                                .build_conditional_branch(is_some, some_bb, rhs_bb)
                                .expect("coalesce branch");

                            // some
                            self.builder.position_at_end(some_bb);
                            let decoded = self
                                .builder
                                .build_int_sub(left_i64, i64_t.const_int(1, false), "dec")
                                .expect("decode");
                            let decoded_i32 = self
                                .builder
                                .build_int_truncate(decoded, self.context.i32_type(), "dec32")
                                .expect("trunc");
                            let some_end = self.builder.get_insert_block().expect("some end");
                            if some_end.get_terminator().is_none() {
                                self.builder
                                    .build_unconditional_branch(merge_bb)
                                    .expect("some br merge");
                            }

                            // rhs
                            self.builder.position_at_end(rhs_bb);
                            let rhs_v = self.compile_expr(right).into_int_value();
                            let rhs_end = self.builder.get_insert_block().expect("rhs end");
                            if rhs_end.get_terminator().is_none() {
                                self.builder
                                    .build_unconditional_branch(merge_bb)
                                    .expect("rhs br merge");
                            }

                            // merge
                            self.builder.position_at_end(merge_bb);
                            if let Some(first) = merge_bb.get_first_instruction() {
                                self.builder.position_before(&first);
                            }
                            let phi = self
                                .builder
                                .build_phi(self.context.i32_type(), "coalesce_i32")
                                .expect("phi ?? i32");
                            phi.add_incoming(&[(&decoded_i32, some_end), (&rhs_v, rhs_end)]);
                            phi.as_basic_value()
                        } else {
                            panic!("?? は ptr? か int? にだけ使えます。");
                        }
                    }
                    Op::With => {
                        // 文字列結合
                        let right_val = self.compile_expr(right);
                        if left_val.is_pointer_value() && right_val.is_pointer_value() {
                            let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                            let fn_val = match self.module.get_function("__kome_str_concat") {
                                Some(f) => f,
                                None => {
                                    let fn_ty = ptr_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                                    self.module.add_function("__kome_str_concat", fn_ty, None)
                                }
                            };
                            let call = self
                                .builder
                                .build_call(
                                    fn_val,
                                    &[
                                        left_val.into_pointer_value().into(),
                                        right_val.into_pointer_value().into(),
                                    ],
                                    "strconcat",
                                )
                                .expect("call __kome_str_concat");
                            match call.try_as_basic_value() {
                                ValueKind::Basic(v) => v,
                                ValueKind::Instruction(_) => panic!("strconcat should return value"),
                            }
                        } else {
                            panic!("with は文字列同士でのみ使えます。");
                        }
                    }
                }
            }
            Expr::IfExpr {
                condition,
                then_body,
                else_body,
            } => {
                // if を式として扱う（両分岐の値を phi で合流）
                let cond_tmp = self.compile_expr(condition);
                let cond_i1 = self.to_bool(cond_tmp);

                let parent_func = self
                    .builder
                    .get_insert_block()
                    .expect("if expr requires insert block")
                    .get_parent()
                    .expect("if expr requires parent func");

                let then_bb = self.context.append_basic_block(parent_func, "if_then");
                let else_bb = self.context.append_basic_block(parent_func, "if_else");
                let merge_bb = self.context.append_basic_block(parent_func, "if_merge");

                self.builder
                    .build_conditional_branch(cond_i1, then_bb, else_bb)
                    .expect("build if branch");

                // then
                self.builder.position_at_end(then_bb);
                let then_val = self.compile_block_expr(then_body);
                let then_end = self.builder.get_insert_block().expect("then end block");
                if then_end.get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .expect("then br merge");
                }

                // else
                self.builder.position_at_end(else_bb);
                let else_val = self.compile_block_expr(else_body);
                let else_end = self.builder.get_insert_block().expect("else end block");
                if else_end.get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .expect("else br merge");
                }

                // merge
                self.builder.position_at_end(merge_bb);
                match (then_val, else_val) {
                    (
                        inkwell::values::BasicValueEnum::IntValue(ti),
                        inkwell::values::BasicValueEnum::IntValue(ei),
                    ) => {
                        let phi = self
                            .builder
                            .build_phi(ti.get_type(), "iftmp")
                            .expect("phi");
                        phi.add_incoming(&[(&ti, then_end), (&ei, else_end)]);
                        phi.as_basic_value()
                    }
                    (
                        inkwell::values::BasicValueEnum::PointerValue(tp),
                        inkwell::values::BasicValueEnum::PointerValue(ep),
                    ) => {
                        let phi = self
                            .builder
                            .build_phi(tp.get_type(), "iftmp")
                            .expect("phi");
                        phi.add_incoming(&[(&tp, then_end), (&ep, else_end)]);
                        phi.as_basic_value()
                    }
                    (
                        inkwell::values::BasicValueEnum::PointerValue(_),
                        inkwell::values::BasicValueEnum::IntValue(ei),
                    ) => {
                        // none / Int の組み合わせは Int? とみなす
                        let i64_t = self.context.i64_type();
                        // else 側で i64 を作ってから phi で合流する（merge で ei を直接使うと SSA が壊れる）
                        if else_end.get_terminator().is_some() {
                            // すでに merge へ br 済みなので、その直前に挿入する
                            let term = else_end.get_terminator().unwrap();
                            self.builder.position_before(&term);
                        } else {
                            self.builder.position_at_end(else_end);
                        }
                        let encoded_else = self
                            .builder
                            .build_int_s_extend(ei, i64_t, "opt_ext")
                            .expect("sext");
                        let encoded_else = self
                            .builder
                            .build_int_add(encoded_else, i64_t.const_int(1, false), "opt_enc")
                            .expect("enc");
                        let none_v = i64_t.const_int(0, false);

                        self.builder.position_at_end(merge_bb);
                        let phi = self
                            .builder
                            .build_phi(i64_t, "iftmp_opt")
                            .expect("phi");
                        phi.add_incoming(&[(&none_v, then_end), (&encoded_else, else_end)]);
                        phi.as_basic_value()
                    }
                    (
                        inkwell::values::BasicValueEnum::IntValue(ti),
                        inkwell::values::BasicValueEnum::PointerValue(_),
                    ) => {
                        // Int / none の組み合わせは Int? とみなす
                        let i64_t = self.context.i64_type();
                        if then_end.get_terminator().is_some() {
                            let term = then_end.get_terminator().unwrap();
                            self.builder.position_before(&term);
                        } else {
                            self.builder.position_at_end(then_end);
                        }
                        let encoded_then = self
                            .builder
                            .build_int_s_extend(ti, i64_t, "opt_ext")
                            .expect("sext");
                        let encoded_then = self
                            .builder
                            .build_int_add(encoded_then, i64_t.const_int(1, false), "opt_enc")
                            .expect("enc");
                        let none_v = i64_t.const_int(0, false);

                        self.builder.position_at_end(merge_bb);
                        let phi = self
                            .builder
                            .build_phi(i64_t, "iftmp_opt")
                            .expect("phi");
                        phi.add_incoming(&[(&encoded_then, then_end), (&none_v, else_end)]);
                        phi.as_basic_value()
                    }
                    _ => panic!("if 式の then/else の型が一致していません。"),
                }
            }
            Expr::IsExpr { value, pat, then_expr } => {
                // `is` は「一致したら then を返し、そうでなければ none(ptr null)」として扱う
                let parent = self
                    .builder
                    .get_insert_block()
                    .expect("is expr requires insert block")
                    .get_parent()
                    .expect("is expr requires parent func");

                let then_bb = self.context.append_basic_block(parent, "is_expr_then");
                let else_bb = self.context.append_basic_block(parent, "is_expr_else");
                let merge_bb = self.context.append_basic_block(parent, "is_expr_merge");

                let v = self.compile_expr(value);
                let cond = self.build_match_pat_cond(v, pat);
                self.builder
                    .build_conditional_branch(cond, then_bb, else_bb)
                    .expect("is expr br");

                self.builder.position_at_end(then_bb);
                let then_v = self.compile_expr(then_expr);
                let then_end = self
                    .builder
                    .get_insert_block()
                    .expect("then bb");
                if then_end.get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .expect("is expr br merge");
                }

                self.builder.position_at_end(else_bb);
                let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                let else_v = ptr_t.const_null().as_basic_value_enum();
                let else_end = self
                    .builder
                    .get_insert_block()
                    .expect("else bb");
                if else_end.get_terminator().is_none() {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .expect("is expr br merge");
                }

                self.builder.position_at_end(merge_bb);
                let phi = self
                    .builder
                    .build_phi(ptr_t, "is_expr_phi")
                    .expect("phi is expr");
                phi.add_incoming(&[(&then_v, then_end), (&else_v, else_end)]);
                phi.as_basic_value()
            }
            Expr::Block(stmts) => self.compile_block_expr(stmts),
            Expr::CallChain { head, tails } => {
                /* Handle bundle.method() calls (e.g., App.run()) */
                if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                    eprintln!(
                        "DEBUG: compile_expr CallChain head={}, tails.len={}",
                        head,
                        tails.len()
                    );
                    for (i, tail) in tails.iter().enumerate() {
                        eprintln!("DEBUG:   tail[{}] = {:?}", i, tail);
                    }
                }

                // ここでは `print` や `io.print` のような特別扱い（ハードコード）はしない。
                // 可変長引数の転送（`...`）は、通常の関数呼び出しとして処理できるように
                // `printf(fmt, ...)` を `vprintf(fmt, va_list)` へ lower する形で実装する。

                // a.b() / a.b.c() / a.b.c.d() などは「末尾の Method」を関数呼び出しに解決する
                // 例: `viewKit.window.create(x)` -> `viewKit_window_create(x)`
                if tails.len() >= 2 {
                    if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                        eprintln!("DEBUG: Found CallChain with {} tails", tails.len());
                    }
                    if let (
                        ast::Accessor::Method(args, trailing_closure),
                        ast::Accessor::Property(method_name),
                    ) = (&tails[tails.len() - 1], &tails[tails.len() - 2])
                    {
                        if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                            eprintln!(
                                "DEBUG: Property + Method found: {}.<...>.{}()",
                                head, method_name
                            );
                        }

                        // head + 途中の Property を '_' で連結して関数名にする
                        // - 途中に Method がある場合はフォールバックへ回す
                        let mut fn_name = head.clone();
                        let mut ok = true;
                        for prop in tails.iter().take(tails.len() - 1) {
                            let ast::Accessor::Property(p) = prop else {
                                ok = false;
                                break;
                            };
                            fn_name.push('_');
                            fn_name.push_str(p);
                        }
                        if !ok {
                            // フォールバック: 旧仕様（bundle_name + method_name）
                            fn_name = format!("{}_{}", head, method_name);
                        }

                        if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                            eprintln!("DEBUG: Looking for function: {}", fn_name);
                        }
                        if let Some(function) = self.module.get_function(&fn_name) {
                            if std::env::var("KOME_DEBUG_CODEGEN").ok().as_deref() == Some("1") {
                                eprintln!("DEBUG: Found function: {}", fn_name);
                            }
                            if function.count_basic_blocks() == 0 {
                                if self.current_module_prefix.is_some()
                                    && !self.allowed_externs.contains(&fn_name)
                                {
                                    panic!(
                                        "モジュール内で C 関数 '{}' を呼び出そうとしましたが、このファイル内で `cinclude`（または `use libc.*`）が宣言されていません。",
                                        fn_name
                                    );
                                }
                            }

                            let mut llvm_args = Vec::new();
                            // Kome 関数の可変長引数（型付き）は、呼び出し側で (ptr data, len) に pack する
                            if let Some(sig) = self.fn_params.get(&fn_name).cloned() {
                                let variadic = sig.iter().position(|p| p.is_variadic);
                                if let Some(var_pos) = variadic {
                                    if var_pos + 1 != sig.len() {
                                        panic!("可変長引数は最後の1つだけ指定できます。");
                                    }
                                    let fixed = var_pos;
                                    if args.len() < fixed {
                                        panic!("引数の数が足りません: {}", fn_name);
                                    }
                                    // 固定引数
                                    for a in args.iter().take(fixed) {
                                        let v = self.compile_expr(a);
                                        llvm_args.push(v.into());
                                    }
                                    // 可変長部分を i32/ptr 配列としてスタックに確保
                                    let var_param = &sig[var_pos];
                                    let elem_kind = var_param.ty.as_str();
                                    let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                                    let (elem_ty, elem_kind_enum) = match elem_kind {
                                        "Ptr" | "Any" | "String" => {
                                            (ptr_t.as_basic_type_enum(), VariableKind::Ptr)
                                        }
                                        "Int" | "i32" => (
                                            self.context.i32_type().as_basic_type_enum(),
                                            VariableKind::I32,
                                        ),
                                        _ => (
                                            self.context.i32_type().as_basic_type_enum(),
                                            VariableKind::I32,
                                        ),
                                    };
                                    let var_vals = args.iter().skip(fixed).collect::<Vec<_>>();
                                    let len = var_vals.len() as u64;
                                    let arr_ty = elem_ty.array_type(var_vals.len() as u32);
                                    let arr = self
                                        .builder
                                        .build_alloca(arr_ty, "varargs_arr")
                                        .expect("alloca varargs arr");
                                    for (i, a) in var_vals.into_iter().enumerate() {
                                        let gep = unsafe {
                                            self.builder.build_in_bounds_gep(
                                                arr_ty,
                                                arr,
                                                &[
                                                    self.context.i32_type().const_int(0, false),
                                                    self.context
                                                        .i32_type()
                                                        .const_int(i as u64, false),
                                                ],
                                                "vararg_gep",
                                            )
                                        }
                                        .expect("gep vararg");
                                        let val = self.compile_expr(a);
                                        match elem_kind_enum {
                                            VariableKind::I32 => {
                                                self.builder
                                                    .build_store(gep, val.into_int_value())
                                                    .ok();
                                            }
                                            VariableKind::Ptr => {
                                                self.builder
                                                    .build_store(gep, val.into_pointer_value())
                                                    .ok();
                                            }
                                            VariableKind::Bool => {
                                                unreachable!("variadic bool is not supported")
                                            }
                                        }
                                    }
                                    // 先頭要素ポインタ
                                    let data_ptr = unsafe {
                                        self.builder.build_in_bounds_gep(
                                            arr_ty,
                                            arr,
                                            &[
                                                self.context.i32_type().const_int(0, false),
                                                self.context.i32_type().const_int(0, false),
                                            ],
                                            "varargs_data",
                                        )
                                    }
                                    .expect("gep varargs data");
                                    llvm_args.push(data_ptr.into());
                                    llvm_args.push(
                                        self.context
                                            .i32_type()
                                            .const_int(len, false)
                                            .as_basic_value_enum()
                                            .into(),
                                    );
                                } else {
                                    for arg in args {
                                        let val = self.compile_expr(arg);
                                        llvm_args.push(val.into());
                                    }
                                }
                            } else {
                                for arg in args {
                                    let val = self.compile_expr(arg);
                                    llvm_args.push(val.into());
                                }
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

                                self.push_default_scope();
                                self.compile_statements(block_stmts)
                                    .expect("Failed to compile trailing closure body");
                                self.pop_default_scope_apply()
                                    .expect("Failed to apply trailing closure defaults");

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
                                ValueKind::Basic(val) => return val,
                                ValueKind::Instruction(_) => {
                                    return self
                                        .context
                                        .i32_type()
                                        .const_int(0, false)
                                        .as_basic_value_enum();
                                }
                            }
                        }

                        // NOTE: 受け手付きメソッドの自動変換はまだ行わない（仕様確定後に実装する）
                    }
                }

                // `a.b` は「引数なし関数 a_b()」として扱う（定数っぽい API 用）
                if tails.len() == 1 {
                    if let ast::Accessor::Property(p) = &tails[0] {
                        let fn_name = format!("{}_{}", head, p);
                        if let Some(function) = self.module.get_function(&fn_name) {
                            let call = self
                                .builder
                                .build_call(function, &[], "gettmp")
                                .expect("call property getter");
                            return match call.try_as_basic_value() {
                                ValueKind::Basic(v) => v,
                                ValueKind::Instruction(_) => self
                                    .context
                                    .i32_type()
                                    .const_int(0, false)
                                    .as_basic_value_enum(),
                            };
                        }
                    }
                }

                if let Some(ast::Accessor::Method(args, trailing_closure)) = tails.first() {
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

                    let function = self
                        .module
                        .get_function(&fn_name)
                        .expect(&format!("Undefined function: {}", head));
                    if function.count_basic_blocks() == 0 {
                        if self.current_module_prefix.is_some()
                            && !self.allowed_externs.contains(&fn_name)
                        {
                            panic!("モジュール内で C 関数 '{}' を呼び出そうとしましたが、このファイル内で `cinclude`（または `use libc.*`）が宣言されていません。", fn_name);
                        }
                    }

                    let mut llvm_args = Vec::new();
                    if let Some(sig) = self.fn_params.get(&fn_name).cloned() {
                        let variadic = sig.iter().position(|p| p.is_variadic);
                        if let Some(var_pos) = variadic {
                            if var_pos + 1 != sig.len() {
                                panic!("可変長引数は最後の1つだけ指定できます。");
                            }
                            let fixed = var_pos;
                            if args.len() < fixed {
                                panic!("引数の数が足りません: {}", fn_name);
                            }
                            for a in args.iter().take(fixed) {
                                let v = self.compile_expr(a);
                                llvm_args.push(v.into());
                            }
                            let var_param = &sig[var_pos];
                            let elem_kind = var_param.ty.as_str();
                            let ptr_t = self.context.ptr_type(AddressSpace::from(0));
                            let (elem_ty, elem_kind_enum) = match elem_kind {
                                "Ptr" | "Any" | "String" => {
                                    (ptr_t.as_basic_type_enum(), VariableKind::Ptr)
                                }
                                "Int" | "i32" => {
                                    (self.context.i32_type().as_basic_type_enum(), VariableKind::I32)
                                }
                                _ => (self.context.i32_type().as_basic_type_enum(), VariableKind::I32),
                            };
                            let var_vals = args.iter().skip(fixed).collect::<Vec<_>>();
                            let len = var_vals.len() as u64;
                            let arr_ty = elem_ty.array_type(var_vals.len() as u32);
                            let arr = self
                                .builder
                                .build_alloca(arr_ty, "varargs_arr")
                                .expect("alloca varargs arr");
                            for (i, a) in var_vals.into_iter().enumerate() {
                                let gep = unsafe {
                                    self.builder.build_in_bounds_gep(
                                        arr_ty,
                                        arr,
                                        &[
                                            self.context.i32_type().const_int(0, false),
                                            self.context.i32_type().const_int(i as u64, false),
                                        ],
                                        "vararg_gep",
                                    )
                                }
                                .expect("gep vararg");
                                let val = self.compile_expr(a);
                                    match elem_kind_enum {
                                        VariableKind::I32 => {
                                            self.builder.build_store(gep, val.into_int_value()).ok();
                                        }
                                        VariableKind::Ptr => {
                                            self.builder.build_store(gep, val.into_pointer_value()).ok();
                                        }
                                        VariableKind::Bool => unreachable!("variadic bool is not supported"),
                                    }
                                }
                            let data_ptr = unsafe {
                                self.builder.build_in_bounds_gep(
                                    arr_ty,
                                    arr,
                                    &[
                                        self.context.i32_type().const_int(0, false),
                                        self.context.i32_type().const_int(0, false),
                                    ],
                                    "varargs_data",
                                )
                            }
                            .expect("gep varargs data");
                            llvm_args.push(data_ptr.into());
                            llvm_args.push(
                                self.context
                                    .i32_type()
                                    .const_int(len, false)
                                    .as_basic_value_enum()
                                    .into(),
                            );
                        } else {
                            for arg in args {
                                let val = self.compile_expr(arg);
                                llvm_args.push(val.into());
                            }
                        }
                    } else {
                        for arg in args {
                            let val = self.compile_expr(arg);
                            llvm_args.push(val.into());
                        }
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

                            self.push_default_scope();
                            self.compile_statements(block_stmts)
                                .expect("Failed to compile trailing closure body");
                            self.pop_default_scope_apply()
                                .expect("Failed to apply trailing closure defaults");

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
                    // 未定義の関数を 0 扱いすると後段で LLVM IR が壊れてセグフォになりやすいので、
                    // 早めに落として原因が分かるようにする。
                    panic!("Undefined function when resolving callchain head: '{head}'");
                }
            }

            Expr::String(s) => {
                let unescaped = Self::unescape_string_literal(s);
                let global_str_ptr = self
                    .builder
                    .build_global_string_ptr(&unescaped, "str_literal")
                    .expect("Codegen: Failed to get global string");

                global_str_ptr.as_basic_value_enum()
            } // TODO: 文字列などはまた実装
        }
    }

    fn to_bool(&self, v: inkwell::values::BasicValueEnum<'ctx>) -> inkwell::values::IntValue<'ctx> {
        match v {
            inkwell::values::BasicValueEnum::IntValue(iv) => {
                if iv.get_type().get_bit_width() == 1 {
                    iv
                } else {
                    self.builder
                        .build_int_compare(
                            IntPredicate::NE,
                            iv,
                            iv.get_type().const_int(0, false),
                            "tobool",
                        )
                        .expect("icmp to bool")
                }
            }
            _ => panic!("bool へ変換できない値です。"),
        }
    }

    fn encode_opt_i32(
        &self,
        v: inkwell::values::BasicValueEnum<'ctx>,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        // int? は i64 の 0=none, (x+1)=some(x)
        if v.is_pointer_value() {
            // none（ptr null）が来た場合は none 扱いにする
            return self
                .context
                .i64_type()
                .const_int(0, false)
                .as_basic_value_enum();
        }
        let iv = v.into_int_value();
        let i64_t = self.context.i64_type();
        if iv.get_type().get_bit_width() == 64 {
            return iv.as_basic_value_enum();
        }
        if iv.get_type().get_bit_width() == 1 {
            // Bool は Optional<Int> にできない
            panic!("int? に bool は入れられません。");
        }
        let ext = self
            .builder
            .build_int_s_extend(iv, i64_t, "opt_ext")
            .expect("sext");
        let enc = self
            .builder
            .build_int_add(ext, i64_t.const_int(1, false), "opt_enc")
            .expect("enc");
        enc.as_basic_value_enum()
    }

    fn encode_opt_ptr(
        &self,
        v: inkwell::values::BasicValueEnum<'ctx>,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        // ptr? は ptr の null=none
        if v.is_pointer_value() {
            return v;
        }
        panic!("ptr? の戻り値はポインタ型である必要があります。");
    }

    fn build_match_pat_cond(
        &self,
        value: inkwell::values::BasicValueEnum<'ctx>,
        pat: &ast::MatchPat,
    ) -> inkwell::values::IntValue<'ctx> {
        let i1_t = self.context.bool_type();
        match pat {
            ast::MatchPat::Wildcard => i1_t.const_int(1, false),
            ast::MatchPat::Integer(n) => {
                let v = value.into_int_value();
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        v,
                        self.context.i32_type().const_int(*n as u64, false),
                        "m_eq",
                    )
                    .expect("icmp")
            }
            ast::MatchPat::Bool(b) => {
                let v = self.to_bool(value);
                self.builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        v,
                        i1_t.const_int(if *b { 1 } else { 0 }, false),
                        "m_beq",
                    )
                    .expect("icmp")
            }
            ast::MatchPat::None => {
                if !value.is_pointer_value() {
                    panic!("none パターンは ptr にだけ使えます。");
                }
                self.builder
                    .build_is_null(value.into_pointer_value(), "m_isnull")
                    .expect("isnull")
            }
            ast::MatchPat::String(_s) => {
                // 今は文字列比較は未実装なので、ポインタ一致のみ
                if !value.is_pointer_value() {
                    panic!("string パターンは ptr にだけ使えます。");
                }
                // string literal は global ptr なので、同一リテラル以外は一致しない
                //（将来的には strcmp を std.string に置く）
                i1_t.const_int(0, false)
            }
            ast::MatchPat::Variant(_name) => {
                // enum の値表現が未実装
                panic!("enum の match は未実装です。");
            }
        }
    }

    fn compile_block_expr(&mut self, stmts: &[Stmt]) -> inkwell::values::BasicValueEnum<'ctx> {
        // 仕様: ブロック式の値は「最後の式」
        if stmts.is_empty() {
            return self
                .context
                .i32_type()
                .const_int(0, false)
                .as_basic_value_enum();
        }

        let last_index = stmts.len() - 1;
        let (prefix, last) = stmts.split_at(last_index);
        if !prefix.is_empty() {
            self.compile_statements(prefix)
                .expect("Failed to compile block prefix statements");
        }

        match &last[0] {
            Stmt::ExprStmt(e) => self.compile_expr(e),
            other => {
                self.compile_statements(std::slice::from_ref(other))
                    .expect("Failed to compile block last statement");
                self.context
                    .i32_type()
                    .const_int(0, false)
                    .as_basic_value_enum()
            }
        }
    }

    fn compile_if_stmt_expr(
        &mut self,
        condition: &Expr,
        then_body: &Stmt,
        else_body: &Option<Box<Stmt>>,
    ) -> inkwell::values::BasicValueEnum<'ctx> {
        let Some(else_body) = else_body else {
            panic!("戻り値のある関数の末尾 if には else が必要です。");
        };

        let cond_tmp = self.compile_expr(condition);
        let cond_i1 = self.to_bool(cond_tmp);

        let parent_func = self
            .builder
            .get_insert_block()
            .expect("if requires insert block")
            .get_parent()
            .expect("if requires parent func");

        let then_bb = self.context.append_basic_block(parent_func, "if_then");
        let else_bb = self.context.append_basic_block(parent_func, "if_else");
        let merge_bb = self.context.append_basic_block(parent_func, "if_merge");

        self.builder
            .build_conditional_branch(cond_i1, then_bb, else_bb)
            .expect("if branch");

        self.builder.position_at_end(then_bb);
        let then_stmts = match then_body {
            Stmt::Bundle { body, .. } => body.as_slice(),
            Stmt::Block(body) => body.as_slice(),
            other => std::slice::from_ref(other),
        };
        let then_val = self.compile_block_expr(then_stmts);
        let then_end = self.builder.get_insert_block().expect("then end");
        if then_end.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .expect("then br merge");
        }

        self.builder.position_at_end(else_bb);
        let else_stmts = match &**else_body {
            Stmt::Bundle { body, .. } => body.as_slice(),
            Stmt::Block(body) => body.as_slice(),
            other => std::slice::from_ref(other),
        };
        let else_val = self.compile_block_expr(else_stmts);
        let else_end = self.builder.get_insert_block().expect("else end");
        if else_end.get_terminator().is_none() {
            self.builder
                .build_unconditional_branch(merge_bb)
                .expect("else br merge");
        }

        self.builder.position_at_end(merge_bb);
        if let Some(first) = merge_bb.get_first_instruction() {
            self.builder.position_before(&first);
        }

        match (then_val, else_val) {
            (inkwell::values::BasicValueEnum::IntValue(ti), inkwell::values::BasicValueEnum::IntValue(ei)) => {
                let phi = self.builder.build_phi(ti.get_type(), "iftmp").expect("phi");
                phi.add_incoming(&[(&ti, then_end), (&ei, else_end)]);
                phi.as_basic_value()
            }
            (inkwell::values::BasicValueEnum::PointerValue(tp), inkwell::values::BasicValueEnum::PointerValue(ep)) => {
                let phi = self.builder.build_phi(tp.get_type(), "iftmp").expect("phi");
                phi.add_incoming(&[(&tp, then_end), (&ep, else_end)]);
                phi.as_basic_value()
            }
            _ => panic!("末尾 if の then/else の型が一致していません。"),
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
