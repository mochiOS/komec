use kome_ast::declarations::{Declaration, TypeMember};
use kome_ast::expressions::Expression;
use kome_ast::patterns::Pattern;
use kome_ast::types::Type;
use kome_parser::{TokenKind, parse, tokenize};

#[test]
fn impl_is_not_a_keyword() {
    let tokens = tokenize("impl Add for Value {}").unwrap();

    assert_eq!(tokens[0].kind, TokenKind::Ident("impl".into()));
    assert_eq!(tokens[2].kind, TokenKind::For);
}

#[test]
fn parses_struct_fields_constants_and_functions() {
    let module = parse(
        r#"
struct Color {
    r: Number
    g: Number
    b: Number

    const BLACK: Color = Color { r: 0 g: 0 b: 0 }

    fn invert(self) -> Color {
        return self
    }
}
"#,
    )
    .unwrap();
    let Declaration::Struct(declaration) = &module.declarations[0] else {
        panic!("expected struct declaration");
    };
    let fields = declaration.fields.as_ref().unwrap();

    assert_eq!(fields.len(), 3);
    assert_eq!(declaration.members.len(), 2);
    assert!(matches!(declaration.members[0], TypeMember::Constant(_)));

    let TypeMember::Function(function) = &declaration.members[1] else {
        panic!("expected function member");
    };
    assert!(matches!(
        &function.params[0],
        Pattern::Ident(identifier) if identifier.name == "self"
    ));

    let TypeMember::Constant(constant) = &declaration.members[0] else {
        unreachable!();
    };
    assert!(matches!(constant.init, Some(Expression::Struct(_))));
}

#[test]
fn parses_for_and_trait_impl_declarations() {
    let module = parse(
        r#"
for Color {
    const WHITE: Color = Color { r: 1, g: 1, b: 1 }
    fn lighten(self) -> Color { return self }
}

for Color: ops::traits::Add {
    fn add(self, other: Color) -> Color { return self }
}
"#,
    )
    .unwrap();

    let Declaration::For(for_declaration) = &module.declarations[0] else {
        panic!("expected for declaration");
    };
    assert_eq!(for_declaration.members.len(), 2);

    let Declaration::For(trait_declaration) = &module.declarations[1] else {
        panic!("expected trait implementation declaration");
    };
    assert!(matches!(
        &trait_declaration.trait_,
        Some(Type::Named(named)) if named.name == "ops::traits::Add"
    ));
    assert!(matches!(
        &trait_declaration.target,
        Type::Named(named) if named.name == "Color"
    ));
}

#[test]
fn rejects_invalid_type_members() {
    assert!(parse("for Color { let value = 1 }").is_err());
    assert!(parse("for Color: Add { value: Number }").is_err());
    assert!(parse("impl Add for Color {}").is_err());
    assert!(parse("struct Color { let value = 1 }").is_err());
}
