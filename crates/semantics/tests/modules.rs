use kome_ast::declarations::Declaration;
use kome_ast::expressions::Expression;
use kome_ast::statements::Statement;
use kome_parser::parse;
use kome_semantics::modules::{SourceModule, link_modules};
use kome_semantics::resolver::ScopeBuilder;
use kome_semantics::typecheck::TypeChecker;

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

#[test]
fn enforces_struct_field_visibility() {
    let linked = link_modules(vec![
        source(
            "library",
            &[],
            r#"
pub struct Value { hidden: Number, pub visible: Number }
for Value {
    pub fn make(value: Number) -> Value {
        return Value { hidden: value, visible: value }
    }
}
"#,
            false,
        ),
        source(
            "app",
            &[],
            r#"
use library::Value
fn main() {
    let value = Value.make(1)
    let hidden = value.hidden
}
"#,
            true,
        ),
    ])
    .unwrap();
    let checked = TypeChecker::check(&linked);
    assert!(
        checked
            .errors
            .iter()
            .any(|error| error.message.contains("member `hidden`")
                && error.message.contains("not visible"))
    );
}

#[test]
fn enforces_inherent_method_visibility() {
    let linked = link_modules(vec![
        source(
            "library",
            &[],
            r#"
pub struct Value {}
for Value {
    pub fn make() -> Value { return Value {} }
    fn hidden(self) -> Number { return 1 }
}
"#,
            false,
        ),
        source(
            "app",
            &[],
            r#"
use library::Value
fn main() {
    let value = Value.make()
    let hidden = value.hidden()
}
"#,
            true,
        ),
    ])
    .unwrap();
    let checked = TypeChecker::check(&linked);
    assert!(
        checked
            .errors
            .iter()
            .any(|error| error.message.contains("member `hidden`")
                && error.message.contains("not visible"))
    );
}

#[test]
fn rejects_private_types_in_public_signatures() {
    let errors = link_modules(vec![source(
        "library",
        &[],
        "struct Hidden {}\npub fn expose() -> Hidden { return Hidden {} }",
        false,
    )])
    .unwrap_err();

    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("public API exposes less-visible type")
    }));
}

#[test]
fn rejects_cyclic_module_imports() {
    let errors = link_modules(vec![
        source(
            "library",
            &["first"],
            "use library::second\npub fn first() {}",
            false,
        ),
        source(
            "library",
            &["second"],
            "use library::first\npub fn second() {}",
            false,
        ),
    ])
    .unwrap_err();

    assert!(errors.iter().any(|error| {
        error.message == "cyclic module import: library::first -> library::second -> library::first"
    }));
}

#[test]
fn resolves_public_reexports_to_the_original_declaration() {
    let linked = link_modules(vec![
        source(
            "library",
            &["internal"],
            "pub struct Value { pub number: Number }",
            false,
        ),
        source("library", &[], "pub use library::internal::Value", false),
        source(
            "app",
            &[],
            "use library::Value\nfn main() { let value = Value { number: 42 } }",
            true,
        ),
    ])
    .unwrap();

    assert!(ScopeBuilder::resolve(&linked).errors.is_empty());
    assert!(TypeChecker::check(&linked).errors.is_empty());
}

#[test]
fn resolves_public_wildcard_reexports() {
    let linked = link_modules(vec![
        source(
            "library",
            &["internal"],
            "pub fn answer() -> Number { return 42 }",
            false,
        ),
        source("library", &[], "pub use library::internal::*", false),
        source(
            "app",
            &[],
            "use library::answer\nfn main() { let value = answer() }",
            true,
        ),
    ])
    .unwrap();

    assert!(ScopeBuilder::resolve(&linked).errors.is_empty());
}

#[test]
fn rejects_reexports_that_widen_visibility() {
    let errors = link_modules(vec![
        source(
            "library",
            &["internal"],
            "pub(package) fn helper() {}",
            false,
        ),
        source("library", &[], "pub use library::internal::helper", false),
    ])
    .unwrap_err();

    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("wider visibility"))
    );
}
