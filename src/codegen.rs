use crate::ast::{Stmt, Expr, Op};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicValue, PointerValue, ValueKind};
use std::collections::HashMap;
use crate::ast;

/// ASTからLLVM IRを生成するコンテキスト
pub struct CodegenContext<'a, 'ctx> {
    pub context: &'ctx Context,
    pub builder: &'a Builder<'ctx>,
    pub module: &'a Module<'ctx>,
    pub variables: HashMap<String, PointerValue<'ctx>>,
}

impl<'a, 'ctx> CodegenContext<'a, 'ctx> {
    /// 複数の文（Statements）を順番に LLVM 命令に変換する
    pub fn compile_statements(&mut self, statements: &[Stmt]) {
        for stmt in statements {
            match stmt {
                Stmt::Declaration { is_state, is_mut, name, value, range } => {
                    // 右辺の式を評価してLLVM Valueに変換
                    let llvm_value = self.compile_expr(value);

                    // TODO: 一旦i32固定としてやってるので型推論つくる
                    let i32_type = self.context.i32_type();
                    let alloc = self.builder.build_alloca(i32_type, name)
                        .expect("Failed to allocate variable");

                    // 確保した領域に初期値をストア (store)
                    self.builder.build_store(alloc, llvm_value)
                        .expect("Failed to store initial value");

                    // この変数を使えるようにシンボルテーブルに登録
                    self.variables.insert(name.clone(), alloc);

                    // TODO: is_stateやrange(within ~ cycle)のロジックは、変数への代入命令を処理するときに境界チェックを行うBasic Blockを挟む形で実装する
                    println!("Codegen: Generated variable '{}' (state: {}, mut: {})", name, is_state, is_mut);
                }
                Stmt::Import(_) => {
                    // インポートはコンパイル時のメタ情報やからLLVM命令の生成はスキップ
                }
                _ => println!("Codegen: Unknown statement: {:?}", stmt),
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
                self.builder.build_load(i32_type, *alloc, name)
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
                    _ => todo!("Codegen: Unknown binary op: {:?}", op),
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
            // TODO: 文字列などはまた実装
            _ => panic!("Codegen: Undefined expression: {:?}", expr),
        }
    }
}