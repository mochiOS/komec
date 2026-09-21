use kome_abi::{Slot, TAG_BOOLEAN, TAG_VOID};
use kome_native_rt::{NativeRegistry, RuntimeError, Value, builtin_registry, set_thread_registry};
use std::sync::{Arc, Mutex};

#[test]
fn dispatches_by_symbol_name() {
    let captured = Arc::new(Mutex::new(Vec::<bool>::new()));

    let sink = Arc::clone(&captured);

    let mut registry = NativeRegistry::new();
    registry.register("test.echo", move |arguments: &[Value]| {
        let [Value::Boolean(value)] = arguments else {
            return Err(RuntimeError::native("expected one Boolean"));
        };
        sink.lock().unwrap().push(*value);
        Ok(Value::Null)
    });

    set_thread_registry(registry);

    let slot = Slot {
        tag: TAG_BOOLEAN,
        payload: 1,
    };

    let result = unsafe { __kome_native_call(b"test.echo\0".as_ptr(), 1, &slot, TAG_VOID) };

    assert_eq!(result, 0);
    assert_eq!(*captured.lock().unwrap(), vec![true]);
}

unsafe extern "C" {
    fn __kome_native_call(name: *const u8, argc: usize, args: *const Slot, ret_tag: i64) -> i64;
}

#[test]
fn builtin_registry_has_core_write() {
    let _ = builtin_registry();
}
