use kome_native_rt::number::Number;

#[test]
fn small_integer_is_inline() {
    let number = Number::parse("42").unwrap();

    assert!(number.is_inline());
    assert_eq!(number.to_string(), "42");
}

#[test]
fn large_integer_is_exact() {
    let literal =
        "9999999999999999999999999999999999999999999999999999999999999999";

    let number = Number::parse(literal).unwrap();

    assert!(!number.is_inline());
    assert_eq!(number.to_string(), literal);
}

#[test]
fn decimal_is_exact() {
    let number = Number::parse("1234567890.12345678901234567890").unwrap();

    assert_eq!(
        number.to_string(),
        "1234567890.1234567890123456789"
    );
}

#[test]
fn addition_is_exact() {
    let left = Number::parse("999999999999999999999999999999").unwrap();
    let right = Number::parse("1").unwrap();

    assert_eq!(
        left.add(&right).to_string(),
        "1000000000000000000000000000000"
    );
}

#[test]
fn decimal_addition_is_exact() {
    let left = Number::parse("0.1").unwrap();
    let right = Number::parse("0.2").unwrap();

    assert_eq!(left.add(&right).to_string(), "0.3");
}

#[test]
fn subtraction_is_exact() {
    let left = Number::parse("1000000000000000000000000000000").unwrap();
    let right = Number::parse("1").unwrap();

    assert_eq!(
        left.sub(&right).to_string(),
        "999999999999999999999999999999"
    );
}

#[test]
fn multiplication_is_exact() {
    let left = Number::parse("12.5").unwrap();
    let right = Number::parse("4").unwrap();

    assert_eq!(left.mul(&right).to_string(), "50");
}

#[test]
fn clone_shares_heap_number() {
    let number =
        Number::parse("999999999999999999999999999999999999999").unwrap();

    assert!(number.is_unique());

    let clone = number.clone();

    assert!(!number.is_unique());

    drop(clone);

    assert!(number.is_unique());
}
