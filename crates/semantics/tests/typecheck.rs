use kome_parser::parse;
use kome_semantics::typecheck::TypeChecker;

#[test]
fn infers_primitive_binding_types() {
    let module = parse(
        r#"
fn main() {
    let number = 1
    let string = "Kome"
    let boolean = true
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn accepts_matching_binding_annotation() {
    let module = parse(
        r#"
fn main() {
    let value: Number = 1
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_mismatched_binding_annotation() {
    let module = parse(
        r#"
fn main() {
    let value: Number = "Kome"
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_matching_assignment() {
    let module = parse(
        r#"
fn main() {
    var value: Number
    value = 1
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_mismatched_assignment() {
    let module = parse(
        r#"
fn main() {
    var value: Number
    value = "Kome"
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_matching_return_type() {
    let module = parse(
        r#"
fn value() -> Number {
    return 1
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_mismatched_return_type() {
    let module = parse(
        r#"
fn value() -> String {
    return 1
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_null_for_optional_type() {
    let module = parse(
        r#"
fn main() {
    let name: String? = null
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_null_for_non_optional_type() {
    let module = parse(
        r#"
fn main() {
    let name: String = null
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn preserves_named_types() {
    let module = parse(
        r#"
fn set_color(color: Color) {
}

fn main() {
    var color: Color
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn resolves_dot_identifier_from_component_parameter_type() {
    let module = parse(
        r#"
component Text(color: Color) {}

fn main() {
    Text(color: .blue)
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn requires_bool_condition() {
    let module = parse(
        r#"
fn main() {
    if 1 {
    }
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}
