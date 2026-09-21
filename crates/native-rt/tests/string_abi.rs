use kome_native_rt::string::{
    __kome_string_compare, __kome_string_concat, __kome_string_create,
    __kome_string_release, KomeString,
};

fn create_raw(value: &str) -> u64 {
    unsafe { __kome_string_create(value.as_ptr(), value.len()) }
}

#[test]
fn creates_string_through_abi() {
    let raw = create_raw("Hello, もち");

    assert_ne!(raw, 0);

    let value = unsafe { KomeString::from_raw_retain(raw) };

    assert_eq!(value.as_str(), "Hello, もち");

    unsafe {
        __kome_string_release(raw);
    }
}

#[test]
fn concatenates_strings_through_abi() {
    let left = create_raw("Hello, ");
    let right = create_raw("Kome");

    let result = unsafe { __kome_string_concat(left, right) };
    let value = unsafe { KomeString::from_raw_retain(result) };

    assert_eq!(value.as_str(), "Hello, Kome");

    unsafe {
        __kome_string_release(left);
        __kome_string_release(right);
        __kome_string_release(result);
    }
}

#[test]
fn compares_strings_through_abi() {
    let apple = create_raw("apple");
    let banana = create_raw("banana");

    assert_eq!(unsafe { __kome_string_compare(apple, banana) }, -1);
    assert_eq!(unsafe { __kome_string_compare(banana, apple) }, 1);
    assert_eq!(unsafe { __kome_string_compare(apple, apple) }, 0);

    unsafe {
        __kome_string_release(apple);
        __kome_string_release(banana);
    }
}
