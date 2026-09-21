use kome_ast::declarations::Declaration;
use kome_ast::expressions::{Expression, LiteralKind};
use kome_ast::types::{PrimitiveTypeKind, Type};
use kome_parser::{TokenKind, parse, tokenize};

#[test]
fn lexes_struct_keyword() {
    let tokens = tokenize("struct Point {}").unwrap();

    assert_eq!(tokens[0].kind, TokenKind::Struct);
}

#[test]
fn parses_struct_fields_and_trailing_comma() {
    let module = parse("struct Point { x: Number, y: Number, }").unwrap();
    let Declaration::Struct(declaration) = &module.declarations[0] else {
        panic!("expected struct declaration");
    };
    let fields = declaration.fields.as_ref().unwrap();

    assert_eq!(declaration.name, "Point");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "x");
    assert!(matches!(
        fields[1].type_,
        Type::Primitive(ref primitive) if primitive.kind == PrimitiveTypeKind::Number
    ));
}

#[test]
fn distinguishes_empty_and_opaque_structs() {
    let module = parse("struct Empty {}\nstruct Opaque").unwrap();

    let Declaration::Struct(empty) = &module.declarations[0] else {
        panic!("expected struct declaration");
    };
    let Declaration::Struct(opaque) = &module.declarations[1] else {
        panic!("expected struct declaration");
    };

    assert_eq!(empty.fields, Some(Vec::new()));
    assert_eq!(opaque.fields, None);
}

#[test]
fn parses_runtime_backed_opaque_struct() {
    let module = parse("@runtime(\"string\") struct String").unwrap();
    let Declaration::Struct(declaration) = &module.declarations[0] else {
        panic!("expected struct declaration");
    };

    assert!(declaration.fields.is_none());
    assert!(matches!(
        &declaration.attributes[0].args[0],
        Expression::Literal(literal)
            if literal.kind == LiteralKind::String("string".into())
    ));
}
