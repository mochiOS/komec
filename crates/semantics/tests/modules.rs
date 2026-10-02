use kome_ast::declarations::Declaration;
use kome_ast::expressions::Expression;
use kome_ast::statements::Statement;
use kome_parser::parse;
use kome_semantics::modules::{SourceModule, link_modules};
use kome_semantics::resolver::ScopeBuilder;

fn source(package: &str, path: &[&str], text: &str, application: bool) -> SourceModule {
    SourceModule::new(
        package,
        path.iter().map(|value| (*value).to_owned()).collect(),
        parse(text).unwrap(),
        application,
    )
}

#[test]
fn qualifies_module_declarations_and_references() {
    let linked = link_modules(vec![
        source("std", &["io"], "pub fn println(value: String) {}", false),
        source(
            "app",
            &[],
            "use std::io\nfn main() { io::println(\"hello\") }",
            true,
        ),
    ])
    .unwrap();

    let Declaration::Function(println) = &linked.declarations[0] else {
        panic!("expected function declaration");
    };
    assert_eq!(println.name, "std::io::println");

    let Declaration::Function(main) = &linked.declarations[1] else {
        panic!("expected application function");
    };
    let Statement::Expression(statement) = &main.body.as_ref().unwrap().statements[0] else {
        panic!("expected expression statement");
    };
    let Expression::Call(call) = &statement.expression else {
        panic!("expected call expression");
    };
    assert!(matches!(
        call.callee.as_ref(),
        Expression::Ident(identifier) if identifier.name == "std::io::println"
    ));
    assert!(ScopeBuilder::resolve(&linked).errors.is_empty());
}

#[test]
fn supports_selective_aliases_and_wildcards() {
    let linked = link_modules(vec![
        source("tools", &[], "pub fn first() {}\npub fn second() {}", false),
        source(
            "app",
            &[],
            "use tools::first as one\nuse tools::*\nfn main() { one() second() }",
            true,
        ),
    ])
    .unwrap();
    assert!(ScopeBuilder::resolve(&linked).errors.is_empty());
}

#[test]
fn rejects_private_cross_module_access() {
    let errors = link_modules(vec![
        source("library", &[], "fn hidden() {}", false),
        source(
            "app",
            &[],
            "use library\nfn main() { library::hidden() }",
            true,
        ),
    ])
    .unwrap_err();

    assert!(errors[0].message.contains("not visible"));
}

#[test]
fn allows_package_visibility_only_inside_the_package() {
    let linked = link_modules(vec![
        source("library", &["shared"], "pub(package) fn helper() {}", false),
        source(
            "library",
            &["client"],
            "use library::shared\npub fn call() { shared::helper() }",
            false,
        ),
    ])
    .unwrap();
    assert!(ScopeBuilder::resolve(&linked).errors.is_empty());

    let errors = link_modules(vec![
        source("library", &["shared"], "pub(package) fn helper() {}", false),
        source(
            "app",
            &[],
            "use library::shared\nfn main() { shared::helper() }",
            true,
        ),
    ])
    .unwrap_err();
    assert!(errors[0].message.contains("not visible"));
}
