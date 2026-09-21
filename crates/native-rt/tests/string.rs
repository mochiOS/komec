use kome_native_rt::string::KomeString;
use std::cmp::Ordering;

#[test]
fn stores_utf8_string() {
    let value = KomeString::new("Hello, もち");

    assert_eq!(value.as_str(), "Hello, もち");
}

#[test]
fn reports_utf8_byte_length() {
    let value = KomeString::new("もち");

    assert_eq!(value.byte_len(), "もち".len());
}

#[test]
fn detects_empty_string() {
    assert!(KomeString::new("").is_empty());
    assert!(!KomeString::new("Kome").is_empty());
}

#[test]
fn concatenates_strings() {
    let left = KomeString::new("Hello, ");
    let right = KomeString::new("Kome");

    let result = left.concat(&right);

    assert_eq!(result.as_str(), "Hello, Kome");
}

#[test]
fn compares_strings() {
    let left = KomeString::new("apple");
    let right = KomeString::new("banana");

    assert_eq!(left.compare(&right), Ordering::Less);
    assert_eq!(right.compare(&left), Ordering::Greater);
    assert_eq!(left.compare(&left), Ordering::Equal);
}

#[test]
fn clone_shares_allocation() {
    let value = KomeString::new("shared");

    assert!(value.is_unique());

    let clone = value.clone();

    assert!(!value.is_unique());
    assert_eq!(value.raw(), clone.raw());

    drop(clone);

    assert!(value.is_unique());
}

#[test]
fn raw_round_trip_retains_string() {
    let value = KomeString::new("runtime");
    let raw = value.raw();

    let retained = unsafe { KomeString::from_raw_retain(raw) };

    drop(value);

    assert_eq!(retained.as_str(), "runtime");
    assert!(retained.is_unique());
}
