//! Ahead-of-time backend for producing standalone Kome executables.

use cranelift::codegen::isa::OwnedTargetIsa;
use cranelift::codegen::settings;
use cranelift::prelude::*;
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use kome_ast::declarations::Module as KomeModule;
use kome_codegen::compile::{analyze_module, compile_module, mangled_name};
use kome_codegen::{CodegenError, CodegenResult};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicU32;

static TEMPORARY_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Compiles `module_ast` into a standalone executable at `output`.
pub fn build_executable(module_ast: &KomeModule, output: &Path) -> CodegenResult<()> {
    let info = analyze_module(module_ast)?;
    info.entry_signature("main")?;

    let mut module = ObjectModule::new(
        ObjectBuilder::new(
            host_isa()?,
            "kome.o",
            cranelift_module::default_libcall_names(),
        )
        .map_err(|error| CodegenError::new(error.to_string(), None))?,
    );

    compile_module(&info, &mut module)?;
    define_c_main(&mut module)?;

    let object_bytes = module
        .finish()
        .emit()
        .map_err(|error| CodegenError::new(format!("failed to emit object file: {error}"), None))?;
    let runtime_library = locate_runtime_library()?;
    let temporary_object = std::env::temp_dir().join(format!(
        "kome-build-{}-{}.o",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));

    write_file(&temporary_object, &object_bytes)?;
    let result = link(&temporary_object, &runtime_library, output);
    let _ = std::fs::remove_file(&temporary_object);
    result?;
    make_executable(output)?;

    Ok(())
}

/// Selects the ISA for AOT output.
///
/// This is deliberately private: it currently targets the host and will be
/// replaced by explicit target selection when cross-compilation is supported.
fn host_isa() -> CodegenResult<OwnedTargetIsa> {
    let flags = settings::Flags::new(settings::builder());

    cranelift_native::builder()
        .map_err(|message| CodegenError::new(message, None))?
        .finish(flags)
        .map_err(|error| CodegenError::new(error.to_string(), None))
}

/// Emits `int main(void) { kome_main(); return 0; }`.
fn define_c_main(module: &mut ObjectModule) -> CodegenResult<()> {
    let kome_main = module
        .get_name(&mangled_name("main"))
        .and_then(|id| match id {
            cranelift_module::FuncOrDataId::Func(func_id) => Some(func_id),
            _ => None,
        })
        .ok_or_else(|| {
            CodegenError::new("internal error: entry function was not declared", None)
        })?;

    let mut signature = module.make_signature();
    signature.returns.push(AbiParam::new(types::I32));
    let main_id = module
        .declare_function("main", Linkage::Export, &signature)
        .map_err(|error| CodegenError::new(error.to_string(), None))?;

    let mut context = module.make_context();
    context.func.signature = signature;
    context.func.name = cranelift::codegen::ir::UserFuncName::user(0, main_id.as_u32());
    let mut function_builder_context = FunctionBuilderContext::new();

    {
        let mut builder = FunctionBuilder::new(&mut context.func, &mut function_builder_context);
        let entry_block = builder.create_block();
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);
        let func_ref = module.declare_func_in_func(kome_main, builder.func);
        builder.ins().call(func_ref, &[]);
        let status = builder.ins().iconst(types::I32, 0);
        builder.ins().return_(&[status]);
        builder.finalize(module.target_config());
    }

    module
        .define_function(main_id, &mut context)
        .map_err(|error| CodegenError::new(format!("failed to compile `main`: {error}"), None))?;
    Ok(())
}

/// Locates `libkome_native_rt.a` from `KOME_NATIVE_RT_LIB` or beside the executable.
fn locate_runtime_library() -> CodegenResult<PathBuf> {
    const LIBRARY: &str = "libkome_native_rt.a";

    if let Some(path) = std::env::var_os("KOME_NATIVE_RT_LIB")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        for candidate in [
            directory.join(LIBRARY),
            directory.join("../").join(LIBRARY),
            directory.join("../lib").join(LIBRARY),
        ] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(CodegenError::new(
        format!(
            "`{LIBRARY}` was not found; build it with \
             `cargo build -p kome_native_rt` or set KOME_NATIVE_RT_LIB"
        ),
        None,
    ))
}

fn link(object: &Path, runtime_library: &Path, output: &Path) -> CodegenResult<()> {
    let invocation = Command::new("cc")
        .arg(object)
        .arg("-o")
        .arg(output)
        .arg(runtime_library)
        .args(["-lpthread", "-ldl", "-lm"])
        .output()
        .map_err(|error| CodegenError::new(format!("failed to run `cc`: {error}"), None))?;

    if !invocation.status.success() {
        return Err(CodegenError::new(
            format!(
                "linking failed:\n{}{}",
                String::from_utf8_lossy(&invocation.stdout),
                String::from_utf8_lossy(&invocation.stderr),
            ),
            None,
        ));
    }
    Ok(())
}

fn make_executable(path: &Path) -> CodegenResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| CodegenError::new(error.to_string(), None))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| CodegenError::new(error.to_string(), None))
}

fn write_file(path: &Path, bytes: &[u8]) -> CodegenResult<()> {
    let mut file = std::fs::File::create(path).map_err(|error| {
        CodegenError::new(format!("cannot create `{}`: {error}", path.display()), None)
    })?;
    file.write_all(bytes).map_err(|error| {
        CodegenError::new(format!("cannot write `{}`: {error}", path.display()), None)
    })
}
