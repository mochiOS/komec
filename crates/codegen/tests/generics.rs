use kome_codegen::compile::analyze_module;

#[test]
fn caches_repeated_function_specializations() {
    let module = kome_parser::parse(
        r#"
fn identity<T>(value: T) -> T { return value }
fn main() {
    identity(1)
    identity(2)
    identity(3)
}
"#,
    )
    .unwrap();
    let info = analyze_module(&module).unwrap();
    assert_eq!(
        info.user_functions()
            .filter(|(name, _, _)| name.starts_with("identity$"))
            .count(),
        1
    );
}

#[test]
fn reports_generic_codegen_errors_before_cranelift() {
    let wrong_arity = kome_parser::parse(
        "struct Box<T> { value: T } fn main() { Box<Number, String> { value: 1 } }",
    )
    .unwrap();
    assert!(
        analyze_module(&wrong_arity)
            .unwrap_err()
            .message()
            .contains("expects 1 type argument")
    );

    let conflict =
        kome_parser::parse("fn same<T>(a: T, b: T) -> T { return a } fn main() { same(1, \"x\") }")
            .unwrap();
    assert!(
        analyze_module(&conflict)
            .unwrap_err()
            .message()
            .contains("conflicting inferred types")
    );

    let recursive =
        kome_parser::parse("struct Node<T> { next: Node<T> } fn main(value: Node<Number>) {}")
            .unwrap();
    assert!(
        analyze_module(&recursive)
            .unwrap_err()
            .message()
            .contains("recursive generic layout")
    );

    let unresolved =
        kome_parser::parse("fn broken<T>(value: T) -> U { return value } fn main() { broken(1) }")
            .unwrap();
    assert!(
        analyze_module(&unresolved)
            .unwrap_err()
            .message()
            .contains("unresolved generic type")
    );
}
