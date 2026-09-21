//! Runtime representation of Kome's managed `String` type.
//!
//! Strings are UTF-8 and stored in reference-counted heap allocations.
//! Copies share the same allocation. Future mutation can use `is_unique()`
//! to implement copy-on-write behavior.

use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt;

/// A runtime Kome `String`.
///
/// The representation is always a 64-bit pointer-sized runtime handle.
pub struct KomeString {
    raw: u64,
}

struct HeapString {
    references: Cell<usize>,
    value: String,
}

impl KomeString {
    /// Creates a managed Kome string from UTF-8 text.
    pub fn new(value: impl Into<String>) -> Self {
        Self::allocate(value.into())
    }

    /// Returns the string contents as UTF-8 text.
    pub fn as_str(&self) -> &str {
        &unsafe { heap_string(self.raw) }.value
    }

    /// Returns the number of bytes in the UTF-8 representation.
    pub fn byte_len(&self) -> usize {
        self.as_str().len()
    }

    /// Returns whether the string is empty.
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// Concatenates two strings into a new managed string.
    pub fn concat(&self, other: &Self) -> Self {
        let left = self.as_str();
        let right = other.as_str();

        let mut value = String::with_capacity(left.len() + right.len());

        value.push_str(left);
        value.push_str(right);

        Self::allocate(value)
    }

    /// Compares two strings lexicographically.
    pub fn compare(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }

    /// Returns whether this string has exactly one heap owner.
    pub fn is_unique(&self) -> bool {
        unsafe { heap_string(self.raw) }.references.get() == 1
    }

    /// Returns the raw runtime representation without transferring ownership.
    pub const fn raw(&self) -> u64 {
        self.raw
    }

    /// Transfers ownership into the raw runtime representation.
    ///
    /// The caller becomes responsible for eventually releasing the returned
    /// handle.
    pub fn into_raw(self) -> u64 {
        let raw = self.raw;

        std::mem::forget(self);

        raw
    }

    /// Creates an owned `KomeString` from a raw runtime representation.
    ///
    /// The returned value acquires an additional reference.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid Kome `String` runtime handle.
    pub unsafe fn from_raw_retain(raw: u64) -> Self {
        retain_raw(raw);

        Self { raw }
    }

    fn allocate(value: String) -> Self {
        let value = Box::new(HeapString {
            references: Cell::new(1),
            value,
        });

        let raw = Box::into_raw(value) as u64;

        Self { raw }
    }
}

impl Clone for KomeString {
    fn clone(&self) -> Self {
        retain_raw(self.raw);

        Self { raw: self.raw }
    }
}

impl Drop for KomeString {
    fn drop(&mut self) {
        release_raw(self.raw);
    }
}

impl PartialEq for KomeString {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for KomeString {}

impl PartialOrd for KomeString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other))
    }
}

impl Ord for KomeString {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl fmt::Debug for KomeString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KomeString")
            .field("value", &self.as_str())
            .finish()
    }
}

impl fmt::Display for KomeString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Increments the reference count for a raw Kome `String` handle.
///
/// # Safety
///
/// `raw` must be a valid Kome `String` runtime handle.
pub unsafe fn retain(raw: u64) {
    retain_raw(raw);
}

/// Decrements the reference count for a raw Kome `String` handle.
///
/// # Safety
///
/// `raw` must be a valid Kome `String` runtime handle and the caller must own
/// one reference to it.
pub unsafe fn release(raw: u64) {
    release_raw(raw);
}

// -- Public ABI --

/// Creates a managed Kome `String` from UTF-8 bytes.
///
/// Returns `0` if the input is not valid UTF-8.
///
/// # Safety
///
/// `pointer` must point to `length` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_string_create(pointer: *const u8, length: usize) -> u64 {
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };

    let Ok(value) = std::str::from_utf8(bytes) else {
        return 0;
    };

    KomeString::new(value).into_raw()
}

/// Increments the reference count for a raw Kome `String` handle.
///
/// # Safety
///
/// `raw` must be a valid Kome `String` runtime handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_string_retain(raw: u64) {
    unsafe {
        retain(raw);
    }
}

/// Decrements the reference count for a raw Kome `String` handle.
///
/// # Safety
///
/// `raw` must be a valid Kome `String` runtime handle and the caller must own
/// one reference to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_string_release(raw: u64) {
    unsafe {
        release(raw);
    }
}

/// Concatenates two Kome `String` values.
///
/// # Safety
///
/// `left` and `right` must be valid Kome `String` runtime handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_string_concat(left: u64, right: u64) -> u64 {
    let left = unsafe { KomeString::from_raw_retain(left) };
    let right = unsafe { KomeString::from_raw_retain(right) };

    left.concat(&right).into_raw()
}

/// Compares two Kome `String` values.
///
/// Returns `-1`, `0`, or `1`.
///
/// # Safety
///
/// `left` and `right` must be valid Kome `String` runtime handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_string_compare(left: u64, right: u64) -> i32 {
    let left = unsafe { KomeString::from_raw_retain(left) };
    let right = unsafe { KomeString::from_raw_retain(right) };

    match left.compare(&right) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

unsafe fn heap_string(raw: u64) -> &'static HeapString {
    unsafe { &*(raw as *const HeapString) }
}

fn retain_raw(raw: u64) {
    let value = unsafe { heap_string(raw) };
    let references = value.references.get();

    value.references.set(
        references
            .checked_add(1)
            .expect("String reference count overflow"),
    );
}

fn release_raw(raw: u64) {
    let pointer = raw as *mut HeapString;
    let value = unsafe { &*pointer };
    let references = value.references.get();

    debug_assert!(references > 0);

    if references == 1 {
        unsafe {
            drop(Box::from_raw(pointer));
        }

        return;
    }

    value.references.set(references - 1);
}
