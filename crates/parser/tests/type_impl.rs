use kome_ast::declarations::Declaration;
use kome_ast::types::Type;
use kome_parser::{TokenKind, parse, tokenize};

#[test]
fn impl_is_not_a_keyword() {
    let tokens = tokenize("impl Add for Value {}").unwrap();

    assert_eq!(tokens[0].kind, TokenKind::Ident("impl".into()));
    assert_eq!(tokens[2].kind, TokenKind::For);
}

#[test]
fn parses_trait_declaration() {
    let tokens = tokenize("trait Add {}").unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Trait);

    let module = parse("trait Add { fn add(self, other: Color) -> Color }").unwrap();
    let Declaration::Trait(declaration) = &module.declarations[0] else {
        panic!("expected trait declaration");
    };

    assert_eq!(declaration.name, "Add");
    assert_eq!(declaration.functions.len(), 1);
    assert_eq!(declaration.functions[0].name, "add");
}

#[test]
fn parses_struct_fields() {
    let module = parse(
        r#"
struct Color {
    r: Number
    g: Number
    b: Number
}
"#,
    )
    .unwrap();
    let Declaration::Struct(declaration) = &module.declarations[0] else {
        panic!("expected struct declaration");
    };
    let fields = declaration.fields.as_ref().unwrap();

    assert_eq!(fields.len(), 3);
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
    assert!(parse("struct Color { const VALUE: Number = 1 }").is_err());
    assert!(parse("struct Color { fn value() -> Number }").is_err());
    assert!(parse("trait Add { const VALUE: Number = 1 }").is_err());
}
