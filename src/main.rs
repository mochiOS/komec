mod ast;

use inkwell::context::Context;
use inkwell::OptimizationLevel;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "syntax/main.pest"]
pub struct KomeParser;

fn main() {
    let context = Context::create();
    let module = context.create_module("main");
    let builder = context.create_builder();

    // 型関係の変数
    let i32_type = context.i32_type();
    let _i8_type = context.i8_type();
    let i8_ptr_type = context.ptr_type(inkwell::AddressSpace::from(1u16));

    // printf関数を宣言
    let printf_fn_type = i32_type.fn_type(&[i8_ptr_type.into()], true);
    let printf_function = module.add_function("printf", printf_fn_type, None);

    // main関数を宣言
    let main_fn_type = i32_type.fn_type(&[], false);
    let main_function = module.add_function("main", main_fn_type, None);

    // main関数にBasic Blockを追加
    let entry_basic_block = context.append_basic_block(main_function, "entry");
    // builderのpositionをentry Basic Blockに設定
    builder.position_at_end(entry_basic_block);

    // ここからmain関数に命令をビルドしていく
    // globalに文字列を宣言
    let hw_string_ptr = builder.build_global_string_ptr("Hello, world!", "hw")
        .expect("Failed to create global string pointer");

    // printfをcall
    builder.build_call(printf_function, &[hw_string_ptr.as_pointer_value().into()], "call").expect("Failed to call printf");
    // main関数は0を返す
    builder.build_return(Some(&i32_type.const_int(0, false))).expect("main function should return a value");

    // JIT実行エンジンを作成し、main関数を実行
    let execution_engine = module.create_jit_execution_engine(OptimizationLevel::Aggressive).unwrap();
    unsafe {
        execution_engine.get_function::<unsafe extern "C" fn()>("main").unwrap().call();
    }
}
