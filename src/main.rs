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
use std::ffi::CString;
use inkwell::values::AsValueRef;

// Declare the C runtime functions as extern so we can obtain their addresses
// directly (without dlsym). These symbols are provided by the C sources
// compiled into the crate by build.rs.
unsafe extern "C" {
    fn __kome_runtime_start_loop();
    fn __kome_runtime_subscribe(name: *const std::os::raw::c_char, cb: *const ());
    fn __kome_runtime_process_events();
    fn __kome_runtime_emit(name: *const std::os::raw::c_char);
}

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

    unsafe { env::set_var("RUST_LOG", "debug!"); }

    if args.len() == 1 {
        println!("Usage: {} <source_file>", args[0]);
        return;
    }


    if args.len() > 2 && args[2] == "-d" {
        unsafe { env::set_var("RUST_LOG", "debug!"); }
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
        current_module_prefix: None,
        allowed_externs: std::collections::HashSet::new(),
    };

    for stmt in &ast_state {
        match stmt {
            ast::Stmt::FnDecl { .. } | ast::Stmt::Recipe { .. } | ast::Stmt::Bundle { .. } | ast::Stmt::Import(..) => {
                // 宣言文だけをコンパイル
                codegen.compile_statements(&[stmt.clone()]).expect("Failed to compile declarations");
            }
            _ => {}
        }
    }

    let i32_type = context.i32_type();
    let entry_fn_type = i32_type.fn_type(&[], false);
    let entry_function = module.add_function("__kome_entry", entry_fn_type, None);
    let entry_block = context.append_basic_block(entry_function, "entry");

    builder.position_at_end(entry_block);

    // Call any registration functions (e.g., onPress) to set up subscriptions
    if let Some(on_press_fn) = module.get_function("onPress") {
        builder.build_call(on_press_fn, &[], "call_onpress").ok();
    }



    for stmt in &ast_state {
        match stmt {
            ast::Stmt::Declaration { .. } | ast::Stmt::Assignment { .. } | ast::Stmt::ExprStmt(..) => {
                codegen.compile_statements(&[stmt.clone()]).expect("Failed to compile entry logic");
            }
            _ => {}
        }
    }

    let zero = i32_type.const_int(0, false);
    builder.build_return(Some(&zero)).expect("Failed to build entry return");

    // デバッグ用
    module.print_to_stderr();

    let execution_engine = module.create_jit_execution_engine(OptimizationLevel::Aggressive).unwrap();

    // Register known C runtime symbols with the JIT so external calls resolve correctly.
    // We obtain direct addresses of the linked C functions via extern declarations
    // above and register them with the JIT. This avoids needing -export-dynamic
    // and also avoids dlsym/libclang interaction issues.
    unsafe {
        if let Some(fn_val) = module.get_function("__kome_runtime_start_loop") {
            let gv = fn_val.as_global_value();
            execution_engine.add_global_mapping(&gv, __kome_runtime_start_loop as usize);
            debug!("[jit-map] mapped __kome_runtime_start_loop -> {:p}", __kome_runtime_start_loop as *const ());
        }

        if let Some(fn_val) = module.get_function("__kome_runtime_subscribe") {
            let gv = fn_val.as_global_value();
            execution_engine.add_global_mapping(&gv, __kome_runtime_subscribe as usize);
            debug!("[jit-map] mapped __kome_runtime_subscribe -> {:p}", __kome_runtime_subscribe as *const ());
        }

        if let Some(fn_val) = module.get_function("__kome_runtime_process_events") {
            let gv = fn_val.as_global_value();
            execution_engine.add_global_mapping(&gv, __kome_runtime_process_events as usize);
            debug!("[jit-map] mapped __kome_runtime_process_events -> {:p}", __kome_runtime_process_events as *const ());
        }

        if let Some(fn_val) = module.get_function("__kome_runtime_emit") {
            let gv = fn_val.as_global_value();
            execution_engine.add_global_mapping(&gv, __kome_runtime_emit as usize);
            debug!("[jit-map] mapped __kome_runtime_emit -> {:p}", __kome_runtime_emit as *const ());
        }
    }

    unsafe {
        if let Ok(entry_fn) = execution_engine.get_function::<unsafe extern "C" fn() -> i32>("__kome_entry") {
            debug!("[runtime] calling __kome_entry()");
            entry_fn.call();
            debug!("[runtime] returned from __kome_entry()");
        } else {
            println!("Runtime Error: Entry function is not defined.");
        }
    }

    // If a main() function was generated, call it so user code runs
    unsafe {
        if let Ok(main_fn) = execution_engine.get_function::<unsafe extern "C" fn()>("main") {
            debug!("[runtime] calling main()");
            debug!("Calling generated main() function");
            main_fn.call();
            debug!("[runtime] returned from main()");
        }
    }

    // After main() runs, process any registered event callbacks
    unsafe {
        if let Ok(process_events) = execution_engine.get_function::<unsafe extern "C" fn()>("__kome_runtime_process_events") {
            debug!("[runtime] calling __kome_runtime_process_events()");
            debug!("Processing runtime events");
            process_events.call();
            debug!("[runtime] returned from __kome_runtime_process_events()");
        }
    }

    unsafe { libc::_exit(0); }
}
