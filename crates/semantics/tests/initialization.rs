use kome_parser::parse;
use kome_semantics::initialization::InitializationChecker;

#[test]
fn rejects_read_before_initialization() {
    let module = parse(
        r#"
fn main() {
    var value: Number
    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].name, "value");
}

#[test]
fn allows_read_after_assignment() {
    let module = parse(
        r#"
fn main() {
    var value: Number
    value = 10
    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn initialized_var_is_immediately_readable() {
    let module = parse(
        r#"
fn main() {
    var value = 10
    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_compound_assignment_before_initialization() {
    let module = parse(
        r#"
fn main() {
    var value: Number
    value += 1
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].name, "value");
}

#[test]
fn allows_value_initialized_in_both_if_branches() {
    let module = parse(
        r#"
fn main(condition: bool) {
    var value: Number

    if condition {
        value = 1
    } else {
        value = 2
    }

    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_value_initialized_in_only_one_if_branch() {
    let module = parse(
        r#"
fn main(condition: bool) {
    var value: Number

    if condition {
        value = 1
    }

    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].name, "value");
}

#[test]
fn loop_assignment_does_not_guarantee_initialization() {
    let module = parse(
        r#"
fn main(condition: bool) {
    var value: Number

    while condition {
        value = 1
    }

    value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].name, "value");
}

#[test]
fn rejects_self_reference_in_initializer() {
    let module = parse(
        r#"
fn main() {
    var value = value
}
"#,
    )
    .unwrap();

    let result = InitializationChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].name, "value");
}
