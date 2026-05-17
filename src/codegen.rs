use std::collections::HashMap;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::IntPredicate;
use inkwell::module::Module;
use inkwell::values::{BasicValue, PointerValue, ValueKind};
use log::debug;
use pest::Parser;
use crate::ast;
use crate::ast::{Stmt, Expr, Op, parse_stmt};
use crate::library::LibraryManager;

/// ASTからLLVM IRを生成するコンテキスト
pub struct CodegenContext<'a, 'ctx> {
    pub context: &'ctx Context,
    pub builder: &'a Builder<'ctx>,
    pub module: &'a Module<'ctx>,
    pub variables: HashMap<String, VariableInfo<'ctx>>,
    pub library_manager: &'a LibraryManager,
    pub current_dir: std::path::PathBuf,
}

/// 変数の情報
#[derive(Clone, Debug)]
pub struct VariableInfo<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub is_state: bool,
}

impl<'a, 'ctx> CodegenContext<'a, 'ctx> {
    /// 複数の文（Statements）を順番に LLVM 命令に変換する
    pub fn compile_statements(&mut self, statements: &[Stmt]) -> Result<(), Box<dyn std::error::Error>> {
        for stmt in statements {
            match stmt {
                #[allow(unused)]
                Stmt::Import(path_parts) => {
                    let full_path = path_parts.join(".");

                    if full_path.starts_with("libc.") {
                        self.library_manager.load_c_header(&full_path, self.context, &self.module);
                        continue;
                    }

                    let std_root = std::env::var("KOME_STD_PATH").unwrap_or_else(|_| "./".to_owned());
                    let relative_path = format!("{}.kome", path_parts.join("/"));
                    let mut kome_file_path = std::path::PathBuf::from(std_root);
                    kome_file_path.push(relative_path);

                    if !kome_file_path.exists() {
                        panic!("Standard library not found at: {:?}", kome_file_path);
                    }

                    let source = std::fs::read_to_string(&kome_file_path)
                        .map_err(|_| format!("Failed to read standard library: {:?}", kome_file_path))?;

                    if let Some(parent) = kome_file_path.parent() {
                        self.current_dir = parent.to_path_buf();
                    }

                    let mut c_header_path = kome_file_path.clone();
                    c_header_path.set_extension("h");
                    if c_header_path.exists() {
                        self.library_manager.load_c_header(c_header_path.to_str().unwrap(), self.context, &self.module);
                    }

                    let pairs = match crate::KomeParser::parse(crate::Rule::program, &source) {
                        Ok(p) => p,
                        Err(e) => return Err(format!("Failed to parse {}: {:?}", full_path, e).into()),
                    };

                    let mut std_ast: Vec<Stmt> = Vec::new();

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

                    self.compile_statements(&std_ast)?;
                }

                Stmt::FnDecl { is_public: _, name, body } => {
                    self.compile_function(name, body);
                }
                Stmt::ExprStmt(expr) => {
                    self.compile_expr(expr);
                }

                Stmt::If { condition, then_body, else_body } => {
                    let condition = self.compile_expr(condition);
                    let parent_func = self.builder.get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    let then_bb = self.context.append_basic_block(parent_func, "then");
                    let else_bb = self.context.append_basic_block(parent_func, "else");
                    let merge_bb = self.context.append_basic_block(parent_func, "ifcont");

                    // 条件に応じて分岐
                    self.builder.build_conditional_branch(condition.into_int_value(), then_bb, else_bb)
                        .expect("Failed to build conditional branch");

                    // thenブロックの構築
                    self.builder.position_at_end(then_bb);

                    if let Stmt::Bundle { body, .. } = &**then_body {
                        self.compile_statements(body).expect("Failed to compile then block statements");
                    } else {
                        self.compile_statements(&[*then_body.clone()]).expect("Failed to compile then block statements");
                    }

                    self.builder.build_unconditional_branch(merge_bb)
                        .expect("Failed to build unconditional branch");

                    // elseブロックの構築
                    self.builder.position_at_end(else_bb);
                    if let Some(else_stmt_box) = else_body {
                        if let Stmt::Bundle { body, .. } = &**else_stmt_box {
                            self.compile_statements(body).expect("Failed to compile else block statements");
                        } else {
                            self.compile_statements(&[*else_stmt_box.clone()]).expect("Failed to compile else block statements");
                        }
                    }

                    self.builder.build_unconditional_branch(merge_bb)
                        .expect("Failed to build unconditional branch");

                    // 合流
                    self.builder.position_at_end(merge_bb);
                }

                Stmt::Bundle { name: _bundle_name, body } => {
                    self.compile_statements(body)?;
                }

                #[allow(unused)]
                Stmt::Recipe { is_public, name, state_deps, body } => {
                    debug!("Codegen: Compiling recipe '{}' (deps: {:?})", name, state_deps);

                    self.variables.clear();
                    let func = self.module.add_function(name, self.context.void_type().fn_type(&[], false), None);

                    // レシピ関数は引数なし・戻り値なしにすうｒ（仮想的に関数にしているだけで言語的には関数じゃない）
                    let void_type = self.context.void_type();
                    let recipe_fn_type = void_type.fn_type(&[], false);

                    // 関数名は bundle 名とレシピ名を組み合わせて一意にする
                    let recipe_fn_name = format!("App_recipe_{}", name);
                    let recipe_function = self.module.add_function(&recipe_fn_name, recipe_fn_type, None);

                    // レシピ関数の中身をビルドするための新しいブロックを作成
                    let recipe_entry_bb = self.context.append_basic_block(recipe_function, "entry");

                    // 現在のビルダーの位置（main関数など）を一時保存しておく
                    let current_bb = self.builder.get_insert_block().unwrap();

                    // ビルダーをレシピ関数の中に移動して、中身（printfなど）をコンパイル
                    self.builder.position_at_end(recipe_entry_bb);

                    let recipe_stmts: Vec<Stmt> = vec![Stmt::ExprStmt(body.clone())];
                    self.compile_statements(&recipe_stmts);

                    // レシピ関数の最後にreturn voidを挟む
                    self.builder.build_return(None)
                        .expect("Failed to build return for recipe function");

                    self.builder.position_at_end(current_bb);

                    let subscribe_fn = match self.module.get_function("__kome_runtime_subscribe") {
                        Some(f) => f,
                        None => {
                            let address_space = inkwell::AddressSpace::from(0);
                            let generic_ptr_type = self.context.ptr_type(address_space);

                            let sub_fn_type = void_type.fn_type(
                                &[generic_ptr_type.into(), generic_ptr_type.into()],
                                false
                            );
                            self.module.add_function("__kome_runtime_subscribe", sub_fn_type, None)
                        }
                    };

                    // 依存している全てのstate変数に対して、このレシピ関数を登録する命令を生成
                    for dep_var in state_deps {
                        // 変数名文字列のグローバルポインタを作成
                        let dep_var_global = self.builder.build_global_string_ptr(dep_var, "dep_var_name")
                            .expect("Failed to generate global string ptr");

                        // レシピ関数のポインタを取得
                        let recipe_fn_ptr = recipe_function.as_global_value().as_pointer_value();

                        self.builder.build_call(
                            subscribe_fn,
                            &[dep_var_global.as_pointer_value().into(), recipe_fn_ptr.into()],
                            "subscribe_call"
                        ).expect("Failed to build runtime subscribe call");

                        debug!("Codegen: Registered '{}' to look at state '{}'", recipe_fn_name, dep_var);
                    }
                }

                Stmt::Declaration { is_public: _, is_state, is_mut: _, name, value, range: _ } => {
                    let llvm_value = self.compile_expr(value);
                    let i32_type = self.context.i32_type();

                    if *is_state {
                        // すでに同名のグローバル変数がないかチェック
                        let global_var = match self.module.get_global(name) {
                            Some(g) => g,
                            None => {
                                let g = self.module.add_global(i32_type, None, name);
                                // 初期値をセット（型に合わせて llvm_value から定数を取り出すか、一旦0初期化）
                                g.set_initializer(&i32_type.const_int(0, false));
                                g
                            }
                        };

                        // 関数を跨いで使えるようにグローバルポインタを登録
                        self.variables.insert(name.clone(), VariableInfo {
                            ptr: global_var.as_pointer_value(),
                            is_state: true,
                        });

                    } else {
                        let ptr = self.builder.build_alloca(i32_type, name)
                            .expect("Failed to allocate variable");

                        // 確保したメモリに初期値をストア
                        self.builder.build_store(ptr, llvm_value)
                            .expect("Failed to store initial value");

                        // ローカル変数としてマップに登録
                        self.variables.insert(name.clone(), VariableInfo {
                            ptr,
                            is_state: false,
                        });
                    }
                }

                Stmt::Assignment { is_default: _, name, value } => {
                    let short_name = name.split('.').last().unwrap().to_string();

                    let ptr = if let Some(var_info) = self.variables.get(name) {
                        var_info.ptr
                    } else if let Some(var_info) = self.variables.get(&short_name) {
                        var_info.ptr
                    } else if let Some(global_var) = self.module.get_global(name).or_else(|| self.module.get_global(&short_name)) {
                        global_var.as_pointer_value()
                    } else {
                        panic!("Undefined variable for assignment: {} (short: {})", name, short_name);
                    };

                    let current_val = self.builder.build_load(self.context.i32_type(), ptr, "loadtmp")
                        .expect("Failed to load variable")
                        .into_int_value();
                    let rhs_val = self.compile_expr(value).into_int_value();
                    let new_val = self.builder.build_int_add(current_val, rhs_val, "addtmp").expect("Failed to build add");
                    self.builder.build_store(ptr, new_val).expect("Failed to store");
                }

                Stmt::While { condition, body } => {
                    let parent_func = self.builder.get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    let cond_bb = self.context.append_basic_block(parent_func, "while_cond");
                    let body_bb = self.context.append_basic_block(parent_func, "while_body");
                    let end_bb = self.context.append_basic_block(parent_func, "while_end");

                    self.builder.build_unconditional_branch(cond_bb)
                        .expect("Failed to branch to while condition");

                    self.builder.position_at_end(cond_bb);
                    let cond_val = self.compile_expr(condition);
                    self.builder.build_conditional_branch(cond_val.into_int_value(), body_bb, end_bb)
                        .expect("Failed to build while conditional branch");

                    self.builder.position_at_end(body_bb);
                    if let Stmt::Bundle { body: body_stmts, .. } = &**body {
                        self.compile_statements(body_stmts).expect("Failed to compile while body statements");
                    } else {
                        self.compile_statements(&[*body.clone()]).expect("Failed to compile while body statements");
                    }

                    self.builder.build_unconditional_branch(cond_bb)
                        .expect("Failed to loop back to while condition");

                    self.builder.position_at_end(end_bb);
                }

                Stmt::For { init, condition, update, body } => {
                    let parent_func = self.builder.get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();

                    // 条件式（i < end）の左辺からループ変数名（"i" など）を特定する
                    let loop_var_name = if let Expr::BinaryOp { left, .. } = condition {
                        if let Expr::Ident(name) = &**left {
                            name.clone()
                        } else {
                            panic!("Codegen Error: For loop condition left-hand side must be an identifier");
                        }
                    } else {
                        panic!("Codegen Error: Invalid for loop condition structure");
                    };

                    // 修正: VariableInfoの参照ではなく、内部の ptr (PointerValue) を直接コピーして受け取る
                    let loop_var_ptr = if let Some(info) = self.variables.get(&loop_var_name) {
                        info.ptr // コピーされるため、ここで self の借用は終わる
                    } else {
                        let i32_type = self.context.i32_type();
                        let new_alloc = self.builder.build_alloca(i32_type, &loop_var_name)
                            .expect("Failed to allocate for-loop variable");

                        self.variables.insert(
                            loop_var_name.clone(),
                            VariableInfo { ptr: new_alloc, is_state: false }
                        );
                        new_alloc
                    };

                    // 修正: alloc_ptr.ptr ではなく、上で取り出した loop_var_ptr を使う
                    let start_val = self.compile_expr(init);
                    self.builder.build_store(loop_var_ptr, start_val)
                        .expect("Failed to store for-loop init value");

                    let cond_bb = self.context.append_basic_block(parent_func, "for_cond");
                    let body_bb = self.context.append_basic_block(parent_func, "for_body");
                    let end_bb = self.context.append_basic_block(parent_func, "for_end");

                    // 条件チェックへジャンプ
                    self.builder.build_unconditional_branch(cond_bb)
                        .expect("Failed to branch to for condition");

                    // 条件チェック
                    self.builder.position_at_end(cond_bb);
                    let cond_val = self.compile_expr(condition);
                    self.builder.build_conditional_branch(cond_val.into_int_value(), body_bb, end_bb)
                        .expect("Failed to build for conditional branch");

                    // ループ本体
                    self.builder.position_at_end(body_bb);

                    let stmts_to_compile: Vec<Stmt> = match &**body {
                        Stmt::Bundle { body: body_stmts, .. } => body_stmts.clone(),
                        other_stmt => vec![other_stmt.clone()],
                    };

                    // これでもうどこからも self は借用されていないので、安全に呼び出せます！
                    self.compile_statements(&stmts_to_compile).expect("Failed to compile for loop body statements");
                    // インクリメント
                    if let Some(update_expr) = update {
                        let next_val = self.compile_expr(update_expr);
                        self.builder.build_store(loop_var_ptr, next_val)
                            .expect("Failed to update for-loop counter");
                    }

                    // 条件チェックブロックへ戻る
                    self.builder.build_unconditional_branch(cond_bb)
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
                // ローカル変数->グローバル
                let ptr = if let Some(var_info) = self.variables.get(name) {
                    var_info.ptr
                } else if let Some(global_var) = self.module.get_global(name) {
                    global_var.as_pointer_value()
                } else {
                    panic!("Undefined variable: {}", name);
                };

                self.builder.build_load(self.context.i32_type(), ptr, name)
                    .expect("Failed to load variable")
            }
            Expr::BinaryOp {left, op, right} => {
                // 左辺と右辺をそれぞれLLVM IRにする
                let left_val = self.compile_expr(left);
                let right_val = self.compile_expr(right);

                match op {
                    Op::Add => {
                        self.builder.build_int_add(left_val.into_int_value(), right_val.into_int_value(), "addtmp")
                            .expect("Failed to build add instruction")
                            .as_basic_value_enum()
                    }
                    Op::Sub => {
                        self.builder.build_int_sub(left_val.into_int_value(), right_val.into_int_value(), "subtmp")
                            .expect("Failed to build sub instruction")
                            .as_basic_value_enum()
                    }
                    Op::Mul => {
                        self.builder.build_int_mul(left_val.into_int_value(), right_val.into_int_value(), "multmp")
                            .expect("Failed to build mul instruction")
                            .as_basic_value_enum()
                    }
                    Op::Div => {
                        self.builder.build_int_signed_div(left_val.into_int_value(), right_val.into_int_value(), "divtmp")
                            .expect("Failed to build div instruction")
                            .as_basic_value_enum()
                    }
                    Op::Eq => { // ==
                        self.builder.build_int_compare(IntPredicate::EQ, left_val.into_int_value(), right_val.into_int_value(), "eqtmp")
                            .expect("Failed to build icmp eq instruction")
                            .as_basic_value_enum()
                    }
                    Op::Neq => { // !=
                        self.builder.build_int_compare(IntPredicate::NE, left_val.into_int_value(), right_val.into_int_value(), "netmp")
                            .expect("Failed to build icmp ne instruction")
                            .as_basic_value_enum()
                    }
                    Op::Lt => { // <
                        self.builder.build_int_compare(IntPredicate::SLT, left_val.into_int_value(), right_val.into_int_value(), "lttmp")
                            .expect("Failed to build icmp slt instruction")
                            .as_basic_value_enum()
                    }
                    Op::Gt => { // >
                        self.builder.build_int_compare(IntPredicate::SGT, left_val.into_int_value(), right_val.into_int_value(), "gttmp")
                            .expect("Failed to build icmp sgt instruction")
                            .as_basic_value_enum()
                    }
                    Op::Le => { // <=
                        self.builder.build_int_compare(IntPredicate::SLE, left_val.into_int_value(), right_val.into_int_value(), "letmp")
                            .expect("Failed to build icmp sle instruction")
                            .as_basic_value_enum()
                    }
                    Op::Ge => { // >=
                        self.builder.build_int_compare(IntPredicate::SGE, left_val.into_int_value(), right_val.into_int_value(), "getmp")
                            .expect("Failed to build icmp sge instruction")
                            .as_basic_value_enum()
                    }
                    Op::And => { // &&
                        self.builder.build_and(left_val.into_int_value(), right_val.into_int_value(), "andtmp")
                            .expect("Failed to build logical and instruction")
                            .as_basic_value_enum()
                    }
                    Op::Or => { // ||
                        self.builder.build_or(left_val.into_int_value(), right_val.into_int_value(), "ortmp")
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
                }
            }
            Expr::Block(stmts) => {
                self.compile_statements(&*stmts.clone()).expect("Failed to compile block statements");
                self.context.i32_type().const_int(0, false).as_basic_value_enum()
            }
            Expr::CallChain { head, tails} => {
                // LLVM moduleなるものから関数を探す
                if let Some(ast::Accessor::Method(args, trailing_closure)) = tails.first() {
                    let function = self.module.get_function(head)
                        .expect(&format!("Undefined function: {}", head));

                    // 引数をLLVM Valueに変換
                    let mut llvm_args = Vec::new();

                    for arg in args {
                        let val = self.compile_expr(arg);
                        llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(val));
                    }

                    // トレイリングクロージャが存在する場合、無名関数を作って引数の末尾に追加する
                    if let Some(block_stmts) = trailing_closure {
                        // 一意な無名関数名を決定（ __kome_anon_closure_N）
                        let mut i = 0;
                        let closure_name = loop {
                            let name = format!("__kome_anon_closure_{}", i);
                            if self.module.get_function(&name).is_none() {
                                break name;
                            }
                            i += 1;
                        };

                        // 無名関数の型を定義 (void -> void)
                        let void_type = self.context.void_type();
                        let closure_fn_type = void_type.fn_type(&[], false);
                        let closure_function = self.module.add_function(&closure_name, closure_fn_type, None);

                        // 基本ブロックを生成して、一時的にビルダーの挿入先を無名関数の中へと移動させる
                        let entry_bb = self.context.append_basic_block(closure_function, "entry");
                        let current_bb = self.builder.get_insert_block().unwrap();
                        self.builder.position_at_end(entry_bb);

                        // クロージャ内のステートメント群（onPress内の処理など）をコンパイル
                        self.compile_statements(block_stmts).expect("Failed to compile trailing closure body");

                        // void関数なので値を返さずにReturn
                        self.builder.build_return(None).expect("Failed to build return for closure");
                        self.builder.position_at_end(current_bb);

                        let closure_ptr = closure_function.as_global_value().as_pointer_value();
                        llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(closure_ptr));
                    }

                    let call = self.builder.build_call(function, &llvm_args, "calltmp")
                        .expect("Codegen: Failed to build function call");

                    // 返り値の処理
                    match call.try_as_basic_value() {
                        ValueKind::Basic(val) => {
                            val
                        }
                        ValueKind::Instruction(_) => {
                            self.context.i32_type().const_int(0, false).as_basic_value_enum()
                        }
                    }
                } else {
                    panic!("Codegen: Undefined function: {}", head);
                }
            }
            Expr::String(s) => {
                let unescaped = s.replace("\\n", "\n");
                let global_str_ptr = self.builder
                    .build_global_string_ptr(&unescaped, "str_literal")
                    .expect("Codegen: Failed to get global string");

                global_str_ptr.as_basic_value_enum()
            }
            // TODO: 文字列などはまた実装
        }
    }

    /// 関数をコンパイルする
    fn compile_function(&mut self, name: &str, body: &[Stmt]) {
        let func = self.module.add_function(name, self.context.void_type().fn_type(&[], false), None);
        let bb = self.context.append_basic_block(func, "entry");
        self.builder.position_at_end(bb);

        self.compile_statements(body).unwrap();

        self.builder.build_return(Some(&self.context.i32_type().const_int(0, false))).unwrap();
    }
}