//! JIT backend for executing Kome modules in-process.

use cranelift::codegen::isa::OwnedTargetIsa;
use cranelift::codegen::settings;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::Module;
use kome_ast::declarations::Module as KomeModule;
use kome_codegen::compile::{analyze_module, compile_module, mangled_name};
use kome_codegen::{CodegenError, CodegenResult};
use kome_native_rt::__kome_native_call;

/// Compiles `module_ast` and runs `entry` in the current process.
///
/// Native calls dispatch through [`kome_native_rt`]; install a custom
/// registry with [`kome_native_rt::set_thread_registry`] beforehand if the
/// built-ins are not enough.
pub fn execute(module_ast: &KomeModule, entry: &str) -> CodegenResult<()> {
    let info = analyze_module(module_ast)?;
    info.entry_signature(entry)?;

    let mut builder =
        JITBuilder::with_isa(native_isa()?, cranelift_module::default_libcall_names());

    register_runtime_symbols(&mut builder);

    let mut module = JITModule::new(builder);

    compile_module(&info, &mut module)?;

    module
        .finalize_definitions()
        .map_err(|error| CodegenError::new(error.to_string(), None))?;

    let func_id = entry_func_id(&module, entry)?;
    let code_pointer = module.get_finalized_function(func_id);
    let function: extern "C" fn() = unsafe { std::mem::transmute(code_pointer) };
    function();

    Ok(())
}

/// Builds the ISA for the host process that owns this JIT instance.
fn native_isa() -> CodegenResult<OwnedTargetIsa> {
    let flags = settings::Flags::new(settings::builder());

    cranelift_native::builder()
        .map_err(|message| CodegenError::new(message, None))?
        .finish(flags)
        .map_err(|error| CodegenError::new(error.to_string(), None))
}

/// Registers the native runtime symbols so the JIT can resolve them without
/// relying on dynamic symbol lookup.
fn register_runtime_symbols(builder: &mut JITBuilder) {
    type NativeCall = unsafe extern "C" fn(*const u8, usize, *const kome_abi::Slot, i64) -> i64;

    let symbols: [(&str, *const u8); 1] = [(
        "__kome_native_call",
        __kome_native_call as NativeCall as usize as *const u8,
    )];

    for (name, pointer) in symbols {
        builder.symbol(name, pointer);
    }
}

fn entry_func_id(module: &JITModule, entry: &str) -> CodegenResult<cranelift_module::FuncId> {
    match module.get_name(&mangled_name(entry)) {
        Some(cranelift_module::FuncOrDataId::Func(func_id)) => Ok(func_id),
        Some(_) => Err(CodegenError::new(
            format!("internal error: `{entry}` did not compile to a function"),
            None,
        )),
        None => Err(CodegenError::new(
            format!("internal error: `{entry}` was not declared"),
            None,
        )),
    }
}
