//! Runtime representation of Kome's arbitrary-precision `Number` type.
//!
//! Small integers are stored directly inside the 64-bit value representation.
//! Larger integers and decimal values are stored in reference-counted heap
//! allocations.

use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt;

/// The low bit marks an inline integer.
///
/// Heap allocations are aligned, so valid pointers always have the low bit
/// cleared.
const SMALL_INTEGER_TAG: u64 = 1;

/// Small integers have 63 payload bits including the sign.
const SMALL_INTEGER_MIN: i64 = -(1_i64 << 62);
const SMALL_INTEGER_MAX: i64 = (1_i64 << 62) - 1;

/// A runtime Kome `Number`.
///
/// The representation is always exactly 64 bits. Small integers are stored
/// directly. Larger integers and decimal values are represented by a pointer
/// to a reference-counted heap allocation.
pub struct Number {
    raw: u64,
}

/// Heap representation for arbitrary-precision `Number` values.
///
/// The numeric value is:
///
/// ```text
/// coefficient × 10^-scale
/// ```
///
/// For example, `12.34` is represented as `coefficient = 1234` and
/// `scale = 2`.
struct HeapNumber {
    references: Cell<usize>,
    coefficient: BigInt,
    scale: u32,
}

/// An error produced while parsing a Kome number literal.
#[derive(Debug, Clone)]
pub struct ParseNumberError {
    literal: String,
}

impl fmt::Display for ParseNumberError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "`{}` is not a valid Number", self.literal)
    }
}

impl std::error::Error for ParseNumberError {}

impl Number {
    /// Parses a decimal Kome number literal.
    ///
    /// Integer literals use the inline representation whenever possible.
    pub fn parse(literal: &str) -> Result<Self, ParseNumberError> {
        let (coefficient, scale) = parse_decimal(literal).ok_or_else(|| ParseNumberError {
            literal: literal.to_owned(),
        })?;

        Ok(Self::from_parts(coefficient, scale))
    }

    /// Creates a `Number` from a signed integer.
    pub fn from_i64(value: i64) -> Self {
        if let Some(raw) = encode_small_integer(value) {
            return Self { raw };
        }

        Self::allocate(BigInt::from(value), 0)
    }

    /// Returns the raw 64-bit runtime representation.
    ///
    /// This does not transfer ownership of the value.
    pub const fn raw(&self) -> u64 {
        self.raw
    }

    /// Creates an owned `Number` from a raw runtime representation.
    ///
    /// The new value acquires its own reference to heap-backed numbers.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid Kome `Number` representation.
    pub unsafe fn from_raw_retain(raw: u64) -> Self {
        retain_raw(raw);

        Self { raw }
    }

    /// Adds two numbers.
    pub fn add(&self, other: &Self) -> Self {
        if let (Some(left), Some(right)) = (
            decode_small_integer(self.raw),
            decode_small_integer(other.raw),
        ) {
            if let Some(result) = left.checked_add(right) {
                if let Some(raw) = encode_small_integer(result) {
                    return Self { raw };
                }
            }
        }

        let (left, left_scale) = self.parts();
        let (right, right_scale) = other.parts();

        let scale = left_scale.max(right_scale);

        let left = scale_coefficient(left, scale - left_scale);
        let right = scale_coefficient(right, scale - right_scale);

        Self::from_parts(left + right, scale)
    }

    /// Subtracts `other` from this number.
    pub fn sub(&self, other: &Self) -> Self {
        if let (Some(left), Some(right)) = (
            decode_small_integer(self.raw),
            decode_small_integer(other.raw),
        ) {
            if let Some(result) = left.checked_sub(right) {
                if let Some(raw) = encode_small_integer(result) {
                    return Self { raw };
                }
            }
        }

        let (left, left_scale) = self.parts();
        let (right, right_scale) = other.parts();

        let scale = left_scale.max(right_scale);

        let left = scale_coefficient(left, scale - left_scale);
        let right = scale_coefficient(right, scale - right_scale);

        Self::from_parts(left - right, scale)
    }

    /// Multiplies two numbers.
    pub fn mul(&self, other: &Self) -> Self {
        if let (Some(left), Some(right)) = (
            decode_small_integer(self.raw),
            decode_small_integer(other.raw),
        ) {
            if let Some(result) = left.checked_mul(right) {
                if let Some(raw) = encode_small_integer(result) {
                    return Self { raw };
                }
            }
        }

        let (left, left_scale) = self.parts();
        let (right, right_scale) = other.parts();

        Self::from_parts(left * right, left_scale + right_scale)
    }

    /// Compares two numbers numerically.
    pub fn compare(&self, other: &Self) -> Ordering {
        if let (Some(left), Some(right)) = (
            decode_small_integer(self.raw),
            decode_small_integer(other.raw),
        ) {
            return left.cmp(&right);
        }

        let (left, left_scale) = self.parts();
        let (right, right_scale) = other.parts();

        let scale = left_scale.max(right_scale);

        let left = scale_coefficient(left, scale - left_scale);
        let right = scale_coefficient(right, scale - right_scale);

        left.cmp(&right)
    }

    /// Returns whether this number uses the inline integer representation.
    pub const fn is_inline(&self) -> bool {
        is_small_integer(self.raw)
    }

    /// Returns whether this value has a unique heap owner.
    ///
    /// Inline values have no heap allocation and are therefore always
    /// considered unique.
    pub fn is_unique(&self) -> bool {
        if is_small_integer(self.raw) {
            return true;
        }

        let heap = unsafe { heap_number(self.raw) };

        heap.references.get() == 1
    }

