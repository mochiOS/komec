//! Reference-counted storage for generated Kome closure values.

use std::sync::atomic::{AtomicUsize, Ordering};

type EnvironmentDestructor = unsafe extern "C" fn(u64);

struct ClosureValue {
    references: AtomicUsize,
    code: u64,
    environment: u64,
    destroy_environment: u64,
}

/// Creates a closure that owns `environment` until its final release.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_closure_alloc(
    code: u64,
    environment: u64,
    destroy_environment: u64,
) -> u64 {
    Box::into_raw(Box::new(ClosureValue {
        references: AtomicUsize::new(1),
        code,
        environment,
        destroy_environment,
    })) as u64
}

/// Retains a generated closure value.
///
/// # Safety
///
/// `closure` must identify a live value returned by [`__kome_closure_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_closure_retain(closure: u64) {
    let closure = unsafe { &*(closure as *const ClosureValue) };
    closure.references.fetch_add(1, Ordering::Relaxed);
}

/// Releases a generated closure and destroys its captured environment last.
///
/// # Safety
///
/// `closure` must identify a live value returned by [`__kome_closure_alloc`],
/// and the caller must own one reference to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_closure_release(closure: u64) {
    let pointer = closure as *mut ClosureValue;
    let value = unsafe { &*pointer };
    if value.references.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }

    let value = unsafe { Box::from_raw(pointer) };
    let destructor: EnvironmentDestructor =
        unsafe { std::mem::transmute(value.destroy_environment as usize) };
    unsafe { destructor(value.environment) };
}

/// Returns the generated function address stored in a closure.
///
/// # Safety
///
/// `closure` must identify a live value returned by [`__kome_closure_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_closure_code(closure: u64) -> u64 {
    unsafe { (*(closure as *const ClosureValue)).code }
}

/// Returns the captured environment owned by a closure.
///
/// # Safety
///
/// `closure` must identify a live value returned by [`__kome_closure_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_closure_environment(closure: u64) -> u64 {
    unsafe { (*(closure as *const ClosureValue)).environment }
}
