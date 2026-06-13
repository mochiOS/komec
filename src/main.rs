use crate::codegen::CodegenContext;
use crate::library::LibraryManager;
use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::targets::{InitializationConfig, Target};
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
    unsafe fn LLVMAddSymbol(symbolName: *const std::os::raw::c_char, symbolValue: *mut c_void);
    unsafe fn __kome_runtime_start_loop();
    unsafe fn __kome_runtime_subscribe(name: *const std::os::raw::c_char, cb: *mut c_void);
    unsafe fn __kome_runtime_process_events();
    unsafe fn __kome_runtime_emit(name: *const std::os::raw::c_char);
    unsafe fn __kome_runtime_set_app(app: *mut c_void);
    unsafe fn __kome_runtime_get_app() -> *mut c_void;
    unsafe fn __kome_printf_i32v(fmt: *const std::os::raw::c_char, data: *const i32, len: i32) -> i32;
    unsafe fn __kome_fs_list(path: *const std::os::raw::c_char) -> *mut c_void;
    unsafe fn __kome_fs_read(path: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
    unsafe fn __kome_process_exec(command: *const std::os::raw::c_char) -> i32;
    unsafe fn __kome_process_exit(code: i32);
    unsafe fn __kome_toml_parse(text: *const std::os::raw::c_char) -> *mut c_void;
    unsafe fn __kome_value_map(list_ptr: *mut c_void, closure_ptr: *mut c_void) -> *mut c_void;
    unsafe fn __kome_value_filter(list_ptr: *mut c_void, closure_ptr: *mut c_void) -> *mut c_void;
    unsafe fn __kome_value_len(list_ptr: *mut c_void) -> i32;
    unsafe fn __kome_value_index(list_ptr: *mut c_void, index: i32) -> *mut c_void;
    unsafe fn __kome_value_name(path_ptr: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
    unsafe fn __kome_value_isDir(path_ptr: *const std::os::raw::c_char) -> bool;
    unsafe fn __kome_value_hasSuffix(value_ptr: *const std::os::raw::c_char, suffix_ptr: *const std::os::raw::c_char) -> bool;
    unsafe fn __kome_value_trimSuffix(value_ptr: *const std::os::raw::c_char, suffix_ptr: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
    unsafe fn __kome_value_entry(toml_ptr: *mut c_void) -> *mut std::os::raw::c_char;
    unsafe fn __kome_value_icon(toml_ptr: *mut c_void) -> *mut std::os::raw::c_char;
    unsafe fn __kome_value_image(base_ptr: *const std::os::raw::c_char, ...) -> *mut std::os::raw::c_char;
    unsafe fn __kome_value_selected(base_ptr: *const std::os::raw::c_char, ...) -> *mut std::os::raw::c_char;
    unsafe fn __kome_std_keyboard_onPress(any: *mut c_void, closure: *mut c_void);
    unsafe fn __kome_std_keyboard_scan(any: *mut c_void, closure: *mut c_void);
    unsafe fn __kome_str_concat(a: *const std::os::raw::c_char, b: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
    unsafe fn concat(a: *const std::os::raw::c_char, b: *const std::os::raw::c_char) -> *mut std::os::raw::c_char;
}

unsafe fn register_llvm_symbol(name: &str, ptr: *const ()) {
    let c_name = CString::new(name).unwrap();
    let leaked_name = c_name.into_raw();
    unsafe {
        LLVMAddSymbol(leaked_name, ptr as *mut c_void);
    }
}

unsafe fn register_builtin_symbols() {
    unsafe {
        register_llvm_symbol("__kome_runtime_start_loop", __kome_runtime_start_loop as *const ());
        register_llvm_symbol("__kome_runtime_subscribe", __kome_runtime_subscribe as *const ());
        register_llvm_symbol("__kome_runtime_process_events", __kome_runtime_process_events as *const ());
        register_llvm_symbol("__kome_runtime_emit", __kome_runtime_emit as *const ());
        register_llvm_symbol("__kome_runtime_set_app", __kome_runtime_set_app as *const ());
        register_llvm_symbol("__kome_runtime_get_app", __kome_runtime_get_app as *const ());
        register_llvm_symbol("__kome_printf_i32v", __kome_printf_i32v as *const ());
        register_llvm_symbol("__kome_fs_list", __kome_fs_list as *const ());
        register_llvm_symbol("__kome_fs_read", __kome_fs_read as *const ());
        register_llvm_symbol("__kome_process_exec", __kome_process_exec as *const ());
        register_llvm_symbol("__kome_process_exit", __kome_process_exit as *const ());
        register_llvm_symbol("__kome_toml_parse", __kome_toml_parse as *const ());
        register_llvm_symbol("__kome_value_map", __kome_value_map as *const ());
        register_llvm_symbol("__kome_value_filter", __kome_value_filter as *const ());
        register_llvm_symbol("__kome_value_len", __kome_value_len as *const ());
        register_llvm_symbol("__kome_value_index", __kome_value_index as *const ());
        register_llvm_symbol("__kome_value_name", __kome_value_name as *const ());
        register_llvm_symbol("__kome_value_isDir", __kome_value_isDir as *const ());
        register_llvm_symbol("__kome_value_hasSuffix", __kome_value_hasSuffix as *const ());
        register_llvm_symbol("__kome_value_trimSuffix", __kome_value_trimSuffix as *const ());
        register_llvm_symbol("__kome_value_entry", __kome_value_entry as *const ());
        register_llvm_symbol("__kome_value_icon", __kome_value_icon as *const ());
        register_llvm_symbol("__kome_value_image", __kome_value_image as *const ());
        register_llvm_symbol("__kome_value_selected", __kome_value_selected as *const ());
        register_llvm_symbol("__kome_std_keyboard_onPress", __kome_std_keyboard_onPress as *const ());
        register_llvm_symbol("__kome_std_keyboard_scan", __kome_std_keyboard_scan as *const ());
        register_llvm_symbol("__kome_str_concat", __kome_str_concat as *const ());
        register_llvm_symbol("concat", concat as *const ());
    }
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

    let component_templates: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

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

    // JIT のターゲット初期化を先に済ませる
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    unsafe {
        register_builtin_symbols();
    }

    let context = Context::create();
    let module = context.create_module("main");
    let builder = context.create_builder();
    let library_manager = LibraryManager::new();
    let mut codegen = CodegenContext {
        context: &context,
        builder: &builder,
        module: &module,
        variables: std::collections::HashMap::new(),
        library_manager: &library_manager,
        current_dir: std::path::PathBuf::new(),
        current_module_prefix: None,
        allowed_externs: std::collections::HashSet::new(),
        register_fn: None,
        fn_params: std::collections::HashMap::new(),
        current_return: None,
        default_scopes: Vec::new(),
        component_templates,
        pending_subscriptions: Vec::new(),
        in_bundle_prelude: false,
        loaded_modules: std::collections::HashSet::new(),
        loading_modules: std::collections::HashSet::new(),
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
                    .compile_statements(std::slice::from_ref(stmt))
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
    if let Some(fn_val) = module.get_function("concat") {
        let gv = fn_val.as_global_value();
        execution_engine.add_global_mapping(&gv, concat as *const () as usize);
        debug!("[jit-map] mapped concat -> {:p}", concat as *const ());
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

    // レシピ購読はホスト側で確定させる
    for (dep_var, recipe_fn_name) in &codegen.pending_subscriptions {
        if let Ok(addr) = execution_engine.get_function_address(recipe_fn_name) {
            let name = CString::new(dep_var.as_str()).unwrap();
            unsafe {
                __kome_runtime_subscribe(name.as_ptr(), addr as *mut c_void);
            }
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
        libc::fflush(std::ptr::null_mut());
        libc::_exit(0);
    }
}