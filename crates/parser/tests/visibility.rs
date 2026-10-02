use kome_ast::declarations::{Declaration, TypeMember, Visibility};
use kome_parser::parse;

#[test]
fn parses_top_level_visibility() {
    let module = parse(
        r#"
pub struct PublicType {}
pub(package) fn shared() {}
fn private() {}
"#,
    )
    .unwrap();

    let Declaration::Struct(public_type) = &module.declarations[0] else {
        panic!("expected a struct declaration");
    };
    assert_eq!(public_type.visibility, Visibility::Public);

    let Declaration::Function(shared) = &module.declarations[1] else {
        panic!("expected a function declaration");
    };
    assert_eq!(shared.visibility, Visibility::Package);

    let Declaration::Function(private) = &module.declarations[2] else {
        panic!("expected a function declaration");
    };
    assert_eq!(private.visibility, Visibility::Private);
}

#[test]
fn parses_field_and_inherent_member_visibility() {
    let module = parse(
        r#"
pub struct User {
    pub name: String,
    secret: String,
}

for User {
    pub fn name(self) -> String { return self.name }
    fn validate(self) -> bool { return true }
}
"#,
    )
    .unwrap();

    let Declaration::Struct(user) = &module.declarations[0] else {
        panic!("expected a struct declaration");
    };
    let fields = user.fields.as_ref().unwrap();
    assert_eq!(fields[0].visibility, Visibility::Public);
    assert_eq!(fields[1].visibility, Visibility::Private);

    let Declaration::For(implementation) = &module.declarations[1] else {
        panic!("expected an implementation declaration");
    };
    let TypeMember::Function(name) = &implementation.members[0] else {
        panic!("expected a function member");
    };
    let TypeMember::Function(validate) = &implementation.members[1] else {
        panic!("expected a function member");
    };
    assert_eq!(name.visibility, Visibility::Public);
    assert_eq!(validate.visibility, Visibility::Private);
}

#[test]
fn trait_functions_are_public_by_definition() {
    let module = parse("pub trait Display { fn display(self) -> String }").unwrap();
    let Declaration::Trait(display) = &module.declarations[0] else {
        panic!("expected a trait declaration");
    };
    assert_eq!(display.visibility, Visibility::Public);
    assert_eq!(display.functions[0].visibility, Visibility::Public);
}

#[test]
fn parses_public_reexports() {
    let module =
        parse("pub use library::Value\npub(package) use library::helper as shared").unwrap();

    let Declaration::Use(public) = &module.declarations[0] else {
        panic!("expected a use declaration");
    };
    let Declaration::Use(package) = &module.declarations[1] else {
        panic!("expected a use declaration");
    };
    assert_eq!(public.visibility, Visibility::Public);
    assert_eq!(package.visibility, Visibility::Package);
}
