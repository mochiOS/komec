//! Reference-counted storage used for user-defined struct values.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::sync::atomic::{AtomicUsize, Ordering};

const HEADER: usize = 8;

fn layout(payload_size: usize) -> Layout {
    Layout::from_size_align(HEADER + payload_size, 8).expect("valid struct allocation layout")
}

/// Allocates a struct payload, zero initialized, with one owning reference.
///
/// `payload_size` is the byte size computed by the code generator from the
/// struct's declared field layout.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_struct_alloc(payload_size: usize) -> u64 {
    let layout = layout(payload_size);
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        handle_alloc_error(layout);
    }
    unsafe {
        base.cast::<AtomicUsize>().write(AtomicUsize::new(1));
        std::ptr::write_bytes(base.add(HEADER), 0, payload_size);
        base.add(HEADER) as u64
    }
}

/// Increments the reference count for a generated struct payload.
///
/// # Safety
///
/// `payload` must be a valid Kome struct payload returned by
/// [`__kome_struct_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_struct_retain(payload: u64) {
    let header = unsafe { (payload as *mut u8).sub(HEADER).cast::<AtomicUsize>() };
    unsafe { &*header }.fetch_add(1, Ordering::Relaxed);
}

/// Returns true when the caller owns the final reference and must destroy fields.
///
/// # Safety
///
/// `payload` must be a valid Kome struct payload and the caller must own one
/// reference to it. When this function returns `1`, the caller must release
/// every managed field and then call [`__kome_struct_dealloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_struct_release(payload: u64) -> u8 {
    let header = unsafe { (payload as *mut u8).sub(HEADER).cast::<AtomicUsize>() };
    let refs = unsafe { &*header };
    u8::from(refs.fetch_sub(1, Ordering::AcqRel) == 1)
}

/// Frees storage after generated code has destroyed every managed field.
///
/// # Safety
///
/// `payload` must be the final reference identified by
/// [`__kome_struct_release`], `payload_size` must match its allocation, and all
/// managed fields must already have been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_struct_dealloc(payload: u64, payload_size: usize) {
    let base = unsafe { (payload as *mut u8).sub(HEADER) };
    unsafe { dealloc(base, layout(payload_size)) };
}
