//! Native runtime for compiled Kome programs.
//!
//! Exposes the C ABI symbols that generated code calls:
//!
//! - `__kome_native_call(name, argc, args, ret_tag)` dispatches `@native`
//!   calls through a [`NativeRegistry`] by function name.
//!
//! The same symbols serve the JIT backend (linked into the compiler process)
//! and AOT builds (archived into `libkome_native_rt.a` and linked into the
//! produced executable).

use kome_abi::{Slot, TAG_BOOLEAN, TAG_NULL, TAG_NUMBER, TAG_VOID};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::fmt;
use std::io::Write;
use std::sync::{Arc, OnceLock};

/// A value exchanged between Kome code and a registered native function.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    Boolean(bool),
    Null,
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Null => formatter.write_str("null"),
        }
    }
}

/// An error returned by a registered native function.
#[derive(Debug)]
pub enum RuntimeError {
    NativeFunctionNotFound { name: String },
    Native { message: String },
}

impl RuntimeError {
    pub fn native(message: impl Into<String>) -> Self {
        Self::Native {
            message: message.into(),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeFunctionNotFound { name } => {
                write!(formatter, "native function `{name}` is not registered")
            }
            Self::Native { message } => write!(formatter, "native function failed: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

type NativeFunction = dyn Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static;

/// Registry mapping `@native` symbols to host-provided Rust functions.
#[derive(Default)]
pub struct NativeRegistry {
    functions: HashMap<String, Box<NativeFunction>>,
}

impl NativeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, name: impl Into<String>, function: F)
    where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        self.functions.insert(name.into(), Box::new(function));
    }

    pub fn call(&self, name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
        let function =
            self.functions
                .get(name)
                .ok_or_else(|| RuntimeError::NativeFunctionNotFound {
                    name: name.to_string(),
                })?;

        function(arguments)
    }
}

/// Builds the built-in native registry used by compiled programs.
pub fn builtin_registry() -> NativeRegistry {
    let mut registry = NativeRegistry::new();
    registry.register("core.write", write);
    registry.register("core.write_line", write_line);
    registry
}

/// Replaces the registry used by `@native` calls on the current thread.
///
/// The default registry is untouched, so other threads (and AOT binaries,
/// which never call this) keep using the built-ins.
pub fn set_thread_registry(registry: NativeRegistry) {
    REGISTRY_OVERRIDE.with(|slot| {
        *slot.borrow_mut() = Some(Arc::new(registry));
    });
}

/// Clears any thread-local registry override.
pub fn clear_thread_registry() {
    REGISTRY_OVERRIDE.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// # Safety
///
/// `name` must point to a NUL-terminated string and `args` must point to
/// `argc` contiguous [`Slot`] values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_native_call(
    name: *const u8,
    argc: usize,
    args: *const Slot,
    ret_tag: i64,
) -> i64 {
    match unsafe { dispatch(name, argc, args, ret_tag) } {
        Ok(payload) => payload,

        Err(message) => fail(&message),
    }
}

static DEFAULT_REGISTRY: OnceLock<Arc<NativeRegistry>> = OnceLock::new();

thread_local! {
    static REGISTRY_OVERRIDE: RefCell<Option<Arc<NativeRegistry>>> = const { RefCell::new(None) };
}

fn registry() -> Arc<NativeRegistry> {
    REGISTRY_OVERRIDE.with(|slot| {
        if let Some(registry) = slot.borrow().clone() {
            return registry;
        }

        DEFAULT_REGISTRY
            .get_or_init(|| Arc::new(builtin_registry()))
            .clone()
    })
}

unsafe fn dispatch(
    name: *const u8,
    argc: usize,
    args: *const Slot,
    ret_tag: i64,
) -> Result<i64, String> {
    let symbol = unsafe { CStr::from_ptr(name as *const c_char) }
        .to_str()
        .map_err(|error| format!("native symbol name is not valid UTF-8: {error}"))?;

    let slots = unsafe { std::slice::from_raw_parts(args, argc) };

    let mut arguments = Vec::with_capacity(argc);
    for slot in slots {
        arguments.push(slot_to_value(slot)?);
    }

    let result = registry()
        .call(symbol, &arguments)
        .map_err(|error: RuntimeError| error.to_string())?;

    payload_for_return(&result, ret_tag)
}

fn slot_to_value(slot: &Slot) -> Result<Value, String> {
    match slot.tag {
        TAG_NUMBER => Ok(Value::Number(f64::from_bits(slot.payload as u64))),
        TAG_BOOLEAN => Ok(Value::Boolean(slot.payload != 0)),
        TAG_NULL => Ok(Value::Null),
        _ => Err(format!(
            "native call received unknown argument tag {}",
            slot.tag
        )),
    }
}

fn payload_for_return(value: &Value, ret_tag: i64) -> Result<i64, String> {
    if ret_tag == TAG_VOID {
        return Ok(0);
    }

    if value_tag(value) != ret_tag {
        return Err(format!(
            "native function returned {}, but the caller expected tag {ret_tag}",
            value_type_name(value),
        ));
    }

    Ok(scalar_payload(value))
}

fn scalar_payload(value: &Value) -> i64 {
    match value {
        Value::Number(number) => number.to_bits() as i64,
        Value::Boolean(flag) => i64::from(*flag),
        Value::Null => 0,
    }
}

fn value_tag(value: &Value) -> i64 {
    match value {
        Value::Number(_) => TAG_NUMBER,
        Value::Boolean(_) => TAG_BOOLEAN,
        Value::Null => TAG_NULL,
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "Number",
        Value::Boolean(_) => "Boolean",
        Value::Null => "Null",
    }
}

fn fail(message: &str) -> i64 {
    let _ = writeln!(std::io::stderr(), "runtime error: {message}");
    std::process::exit(1);
}

fn write(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [value] = arguments else {
        return Err(RuntimeError::native(
            "core.write expects exactly one argument",
        ));
    };

    print!("{value}");

    std::io::stdout()
        .flush()
        .map_err(|error| RuntimeError::native(format!("failed to flush stdout: {error}")))?;

    Ok(Value::Null)
}

fn write_line(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [value] = arguments else {
        return Err(RuntimeError::native(
            "core.write_line expects exactly one argument",
        ));
    };

    println!("{value}");

    Ok(Value::Null)
}
