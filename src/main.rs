use crate::codegen::CodegenContext;
use crate::library::LibraryManager;
use env_logger;
use inkwell::OptimizationLevel;
use inkwell::context::Context;
use std::os::raw::c_void;
use log::*;
use pest::Parser;
use pest_derive::Parser;
use std::env;
use std::fs;
use std::ffi::CString;

// Declare the C runtime functions as extern so we can obtain their addresses
// directly (without dlsym). These symbols are provided by the C sources
// compiled into the crate by build.rs.
unsafe extern "C" {
    unsafe fn __kome_runtime_start_loop();
    unsafe fn __kome_runtime_subscribe(name: *const std::os::raw::c_char, cb: *mut c_void);
    unsafe fn __kome_runtime_process_events();
    unsafe fn __kome_runtime_emit(name: *const std::os::raw::c_char);
    unsafe fn __kome_printf_i32v(fmt: *const std::os::raw::c_char, data: *const i32, len: i32) -> i32;
    unsafe fn __kome_std_keyboard_onPress(any: *mut c_void, closure: *mut c_void);
    unsafe fn __kome_std_keyboard_scan(any: *mut c_void, closure: *mut c_void);
    unsafe fn __kome_str_concat(a: *const std::os::raw::c_char, b: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
}

mod ast;
mod codegen;
pub mod library;
mod state;
mod typecheck;

#[derive(Parser)]
#[grammar = "syntax/main.pest"]
pub struct KomeParser;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() == 1 {
        println!("Usage: {} <source_file>", args[0]);
        unsafe {
            libc::fflush(std::ptr::null_mut());
            libc::_exit(1);
        }
    }

    // -d が付いているときだけログを出す（普段は静かに）
    if args.len() > 2 && args[2] == "-d" {
        unsafe {
            env::set_var("RUST_LOG", "debug");
        }
    }
    env_logger::init();

    let source_file = args[1].clone();
    if let Err(e) = fs::read_to_string(&source_file) {
        println!("Error reading file {}: {}", source_file, e);
        unsafe {
            libc::fflush(std::ptr::null_mut());
            libc::_exit(1);
        }
    }

    // パース前の生ソース
    let raw_source = match fs::read_to_string(&source_file) {
        Ok(content) => content,
        Err(_) => {
            println!("Error reading file: {}", source_file);
            unsafe {
                libc::fflush(std::ptr::null_mut());
                libc::_exit(1);
            }
        }
    };

    // パースした結果（Pair型）
    let parse = match KomeParser::parse(Rule::program, &raw_source) {
        Ok(parse) => parse,
        Err(e) => {
            println!("Parse error:\n{}", e);
            unsafe {
                libc::fflush(std::ptr::null_mut());
                libc::_exit(1);
            }
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
                Rule::EOI => { /* ignore */ }
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

    if let Err(e) = crate::typecheck::typecheck_program(&ast_state) {
        eprintln!("Type Error: {e}");
        unsafe {
            libc::fflush(std::ptr::null_mut());
            libc::_exit(1);
        }
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
        register_fn: None,
        fn_params: std::collections::HashMap::new(),
        current_return: None,
    };

    for stmt in &ast_state {
        match stmt {
            ast::Stmt::FnDecl { .. }
            | ast::Stmt::Recipe { .. }
            | ast::Stmt::Bundle { .. }
            | ast::Stmt::Import(..) => {
                // 宣言文だけをコンパイル
                codegen
                    .compile_statements(&[stmt.clone()])
                    .expect("Failed to compile declarations");
            }
            _ => {}
        }
    }

    let i32_type = context.i32_type();
    let entry_fn_type = i32_type.fn_type(&[], false);
    let entry_function = module.add_function("__kome_entry", entry_fn_type, None);
    let entry_block = context.append_basic_block(entry_function, "entry");

    builder.position_at_end(entry_block);

    for stmt in &ast_state {
        match stmt {
            ast::Stmt::Declaration { .. }
            | ast::Stmt::Assignment { .. }
            | ast::Stmt::ExprStmt(..) => {
                codegen
                    .compile_statements(&[stmt.clone()])
                    .expect("Failed to compile entry logic");
            }
            _ => {}
        }
    }

    let zero = i32_type.const_int(0, false);
    builder
        .build_return(Some(&zero))
        .expect("Failed to build entry return");

    // デバッグ
    if env::var("KOME_DEBUG_IR").ok().as_deref() == Some("1") {
        module.print_to_stderr();
    }

    // IR が壊れていると JIT が SIGTRAP で落ちることがあるので、ここで検証する
    if let Err(e) = module.verify() {
        eprintln!("IR Verify Error: {e}");
        unsafe {
            libc::fflush(std::ptr::null_mut());
            libc::_exit(1);
        }
    }

    let execution_engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .unwrap();

    if let Some(fn_val) = module.get_function("__kome_runtime_start_loop") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_runtime_start_loop as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_runtime_start_loop -> {:p}",
            __kome_runtime_start_loop as *const ()
        );
    }

    if let Some(fn_val) = module.get_function("__kome_runtime_subscribe") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_runtime_subscribe as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_runtime_subscribe -> {:p}",
            __kome_runtime_subscribe as *const ()
        );
    }

    if let Some(fn_val) = module.get_function("__kome_runtime_process_events") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_runtime_process_events as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_runtime_process_events -> {:p}",
            __kome_runtime_process_events as *const ()
        );
    }

    if let Some(fn_val) = module.get_function("__kome_runtime_emit") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_runtime_emit as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_runtime_emit -> {:p}",
            __kome_runtime_emit as *const ()
        );
    }

    // std/io 側の C ヘルパ（print/println 用）
    if let Some(fn_val) = module.get_function("__kome_printf_i32v") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_printf_i32v as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_printf_i32v -> {:p}",
            __kome_printf_i32v as *const ()
        );
    }

    // std/io.keyboard 側の C 実装
    if let Some(fn_val) = module.get_function("__kome_std_keyboard_onPress") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_std_keyboard_onPress as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_std_keyboard_onPress -> {:p}",
            __kome_std_keyboard_onPress as *const ()
        );
    }
    if let Some(fn_val) = module.get_function("__kome_std_keyboard_scan") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_std_keyboard_scan as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_std_keyboard_scan -> {:p}",
            __kome_std_keyboard_scan as *const ()
        );
    }

    // std/string 側の C 実装
    if let Some(fn_val) = module.get_function("__kome_str_concat") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, __kome_str_concat as *const () as usize);
        debug!(
            "[jit-map] mapped __kome_str_concat -> {:p}",
            __kome_str_concat as *const ()
        );
    }

    unsafe {
        if let Ok(entry_fn) =
            execution_engine.get_function::<unsafe extern "C" fn() -> i32>("__kome_entry")
        {
            debug!("[runtime] calling __kome_entry()");
            entry_fn.call();
            debug!("[runtime] returned from __kome_entry()");
        } else {
            println!("Runtime Error: Entry function is not defined.");
        }
    }

    // std/bundle などが生成した subscribe 登録を実行
    // まずはランタイム関数が呼べるかの自己診断（JIT とは無関係）
    if std::env::var("KOME_SELFTEST_SUBSCRIBE").ok().as_deref() == Some("1") {
        let s = CString::new("selftest").unwrap();
        unsafe {
            __kome_runtime_subscribe(s.as_ptr(), std::ptr::null_mut());
        }
    }
    unsafe {
        if let Ok(register_fn) =
            execution_engine.get_function::<unsafe extern "C" fn()>("__kome_register")
        {
            debug!("[runtime] calling __kome_register()");
            register_fn.call();
            debug!("[runtime] returned from __kome_register()");
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
        if let Ok(process_events) =
            execution_engine.get_function::<unsafe extern "C" fn()>("__kome_runtime_process_events")
        {
            debug!("[runtime] calling __kome_runtime_process_events()");
            debug!("Processing runtime events");
            process_events.call();
            debug!("[runtime] returned from __kome_runtime_process_events()");
        }
    }

    unsafe {
        // _exit は stdio を flush しないので、出力が消えることがある
        libc::fflush(std::ptr::null_mut());
        libc::_exit(0);
    }
}
