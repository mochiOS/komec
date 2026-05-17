use std::fs;
use std::env;
use inkwell::context::Context;
use inkwell::OptimizationLevel;
use pest_derive::Parser;
use pest::Parser;
use env_logger;
use log::*;
use crate::codegen::CodegenContext;
use crate::library::LibraryManager;

mod ast;
mod codegen;
pub mod library;
mod state;

#[derive(Parser)]
#[grammar = "syntax/main.pest"]
pub struct KomeParser;

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();

    unsafe { env::set_var("RUST_LOG", "debug"); }

    if args.len() == 1 {
        println!("Usage: {} <source_file>", args[0]);
        return;
    }


    if args.len() > 2 && args[2] == "-d" {
        unsafe { env::set_var("RUST_LOG", "debug"); }
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

    debug!("Generated AST:");
    for stmt in &ast_state {
        debug!("{:?}", stmt);
    }

    let context = Context::create();
    let module = context.create_module("main");
    let builder = context.create_builder();
    let mut codegen = CodegenContext {
        context: &context,
        builder: &builder,
        module: &module,
        variables: std::collections::HashMap::new(),
        library_manager: &LibraryManager::new(),
        current_dir: std::path::PathBuf::new(),
    };

    let i32_type = context.i32_type();
    let main_fn_type = i32_type.fn_type(&[], false);
    let main_function = module.add_function("main", main_fn_type, None);
    let entry_block = context.append_basic_block(main_function, "entry");

    builder.position_at_end(entry_block);
    codegen.compile_statements(&ast_state).expect("Failed to compile statements");

    let zero = i32_type.const_int(0, false);
    builder.build_return(Some(&zero)).expect("Failed to build main return");

    // デバッグ用LLVM IR
    println!("Generated LLVM IR:\n{}", module.print_to_string().to_string());

    let execution_engine = module.create_jit_execution_engine(OptimizationLevel::Aggressive).unwrap();

    unsafe {
        if let Ok(main_function) = execution_engine.get_function::<unsafe extern "C" fn() -> i32>("main") {
            let _result = main_function.call();
        } else {
            println!("Runtime Error: main function is not defined in the source file.");
        }
    }
}