    fn from_parts(mut coefficient: BigInt, mut scale: u32) -> Self {
        normalize(&mut coefficient, &mut scale);

        if scale == 0 {
            if let Some(value) = coefficient.to_i64() {
                if let Some(raw) = encode_small_integer(value) {
                    return Self { raw };
                }
            }
        }

        Self::allocate(coefficient, scale)
    }

    fn allocate(coefficient: BigInt, scale: u32) -> Self {
        let number = Box::new(HeapNumber {
            references: Cell::new(1),
            coefficient,
            scale,
        });

        let raw = Box::into_raw(number) as u64;

        debug_assert_eq!(raw & SMALL_INTEGER_TAG, 0);

        Self { raw }
    }

    fn parts(&self) -> (BigInt, u32) {
        if let Some(value) = decode_small_integer(self.raw) {
            return (BigInt::from(value), 0);
        }

        let number = unsafe { heap_number(self.raw) };

        (number.coefficient.clone(), number.scale)
    }
}

impl Clone for Number {
    fn clone(&self) -> Self {
        retain_raw(self.raw);

        Self { raw: self.raw }
    }
}

impl Drop for Number {
    fn drop(&mut self) {
        release_raw(self.raw);
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl Eq for Number {}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other))
    }
}

impl Ord for Number {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

impl fmt::Debug for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Number")
            .field("value", &self.to_string())
            .field("inline", &self.is_inline())
            .finish()
    }
}

impl fmt::Display for Number {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (coefficient, scale) = self.parts();

        if scale == 0 {
            return write!(formatter, "{coefficient}");
        }

        let negative = coefficient.sign() == num_bigint::Sign::Minus;
        let mut digits = coefficient.magnitude().to_str_radix(10);

        if digits.len() <= scale as usize {
            let zeros = scale as usize + 1 - digits.len();

            digits = format!("{}{}", "0".repeat(zeros), digits);
        }

        let decimal_index = digits.len() - scale as usize;

        if negative {
            formatter.write_str("-")?;
        }

        formatter.write_str(&digits[..decimal_index])?;
        formatter.write_str(".")?;
        formatter.write_str(&digits[decimal_index..])
    }
}

/// Increments the reference count for a raw `Number` representation.
///
/// Inline numbers require no bookkeeping.
///
/// # Safety
///
/// `raw` must be a valid Kome `Number` representation.
pub unsafe fn retain(raw: u64) {
    retain_raw(raw);
}

/// Decrements the reference count for a raw `Number` representation.
///
/// The heap allocation is destroyed when the final reference is released.
///
/// # Safety
///
/// `raw` must be a valid Kome `Number` representation and the caller must own
/// one reference to it.
pub unsafe fn release(raw: u64) {
    release_raw(raw);
}

fn parse_decimal(literal: &str) -> Option<(BigInt, u32)> {
    let mut split = literal.split('.');

    let integer = split.next()?;
    let fraction = split.next();

    if split.next().is_some() || integer.is_empty() {
        return None;
    }

    match fraction {
        Some(fraction) => {
            if fraction.is_empty() {
                return None;
            }

            let digits = format!("{integer}{fraction}");
            let coefficient = digits.parse::<BigInt>().ok()?;

            Some((coefficient, fraction.len() as u32))
        }

        None => {
            let coefficient = integer.parse::<BigInt>().ok()?;

            Some((coefficient, 0))
        }
    }
}

fn normalize(coefficient: &mut BigInt, scale: &mut u32) {
    if coefficient.is_zero() {
        *scale = 0;
        return;
    }

    let ten = BigInt::from(10_u8);

    while *scale > 0 && (&*coefficient % &ten).is_zero() {
        *coefficient /= &ten;
        *scale -= 1;
    }
}

fn scale_coefficient(coefficient: BigInt, digits: u32) -> BigInt {
    if digits == 0 {
        return coefficient;
    }

    coefficient * BigInt::from(10_u8).pow(digits)
}

const fn is_small_integer(raw: u64) -> bool {
    raw & SMALL_INTEGER_TAG != 0
}

fn encode_small_integer(value: i64) -> Option<u64> {
    if !(SMALL_INTEGER_MIN..=SMALL_INTEGER_MAX).contains(&value) {
        return None;
    }

    Some(((value as u64) << 1) | SMALL_INTEGER_TAG)
}

fn decode_small_integer(raw: u64) -> Option<i64> {
    if !is_small_integer(raw) {
        return None;
    }

    Some((raw as i64) >> 1)
}

unsafe fn heap_number(raw: u64) -> &'static HeapNumber {
    debug_assert!(!is_small_integer(raw));

    unsafe { &*(raw as *const HeapNumber) }
}

fn retain_raw(raw: u64) {
    if is_small_integer(raw) {
        return;
    }

    let number = unsafe { heap_number(raw) };
    let references = number.references.get();

    number.references.set(
        references
            .checked_add(1)
            .expect("Number reference count overflow"),
    );
}

fn release_raw(raw: u64) {
    if is_small_integer(raw) {
        return;
    }

    let pointer = raw as *mut HeapNumber;
    let number = unsafe { &*pointer };
    let references = number.references.get();

    debug_assert!(references > 0);

    if references == 1 {
        unsafe {
            drop(Box::from_raw(pointer));
        }

        return;
    }

    number.references.set(references - 1);
}
