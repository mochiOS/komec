//! Reference-counted storage for homogeneous generated Kome lists.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::sync::atomic::{AtomicUsize, Ordering};

const HEADER_WORDS: usize = 2;
const WORD: usize = 8;

fn layout(length: usize) -> Layout {
    Layout::from_size_align((HEADER_WORDS + length) * WORD, WORD).expect("valid list layout")
}

/// Allocates a zero-initialized list with one owning reference.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_list_alloc(length: usize) -> u64 {
    let layout = layout(length);
    let base = unsafe { alloc(layout) };
    if base.is_null() {
        handle_alloc_error(layout);
    }
    unsafe {
        base.cast::<AtomicUsize>().write(AtomicUsize::new(1));
        base.add(WORD).cast::<usize>().write(length);
        std::ptr::write_bytes(base.add(HEADER_WORDS * WORD), 0, length * WORD);
        base.add(HEADER_WORDS * WORD) as u64
    }
}

/// Returns the number of elements stored in a list.
///
/// # Safety
///
/// `payload` must be a live handle returned by [`__kome_list_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_list_len(payload: u64) -> usize {
    unsafe { *((payload as *const u8).sub(WORD).cast::<usize>()) }
}

/// Validates a signed list index and returns it as a byte-addressable index.
///
/// Invalid indices terminate with a clear runtime diagnostic.
///
/// # Safety
///
/// `payload` must be a live handle returned by [`__kome_list_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_list_require_index(payload: u64, index: i64) -> usize {
    let length = unsafe { __kome_list_len(payload) };
    if index >= 0 && (index as usize) < length {
        return index as usize;
    }
    eprintln!("runtime error: list index {index} is out of bounds for length {length}");
    std::process::exit(1);
}

/// Increments a generated list's owning reference count.
///
/// # Safety
///
/// `payload` must be a live handle returned by [`__kome_list_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_list_retain(payload: u64) {
    let refs = unsafe {
        (payload as *mut u8)
            .sub(HEADER_WORDS * WORD)
            .cast::<AtomicUsize>()
    };
    unsafe { &*refs }.fetch_add(1, Ordering::Relaxed);
}

/// Releases a list reference and returns `1` to its final owner.
///
/// # Safety
///
/// `payload` must be live and the caller must own one reference. The final
/// owner must release managed elements before calling [`__kome_list_dealloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_list_release(payload: u64) -> u8 {
    let refs = unsafe {
        (payload as *mut u8)
            .sub(HEADER_WORDS * WORD)
            .cast::<AtomicUsize>()
    };
    u8::from(unsafe { &*refs }.fetch_sub(1, Ordering::AcqRel) == 1)
}

/// Frees a final list reference after generated code released its elements.
///
/// # Safety
///
/// `payload` must be the final reference and all managed elements must already
/// have been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_list_dealloc(payload: u64) {
    let length = unsafe { __kome_list_len(payload) };
    let base = unsafe { (payload as *mut u8).sub(HEADER_WORDS * WORD) };
    unsafe { dealloc(base, layout(length)) };
}
