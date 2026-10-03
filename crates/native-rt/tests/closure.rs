use kome_native_rt::closure::{
    __kome_closure_alloc, __kome_closure_code, __kome_closure_environment,
};
use std::sync::atomic::{AtomicUsize, Ordering};

static DESTROYED: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn destroy_environment(environment: u64) {
    assert_eq!(environment, 17);
    DESTROYED.fetch_add(1, Ordering::SeqCst);
}

#[repr(C)]
struct ClosurePrefix {
    code: u64,
    environment: u64,
    retain: unsafe extern "C" fn(u64),
    release: unsafe extern "C" fn(u64),
}

#[test]
fn exposes_a_stable_owned_c_callback_prefix() {
    DESTROYED.store(0, Ordering::SeqCst);
    let closure = __kome_closure_alloc(11, 17, destroy_environment as *const () as usize as u64);
    let prefix = unsafe { &*(closure as *const ClosurePrefix) };

    assert_eq!(prefix.code, 11);
    assert_eq!(prefix.environment, 17);
    assert_eq!(unsafe { __kome_closure_code(closure) }, 11);
    assert_eq!(unsafe { __kome_closure_environment(closure) }, 17);

    unsafe {
        (prefix.retain)(closure);
        (prefix.release)(closure);
    }
    assert_eq!(DESTROYED.load(Ordering::SeqCst), 0);

    unsafe { (prefix.release)(closure) };
    assert_eq!(DESTROYED.load(Ordering::SeqCst), 1);
}
