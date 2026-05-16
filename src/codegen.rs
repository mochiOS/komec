use std::collections::HashMap;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::IntPredicate;
use inkwell::module::Module;
use inkwell::values::{BasicValue, PointerValue, ValueKind};
use log::debug;
use crate::ast;
use crate::ast::{Stmt, Expr, Op};
use crate::library::LibraryManager;

/// ASTからLLVM IRを生成するコンテキスト
pub struct CodegenContext<'a, 'ctx> {
    pub context: &'ctx Context,
    pub builder: &'a Builder<'ctx>,
    pub module: &'a Module<'ctx>,
    pub variables: HashMap<String, VariableInfo<'ctx>>,
}

/// 変数の情報
#[derive(Clone, Debug)]
pub struct VariableInfo<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub is_state: bool,
}

impl<'a, 'ctx> CodegenContext<'a, 'ctx> {
    /// 複数の文（Statements）を順番に LLVM 命令に変換する
    pub fn compile_statements(&mut self, statements: &[Stmt]) {
        for stmt in statements {
            match stmt {
                #[allow(unused)]
                Stmt::Declaration { is_public, is_state, is_mut, name, value, range: _range } => {
                    let llvm_value = self.compile_expr(value);
                    let i32_type = self.context.i32_type();
                    let alloc = self.builder.build_alloca(i32_type, name)
                        .expect("Failed to allocate variable");

                    self.builder.build_store(alloc, llvm_value)
                        .expect("Failed to store initial value");

                    let info = VariableInfo {
                        ptr: alloc,
                        is_state: *is_state,
                    };
                    self.variables.insert(name.clone(), info);

                    debug!("Codegen: Generated variable '{}' (state: {})", name, is_state);
                }
                Stmt::Import(path) => {
                    let full_path = path.join(".");
                    debug!("Codegen: importing library: {}", full_path);

                    let success = LibraryManager::new().load_c_header(&full_path, self.context, self.module);

                    if !success {
                        panic!("Codegen: Failed to load library: {}", full_path);
                    }
                }
                Stmt::ExprStmt(expr) => {
                    self.compile_expr(expr);
                }

                #[allow(unused)]
                Stmt::FnDecl { is_public, name, body } => {
                    // TODO: 必要に応じて is_public の情報を compile_function に引き渡す
                    self.compile_function(name, body);
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
                        self.compile_statements(body);
                    } else {
                        self.compile_statements(&[*then_body.clone()]);
                    }

                    self.builder.build_unconditional_branch(merge_bb)
                        .expect("Failed to build unconditional branch");

                    // elseブロックの構築
                    self.builder.position_at_end(else_bb);
                    if let Some(else_stmt_box) = else_body {
                        if let Stmt::Bundle { body, .. } = &**else_stmt_box {
                            self.compile_statements(body);
                        } else {
                            self.compile_statements(&[*else_stmt_box.clone()]);
                        }
                    }

                    self.builder.build_unconditional_branch(merge_bb)
                        .expect("Failed to build unconditional branch");

                    // 合流
                    self.builder.position_at_end(merge_bb);
                }

                Stmt::Bundle { name, body } => {
                    debug!("Codegen: Entering bundle namespace: {}", name);
                    // TODO: 内部の変数やレシピをこの名前空間に紐付ける処理（シンボルテーブルの階層化など）
                    self.compile_statements(body);
                }

                #[allow(unused)]
                Stmt::Recipe { is_public, name, state_deps, body } => {
                    debug!("Codegen: Compiling recipe '{}' (public: {}, deps: {:?})", name, is_public, state_deps);
                    // TODO: 構造を返す隠しLLVM関数の生成
                }

                Stmt::Assignment { is_default, name, value } => {
                    debug!("Codegen: Assigning to '{}' (is_default: {})", name, is_default);
                    let new_llvm_value = self.compile_expr(value);

                    if let Some(info) = self.variables.get(name) {

                        // 値をメモリ（allocaポインタ）に書き戻す
                        self.builder.build_store(info.ptr, new_llvm_value)
                            .expect(&format!("Failed to store value for variable '{}'", name));

                        // もし対象の変数がstate変数だった場合通知関数を仕込む
                        if info.is_state {
                            // モジュールから通知関数（プロトタイプ）を取得
                            let notify_fn = match self.module.get_function("__kome_runtime_notify_change") {
                                Some(f) => f,
                                None => {
                                    // なければその場で外部関数として登録void __kome_runtime_notify_change(char*)
                                    let void_type = self.context.void_type();
                                    let i8_ptr_type = self.context.i32_type().ptr_type(inkwell::AddressSpace::from(0));
                                    let fn_type = void_type.fn_type(&[i8_ptr_type.into()], false);
                                    self.module.add_function("__kome_runtime_notify_change", fn_type, None)
                                }
                            };

                            // 変数名の文字列をLLVMのグローバル文字列ポインタとして生成
                            let var_name_global = self.builder.build_global_string_ptr(name, "state_var_name")
                                .expect("Failed to generate global string ptr");

                            // 関数を呼び出す命令（Call）を挿入
                            self.builder.build_call(notify_fn, &[var_name_global.as_pointer_value().into()], "notify_call")
                                .expect("Failed to build runtime notify call");

                            debug!("Codegen: Inserted state change hook for '{}'", name);
                        }
                    } else {
                        panic!("Codegen Error: Variable '{}' not found for assignment", name);
                    }
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
                        self.compile_statements(body_stmts);
                    } else {
                        self.compile_statements(&[*body.clone()]);
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
                    self.compile_statements(&stmts_to_compile);
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
    }

    /// 式（Expression）を評価し、LLVM の値（BasicValueEnum）を返す
    fn compile_expr(&self, expr: &Expr) -> inkwell::values::BasicValueEnum<'ctx> {
        match expr {
            Expr::Integer(val) => {
                // 整数リテラルをLLVMのi32に変換
                // TODO: ここも型推論
                let i32_type = self.context.i32_type();
                i32_type.const_int(*val as u64, false).as_basic_value_enum()
            }
            Expr::Ident(name) => {
                // 変数名から値を取り出す
                let alloc = self.variables.get(name)
                    .expect(&format!("Undefined variable: {}", name));
                let i32_type = self.context.i32_type();
                self.builder.build_load(i32_type, alloc.ptr, name)
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
            Expr::CallChain { head, tails} => {
                // LLVM moduleなるものから関数を探す
                if let Some(ast::Accessor::Method(args)) = tails.first() {
                    let function = self.module.get_function(head)
                        .expect(&format!("Undefined function: {}", head));

                    // 引数をLLVM Valueに変換
                    let mut llvm_args = Vec::new();

                    for arg in args {
                        let val = self.compile_expr(arg);
                        llvm_args.push(inkwell::values::BasicMetadataValueEnum::from(val));
                    }

                    let call = self.builder.build_call(function, &llvm_args, "calltmp")
                        .expect("Codegen: Failed to build function call");

                    // 返り値の処理
                    match call.try_as_basic_value() {
                        ValueKind::Basic(val) => {
                            val
                        }
                        // Voidとかでも一旦0: i32を返す
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
    pub fn compile_function(&mut self, name: &str, body: &[Stmt]) {
        let i32_type = self.context.i32_type();
        let fn_type = i32_type.fn_type(&[], false); // TODO: 考慮引数対応

        let function = self.module.add_function(name, fn_type, None);
        let entry_block = self.context.append_basic_block(function, "entry");

        self.builder.position_at_end(entry_block);

        self.variables.clear();
        self.compile_statements(body);

        self.builder.build_return(Some(&i32_type.const_int(0, false)))
            .expect("Function should return a value");
    }
}