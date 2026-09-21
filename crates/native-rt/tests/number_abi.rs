use kome_native_rt::number::{
    __kome_number_add,
    __kome_number_compare,
    __kome_number_mul,
    __kome_number_parse,
    __kome_number_release,
    __kome_number_sub,
    Number,
};

fn parse_raw(value: &str) -> u64 {
    unsafe {
        __kome_number_parse(value.as_ptr(), value.len())
    }
}

#[test]
fn parses_number_through_abi() {
    let raw = parse_raw("123456789012345678901234567890");

    assert_ne!(raw, 0);

    let number = unsafe { Number::from_raw_retain(raw) };

    assert_eq!(
        number.to_string(),
        "123456789012345678901234567890"
    );

    unsafe {
        __kome_number_release(raw);
    }
}

#[test]
fn adds_numbers_through_abi() {
    let left = parse_raw("0.1");
    let right = parse_raw("0.2");

    let result = unsafe {
        __kome_number_add(left, right)
    };

    let number = unsafe {
        Number::from_raw_retain(result)
    };

    assert_eq!(number.to_string(), "0.3");

    unsafe {
        __kome_number_release(left);
        __kome_number_release(right);
        __kome_number_release(result);
    }
}

#[test]
fn subtracts_numbers_through_abi() {
    let left = parse_raw("1000");
    let right = parse_raw("1");

    let result = unsafe {
        __kome_number_sub(left, right)
    };

    let number = unsafe {
        Number::from_raw_retain(result)
    };

    assert_eq!(number.to_string(), "999");

    unsafe {
        __kome_number_release(left);
        __kome_number_release(right);
        __kome_number_release(result);
    }
}

#[test]
fn multiplies_numbers_through_abi() {
    let left = parse_raw("12.5");
    let right = parse_raw("4");

    let result = unsafe {
        __kome_number_mul(left, right)
    };

    let number = unsafe {
        Number::from_raw_retain(result)
    };

    assert_eq!(number.to_string(), "50");

    unsafe {
        __kome_number_release(left);
        __kome_number_release(right);
        __kome_number_release(result);
    }
}

#[test]
fn compares_numbers_through_abi() {
    let smaller = parse_raw("1");
    let larger = parse_raw("2");

    assert_eq!(
        unsafe {
            __kome_number_compare(smaller, larger)
        },
        -1
    );

    assert_eq!(
        unsafe {
            __kome_number_compare(larger, smaller)
        },
        1
    );

    assert_eq!(
        unsafe {
            __kome_number_compare(smaller, smaller)
        },
        0
    );

    unsafe {
        __kome_number_release(smaller);
        __kome_number_release(larger);
    }
}
