//! Native runtime for compiled Kome programs.
//!
//! Exposes the C ABI symbols that generated code calls:
//!
//! - `__kome_native_call(name, argc, args, ret_tag)` dispatches `@native`
//!   calls through a [`NativeRegistry`] by function name.
//! - `__kome_task_*` owns task state, result storage, waiting, reference
//!   counting, and the cancellation/failure states used by generated code.
//!
//! The same symbols serve the JIT backend (linked into the compiler process)
//! and AOT builds (archived into `libkome_native_rt.a` and linked into the
//! produced executable).

pub mod io;
pub mod list;
pub mod number;
mod reactor;
pub mod socket;
pub mod string;
pub mod struct_value;
pub mod task;

use crate::number::Number;
use crate::socket::KomeSocket;
use crate::string::KomeString;
use kome_abi::{
    Slot, TAG_BOOLEAN, TAG_F32, TAG_F64, TAG_NULL, TAG_NUMBER, TAG_SIGNED_INTEGER, TAG_SOCKET,
    TAG_STRING, TAG_UNSIGNED_INTEGER, TAG_VOID,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::fmt;
use std::io::Write;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// A value exchanged between Kome code and a registered native function.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(Number),
    String(KomeString),
    Socket(KomeSocket),
    Boolean(bool),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Float(f64),
    Null,
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "{value}"),
            Self::Socket(value) => write!(formatter, "{value}"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::SignedInteger(value) => write!(formatter, "{value}"),
            Self::UnsignedInteger(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
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
    /// Creates an error reported by a native runtime function.
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
    /// Creates an empty native-function registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers or replaces a native function under `name`.
    pub fn register<F>(&mut self, name: impl Into<String>, function: F)
    where
        F: Fn(&[Value]) -> Result<Value, RuntimeError> + Send + Sync + 'static,
    {
        self.functions.insert(name.into(), Box::new(function));
    }

    /// Calls a registered native function.
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
    registry.register("core.write_error", write_error);
    registry.register("core.write_error_line", write_error_line);
    registry.register("io.sleep", io_sleep);
    registry.register("io.socket_connect", io_socket_connect);
    registry.register("io.socket_bind", io_socket_bind);
    registry.register("io.socket_accept", io_socket_accept);
    registry.register("io.socket_read", io_socket_read);
    registry.register("io.socket_write", io_socket_write);
    registry.register("io.socket_close", io_socket_close);
    registry.register("time.monotonic_milliseconds", monotonic_milliseconds);
    registry
}

fn integer_argument(value: &Value, name: &str) -> Result<i64, RuntimeError> {
    let Value::Number(value) = value else {
        return Err(RuntimeError::native(format!("{name} must be a Number")));
    };
    value
        .to_i64()
        .ok_or_else(|| RuntimeError::native(format!("{name} must be an integer in i64 range")))
}

fn monotonic_milliseconds(arguments: &[Value]) -> Result<Value, RuntimeError> {
    if !arguments.is_empty() {
        return Err(RuntimeError::native(
            "time.monotonic_milliseconds expects no arguments",
        ));
    }

    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_millis();
    let elapsed = i64::try_from(elapsed).unwrap_or(i64::MAX);

    Ok(Value::Number(Number::from_i64(elapsed)))
}

fn io_sleep(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [milliseconds] = arguments else {
        return Err(RuntimeError::native("io.sleep expects one argument"));
    };
    let milliseconds = integer_argument(milliseconds, "milliseconds")?;
    if milliseconds < 0 {
        return Err(RuntimeError::native("milliseconds must not be negative"));
    }
    io::sleep(milliseconds as u64);
    Ok(Value::Null)
}

fn io_socket_connect(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [Value::String(host), port] = arguments else {
        return Err(RuntimeError::native(
            "io.socket_connect expects a String host and Number port",
        ));
    };
    let port = integer_argument(port, "socket port")?;
    let port = u16::try_from(port)
        .map_err(|_| RuntimeError::native("socket port must be between 0 and 65535"))?;
    io::socket_connect(host.as_str(), port)
        .map(Value::Socket)
        .map_err(|error| RuntimeError::native(format!("socket connect failed: {error}")))
}

fn io_socket_bind(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [Value::String(host), port] = arguments else {
        return Err(RuntimeError::native(
            "io.socket_bind expects a String host and Number port",
        ));
    };
    let port = integer_argument(port, "listener port")?;
    let port = u16::try_from(port)
        .map_err(|_| RuntimeError::native("listener port must be between 0 and 65535"))?;
    io::socket_bind(host.as_str(), port)
        .map(Value::Socket)
        .map_err(|error| RuntimeError::native(format!("socket bind failed: {error}")))
}

fn io_socket_accept(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [Value::Socket(listener)] = arguments else {
        return Err(RuntimeError::native(
            "io.socket_accept expects a TcpListener",
        ));
    };
    io::managed_socket_accept(listener)
        .map(Value::Socket)
        .map_err(|error| RuntimeError::native(format!("socket accept failed: {error}")))
}

fn io_socket_read(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [socket, maximum] = arguments else {
        return Err(RuntimeError::native("io.socket_read expects two arguments"));
    };
    let maximum = integer_argument(maximum, "maximum read size")?;
    let maximum = usize::try_from(maximum)
        .map_err(|_| RuntimeError::native("maximum read size must not be negative"))?;
    let result = match socket {
        Value::Socket(socket) => io::managed_socket_read(socket, maximum),
        legacy_fd => {
            let fd = integer_argument(legacy_fd, "socket fd")?;
            let fd =
                i32::try_from(fd).map_err(|_| RuntimeError::native("socket fd is out of range"))?;
            io::socket_read(fd, maximum)
        }
    };
    result
        .map(Value::String)
        .map_err(|error| RuntimeError::native(format!("socket read failed: {error}")))
}

fn io_socket_write(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [socket, Value::String(value)] = arguments else {
        return Err(RuntimeError::native(
            "io.socket_write expects a socket fd and String",
        ));
    };
    let result = match socket {
        Value::Socket(socket) => io::managed_socket_write(socket, value),
        legacy_fd => {
            let fd = integer_argument(legacy_fd, "socket fd")?;
            let fd =
                i32::try_from(fd).map_err(|_| RuntimeError::native("socket fd is out of range"))?;
            io::socket_write(fd, value)
        }
    };
    result
        .map(|written| Value::Number(Number::from_i64(written as i64)))
        .map_err(|error| RuntimeError::native(format!("socket write failed: {error}")))
}

fn io_socket_close(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [Value::Socket(socket)] = arguments else {
        return Err(RuntimeError::native("io.socket_close expects a Socket"));
    };
    socket
        .close()
        .map(|()| Value::Null)
        .map_err(|error| RuntimeError::native(format!("socket close failed: {error}")))
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
        TAG_NUMBER => Ok(Value::Number(unsafe {
            Number::from_raw_retain(slot.payload as u64)
        })),
        TAG_STRING => Ok(Value::String(unsafe {
            KomeString::from_raw_retain(slot.payload as u64)
        })),
        TAG_SOCKET => Ok(Value::Socket(unsafe {
            KomeSocket::from_raw_retain(slot.payload as u64)
        })),
        TAG_BOOLEAN => Ok(Value::Boolean(slot.payload != 0)),
        TAG_SIGNED_INTEGER => Ok(Value::SignedInteger(slot.payload)),
        TAG_UNSIGNED_INTEGER => Ok(Value::UnsignedInteger(slot.payload as u64)),
        TAG_F32 => Ok(Value::Float(f32::from_bits(slot.payload as u32) as f64)),
        TAG_F64 => Ok(Value::Float(f64::from_bits(slot.payload as u64))),
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

    let fixed_payload = match (ret_tag, value) {
        (TAG_SIGNED_INTEGER, Value::SignedInteger(value)) => Some(*value),
        (TAG_UNSIGNED_INTEGER, Value::UnsignedInteger(value)) => Some(*value as i64),
        (TAG_F32, Value::Float(value)) => Some((*value as f32).to_bits() as i64),
        (TAG_F64, Value::Float(value)) => Some(value.to_bits() as i64),
        _ => None,
    };
    if let Some(payload) = fixed_payload {
        return Ok(payload);
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
        Value::Number(number) => number.clone().into_raw() as i64,
        Value::String(string) => string.clone().into_raw() as i64,
        Value::Socket(socket) => socket.clone().into_raw() as i64,
        Value::Boolean(flag) => i64::from(*flag),
        Value::SignedInteger(value) => *value,
        Value::UnsignedInteger(value) => *value as i64,
        Value::Float(value) => value.to_bits() as i64,
        Value::Null => 0,
    }
}

fn value_tag(value: &Value) -> i64 {
    match value {
        Value::Number(_) => TAG_NUMBER,
        Value::String(_) => TAG_STRING,
        Value::Socket(_) => TAG_SOCKET,
        Value::Boolean(_) => TAG_BOOLEAN,
        Value::SignedInteger(_) => TAG_SIGNED_INTEGER,
        Value::UnsignedInteger(_) => TAG_UNSIGNED_INTEGER,
        Value::Float(_) => TAG_F64,
        Value::Null => TAG_NULL,
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "Number",
        Value::String(_) => "String",
        Value::Socket(_) => "Socket",
        Value::Boolean(_) => "bool",
        Value::SignedInteger(_) => "signed integer",
        Value::UnsignedInteger(_) => "unsigned integer",
        Value::Float(_) => "float",
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

fn write_error(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [value] = arguments else {
        return Err(RuntimeError::native(
            "core.write_error expects exactly one argument",
        ));
    };

    eprint!("{value}");

    std::io::stderr()
        .flush()
        .map_err(|error| RuntimeError::native(format!("failed to flush stderr: {error}")))?;

    Ok(Value::Null)
}

fn write_error_line(arguments: &[Value]) -> Result<Value, RuntimeError> {
    let [value] = arguments else {
        return Err(RuntimeError::native(
            "core.write_error_line expects exactly one argument",
        ));
    };

    eprintln!("{value}");

    Ok(Value::Null)
}
