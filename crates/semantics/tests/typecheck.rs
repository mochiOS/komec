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
fn checks_named_struct_types_and_retains_fields() {
    let module = parse(
        r#"
struct Point {
    x: Number,
    y: Number,
}

fn identity(point: Point) -> Point {
    return point
}

fn main() {
    let point: Point = { x: 1, y: 2 }
    let x: Number = point.x
}
"#,
    )
    .unwrap();
    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
    assert_eq!(result.structs["Point"].fields.as_ref().unwrap().len(), 2);
}

#[test]
fn retains_runtime_backing_and_reports_duplicate_structs() {
    let module =
        parse("@runtime(\"string\") struct Text\nstruct Duplicate {}\nstruct Duplicate {}")
            .unwrap();
    let result = TypeChecker::check(&module);

    assert_eq!(result.structs["Text"].runtime.as_deref(), Some("string"));
    assert_eq!(result.errors.len(), 1);
    assert!(
        result.errors[0]
            .message
            .contains("duplicate struct `Duplicate`")
    );
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

#[test]
fn accepts_fixed_width_integer_literal() {
    let module = parse(
        r#"
fn main() {
    let value: i32 = 10
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_fractional_literal_for_integer_type() {
    let module = parse(
        r#"
fn main() {
    let value: i32 = 3.14
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_fixed_width_float_literal() {
    let module = parse(
        r#"
fn main() {
    let value: f32 = 3.14
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_implicit_number_to_fixed_width_conversion() {
    let module = parse(
        r#"
fn main() {
    let number = 10
    let value: i32 = number
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn rejects_implicit_fixed_width_to_number_conversion() {
    let module = parse(
        r#"
fn main() {
    let value: i32 = 10
    let number: Number = value
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn rejects_implicit_conversion_between_integer_widths() {
    let module = parse(
        r#"
fn main() {
    let small: i32 = 10
    let large: i64 = small
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_fixed_width_arithmetic() {
    let module = parse(
        r#"
fn main() {
    let left: i32 = 10
    let result = left + 20
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn rejects_arithmetic_between_different_numeric_types() {
    let module = parse(
        r#"
fn main() {
    let left: i32 = 10
    let right: i64 = 20
    let result = left + right
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert_eq!(result.errors.len(), 1);
}

#[test]
fn accepts_fixed_width_compound_assignment() {
    let module = parse(
        r#"
fn main() {
    var value: u64 = 10
    value += 1
}
"#,
    )
    .unwrap();

    let result = TypeChecker::check(&module);

    assert!(result.errors.is_empty());
}
