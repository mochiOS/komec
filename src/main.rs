use std::fs;
use std::env;
use inkwell::context::Context;
use inkwell::OptimizationLevel;
use pest_derive::Parser;
use pest::Parser;

mod ast;

#[derive(Parser)]
#[grammar = "syntax/main.pest"]
pub struct KomeParser;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        println!("Usage: {} <source_file>", args[0]);
        return;
    }

    let mut debug = false;
    if args[2] == "-d" {
        debug = true;
    }

    let source_file = args[1].clone();
    if let Err(e) = fs::read_to_string(&source_file) {
        println!("Error reading file {}: {}", source_file, e);
        return;
    }

    // パース前の生ソース
    let raw_source = match fs::read_to_string(&source_file) {
        Ok(content) => content,
        Err(_) => {
            println!("Error reading file: {}", source_file);
            return;
        }
    };

    // パースした結果（Pair型）
    let parse = match KomeParser::parse(Rule::program, &raw_source) {
        Ok(parse) => parse,
        Err(e) => {
            println!("Parse error:\n{}", e);
            return;
        }
    };

    let mut ast_state: Vec<ast::Stmt> = Vec::new();

    // ルールに基づいて中身取り出す
    if let Some(pair) = parse.into_iter().next() {
        for pair in pair.into_inner() {
            match pair.as_rule() {
                Rule::stmt => {
                    let inner_stmt = pair.into_inner().next().unwrap();

                    let ast_node = ast::parse_stmt(inner_stmt);
                    ast_state.push(ast_node);
                }
                Rule::EOI => {
                    /* ignore */
                }
                _ => {
                    println!("Invalid rule: {:?}", pair.as_rule());
                }
            }
        }
    }

    if debug == true {
        println!("Generated AST:");
        for stmt in ast_state {
            println!("{:?}", stmt);
        }
    }

    let context = Context::create();
    let module = context.create_module("main");
    let builder = context.create_builder();

    // 基本型の定義
    let i32_type = context.i32_type();
    let i8_ptr_type = context.ptr_type(inkwell::AddressSpace::from(0u16));

    // printf関数
    let printf_fn_type = i32_type.fn_type(&[i8_ptr_type.into()], true);
    let printf_function = module.add_function("printf", printf_fn_type, None);

    // main関数
    let main_fn_type = i32_type.fn_type(&[], false);
    let main_function = module.add_function("main", main_fn_type, None);

    let entry_basic_block = context.append_basic_block(main_function, "entry");
    builder.position_at_end(entry_basic_block);

    // TODO: ここで本来はさっき作った `ast_statements` をループで回す

    let hw_string_ptr = builder.build_global_string_ptr("Hello, world!", "hw")
        .expect("Failed to create global string pointer");

    builder.build_call(printf_function, &[hw_string_ptr.as_pointer_value().into()], "call")
        .expect("Failed to call printf");

    builder.build_return(Some(&i32_type.const_int(0, false)))
        .expect("main function should return a value");

    let execution_engine = module.create_jit_execution_engine(OptimizationLevel::Aggressive).unwrap();
    unsafe {
        execution_engine.get_function::<unsafe extern "C" fn()>("main").unwrap().call();
    }
}
