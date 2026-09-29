use kome_semantics::resolver::ScopeBuilder;
use kome_semantics::typecheck::{SemanticType, TypeChecker};

#[test]
fn resolves_and_types_generic_struct_members_and_inferred_calls() {
    let module = kome_parser::parse(
        r#"
struct Container<T> { value: T }
fn identity<T>(value: T) -> T { return value }
fn main() {
    let number = Container<Number> { value: 42 }
    let value: Number = identity(number.value)
}
"#,
    )
    .unwrap();
    assert!(ScopeBuilder::resolve(&module).errors.is_empty());
    let result = TypeChecker::check(&module);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.structs["Container"].type_parameters, ["T"]);
}

#[test]
fn rejects_duplicate_parameters_wrong_arity_and_inference_conflicts() {
    let duplicate = kome_parser::parse("struct Bad<T, T> { value: T }").unwrap();
    assert!(
        TypeChecker::check(&duplicate)
            .errors
            .iter()
            .any(|error| error.message.contains("duplicate generic parameter"))
    );

    let wrong_arity = kome_parser::parse(
        "struct Box<T> { value: T } fn main() { Box<Number, String> { value: 1 } }",
    )
    .unwrap();
    assert!(
        TypeChecker::check(&wrong_arity)
            .errors
            .iter()
            .any(|error| error.message.contains("expects 1 type argument"))
    );

    let conflict =
        kome_parser::parse("fn same<T>(a: T, b: T) -> T { return a } fn main() { same(1, \"x\") }")
            .unwrap();
    assert!(
        TypeChecker::check(&conflict)
            .errors
            .iter()
            .any(|error| error.message.contains("conflicting inferred types"))
    );
}

#[test]
fn represents_type_parameters_distinctly() {
    let module = kome_parser::parse("struct Box<T> { value: T }").unwrap();
    let result = TypeChecker::check(&module);
    assert_eq!(
        result.structs["Box"].fields.as_ref().unwrap()["value"],
        SemanticType::TypeParameter("T".into())
    );
}

#[test]
fn validates_generic_trait_implementations() {
    let module = kome_parser::parse(
        r#"
trait Getter<T> { fn get(self) -> T }
struct Box<T> { value: T }
for Box<T>: Getter<T> { fn get(self) -> T { return self.value } }
fn main() {
    let value: Number = Box<Number> { value: 42 }.get()
}
"#,
    )
    .unwrap();
    let result = TypeChecker::check(&module);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn reports_unknown_generic_types_and_invalid_generic_trait_implementations() {
    let unknown = kome_parser::parse("fn broken<T>(value: Missing) -> T { return value }").unwrap();
    assert!(
        ScopeBuilder::resolve(&unknown)
            .errors
            .iter()
            .any(|error| error.to_string().contains("Missing"))
    );

    let invalid = kome_parser::parse(
        r#"
trait Getter<T> { fn get(self) -> T }
struct Box<T> { value: T }
for Box<T>: Getter<T> { fn get(self) -> String { return "wrong" } }
"#,
    )
    .unwrap();
    assert!(
        TypeChecker::check(&invalid)
            .errors
            .iter()
            .any(|error| error.message.contains("incompatible signature"))
    );
}
