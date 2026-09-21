//! Binary ABI shared between compiled Kome code and the native runtime.
//!
//! Calls into the native runtime marshal arguments through [`Slot`] values:
//! fixed 16-byte `{ tag, payload }` records. This crate deliberately contains
//! no runtime value model or dispatch implementation.

/// Payload is unused and always `0`.
pub const TAG_NULL: i64 = 0;

/// Used as a return-type tag when the native result is discarded.
pub const TAG_VOID: i64 = 1;

/// Payload holds a raw Kome `Number` runtime handle.
pub const TAG_NUMBER: i64 = 2;

/// Payload holds `0` or `1`.
pub const TAG_BOOLEAN: i64 = 3;

/// Payload holds a raw Kome `String` runtime handle.
pub const TAG_STRING: i64 = 4;

/// One marshalled argument or return value exchanged with the native runtime.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub tag: i64,
    pub payload: i64,
}

impl Slot {
    /// Builds a scalar-value slot.
    pub fn scalar(tag: i64, payload: i64) -> Self {
        Self { tag, payload }
    }
}
