//! Binary ABI shared between compiled Kome code and the native runtime.
//!
//! Compiled Kome code is statically typed: `Number` maps to a Cranelift
//! `F64`, `Boolean` and `Null` map to `I8`, and `String` maps to an `I64`
//! holding a pointer to a runtime-allocated `{ ptr, len }` box.
//!
//! Calls into the native runtime marshal arguments through [`Slot`] values:
//! fixed 16-byte `{ tag, payload }` records, so a single dispatcher symbol
//! serves every `@native` declaration in both the JIT and AOT backends.

use crate::Value;

/// Payload holds the bit pattern of an `f64`.
pub const TAG_NUMBER: i64 = 0;

/// Payload holds `0` or `1`.
pub const TAG_BOOLEAN: i64 = 1;

/// Payload holds a pointer to a runtime string box.
pub const TAG_STRING: i64 = 2;

/// Payload is unused and always `0`.
pub const TAG_NULL: i64 = 3;

/// Used as a return-type tag when the native result is discarded.
pub const TAG_VOID: i64 = 4;

/// One marshalled argument or return value exchanged with the runtime.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub tag: i64,
    pub payload: i64,
}

impl Slot {
    /// Builds a slot for a scalar value (anything except `String`).
    pub fn scalar(tag: i64, payload: i64) -> Self {
        Self { tag, payload }
    }

    /// Converts a slot carrying a scalar value into a [`Value`].
    ///
    /// Returns `None` when the tag is unknown or the slot carries a string
    /// box, which only the native runtime can dereference.
    pub fn to_scalar_value(&self) -> Option<Value> {
        match self.tag {
            TAG_NUMBER => Some(Value::Number(f64::from_bits(self.payload as u64))),
            TAG_BOOLEAN => Some(Value::Boolean(self.payload != 0)),
            TAG_NULL => Some(Value::Null),
            _ => None,
        }
    }
}

/// Converts a scalar [`Value`] into a slot payload.
///
/// Returns `None` for `Value::String`, which must be boxed by the native
/// runtime before it can cross the boundary.
pub fn scalar_payload(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => Some(number.to_bits() as i64),
        Value::Boolean(flag) => Some(i64::from(*flag)),
        Value::Null => Some(0),
        Value::String(_) => None,
    }
}

/// Maps a [`Value`] to its ABI tag.
pub fn value_tag(value: &Value) -> i64 {
    match value {
        Value::Number(_) => TAG_NUMBER,
        Value::Boolean(_) => TAG_BOOLEAN,
        Value::String(_) => TAG_STRING,
        Value::Null => TAG_NULL,
    }
}
