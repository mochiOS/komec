use std::ffi::CStr;
use std::os::raw::c_char;
use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;

type RecipeFn = unsafe extern "C" fn();

lazy_static! {
    static ref RECIPIENT_REGISTRY: Mutex<HashMap<String, Vec<RecipeFn>>> = Mutex::new(HashMap::new());
}

// NOTE: subscription is implemented in the C runtime (std/runtime.c).
// The Rust-side registry was removed to avoid duplicate symbol when linking
// the C runtime. If you want Rust to own the registry instead, remove the
// C implementation in `std/runtime.c` and reintroduce the function below.

/// stateへの代入命令の直後に呼び出される通知関数
///
/// LLVMようなので叩かないでください
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_runtime_notify_change(var_name_ptr: *const c_char) {
    unsafe {
        if var_name_ptr.is_null() { return; }

        if let Ok(c_str) = CStr::from_ptr(var_name_ptr).to_str() {
            let registry = RECIPIENT_REGISTRY.lock().unwrap();

            if let Some(recipes) = registry.get(c_str) {
                for recipe in recipes {
                    recipe();
                }
            }
        }
    }
}
