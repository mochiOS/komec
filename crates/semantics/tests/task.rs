use kome_semantics::resolver::ScopeBuilder;
use kome_semantics::typecheck::TypeChecker;

#[test]
fn resolves_and_types_task_results() {
    let module = kome_parser::parse(
        r#"
fn load_number() -> Number { return 42 }
fn load_text() -> String { return "Kome" }
fn main() {
    let a: Task<Number> = task load_number()
    let b: Task<String> = task load_text()
    let number: Number = wait a
    let text: String = wait b
}
"#,
    )
    .unwrap();
    assert!(ScopeBuilder::resolve(&module).errors.is_empty());
    let result = TypeChecker::check(&module);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn rejects_waiting_for_a_non_task_and_wrong_task_arity() {
    let non_task = kome_parser::parse("fn main() { let value = wait 42 }").unwrap();
    assert!(
        TypeChecker::check(&non_task)
            .errors
            .iter()
            .any(|error| error.message.contains("expects Task<T>"))
    );

    let wrong_arity = kome_parser::parse("fn take(value: Task<Number, String>) {}").unwrap();
    assert!(
        TypeChecker::check(&wrong_arity)
            .errors
            .iter()
            .any(|error| error.message.contains("expects 1 type argument"))
    );
}

#[test]
fn checks_task_annotations_against_the_result_type() {
    let module = kome_parser::parse(
        r#"
fn load() -> Number { return 42 }
fn main() { let pending: Task<String> = task load() }
"#,
    )
    .unwrap();
    assert!(!TypeChecker::check(&module).errors.is_empty());
}

#[test]
fn types_cancel_as_void_and_rejects_non_tasks() {
    let valid = kome_parser::parse(
        "fn value() -> Number { return 1 } fn main() { let work = task value() cancel work }",
    )
    .unwrap();
    assert!(TypeChecker::check(&valid).errors.is_empty());

    let invalid = kome_parser::parse("fn main() { cancel 42 }").unwrap();
    assert!(
        TypeChecker::check(&invalid)
            .errors
            .iter()
            .any(|error| error.message.contains("`cancel` expects Task<T>"))
    );
}
