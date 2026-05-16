use std::ffi::CStr;
use std::os::raw::c_char;
use std::collections::HashMap;
use std::sync::Mutex;
use lazy_static::lazy_static;

type RecipeFn = unsafe extern "C" fn();

lazy_static! {
    static ref RECIPIENT_REGISTRY: Mutex<HashMap<String, Vec<RecipeFn>>> = Mutex::new(HashMap::new());
}

/// 変数とレシピの依存関係を動的に登録する関数
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_runtime_subscribe(var_name_ptr: *const c_char, recipe: RecipeFn) {
    unsafe {
        if var_name_ptr.is_null() { return; }
        if let Ok(c_str) = CStr::from_ptr(var_name_ptr).to_str() {
            let mut registry = RECIPIENT_REGISTRY.lock().unwrap();
            registry.entry(c_str.to_string()).or_insert_with(Vec::new).push(recipe);
        }
    }
}

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
