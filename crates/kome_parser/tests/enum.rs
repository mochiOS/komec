use kome_ast::{
    Span,
    declarations::Declaration,
    expressions::{
        Expression, LiteralKind,
    },
    types::Type,
};

use kome_parser::{
    FrontendError, ParseErrorKind,
    TokenKind, parse, tokenize,
};

#[test]
fn lexes_enum_keyword() {
    let tokens = tokenize("enum Color {}")
        .unwrap();

    assert_eq!(
        tokens[0].kind,
        TokenKind::Enum,
    );

    assert_eq!(
        tokens[0].span,
        Span::new(0, 4),
    );
}

#[test]
fn parses_enum_declaration() {
    let source = r#"
enum Color {
    blue,
    red,
    green,
}
"#;

    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 1);

    let Declaration::Enum(declaration) =
        &module.declarations[0]
    else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.name, "Color");
    assert_eq!(declaration.cases.len(), 3);

    assert_eq!(
        declaration.cases[0].name,
        "blue",
    );

    assert_eq!(
        declaration.cases[1].name,
        "red",
    );

    assert_eq!(
        declaration.cases[2].name,
        "green",
    );
}

#[test]
fn parses_empty_enum() {
    let module = parse(
        "enum Empty {}",
    )
        .unwrap();

    let Declaration::Enum(declaration) =
        &module.declarations[0]
    else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.name, "Empty");
    assert!(declaration.cases.is_empty());
}

#[test]
fn parses_enum_without_trailing_comma() {
    let module = parse(
        r#"
        enum Alignment {
            start,
            center,
            end
        }
        "#,
    )
        .unwrap();

    let Declaration::Enum(declaration) =
        &module.declarations[0]
    else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.cases.len(), 3);
}

#[test]
fn parses_attributed_enum() {
    let source = r#"
@nativeEnum("Color")
enum Color {
    blue,
    red,
}
"#;

    let module = parse(source).unwrap();

    let Declaration::Enum(declaration) =
        &module.declarations[0]
    else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.attributes.len(), 1);

    let attribute =
        &declaration.attributes[0];

    assert_eq!(attribute.name, "nativeEnum");
    assert_eq!(attribute.args.len(), 1);

    assert!(matches!(
        &attribute.args[0],
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "Color".into()
                )
    ));

    assert_eq!(declaration.span.start, 1);
}

#[test]
fn parses_enum_as_component_parameter_type() {
    let source = r#"
enum Color {
    blue,
    red,
}

component Text(
    color: Color = .blue,
) {}
"#;

    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 2);

    let Declaration::Enum(color) =
        &module.declarations[0]
    else {
        panic!("expected enum declaration");
    };

    assert_eq!(color.name, "Color");

    let Declaration::Component(text) =
        &module.declarations[1]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(text.params.len(), 1);

    assert!(matches!(
        &text.params[0].type_,
        Type::Named(named)
            if named.name == "Color"
    ));

    assert!(matches!(
        &text.params[0].default,
        Some(Expression::DotIdent(value))
            if value.name == "blue"
    ));
}

#[test]
fn rejects_missing_case_comma() {
    let error = parse(
        r#"
        enum Color {
            blue
            red
        }
        "#,
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected:
                "`,` or `}` after an enum case",
            found: TokenKind::Ident(
                "red".into(),
            ),
        },
    );
}

#[test]
fn rejects_invalid_enum_case() {
    let error = parse(
        r#"
        enum Color {
            "blue",
        }
        "#,
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an enum case name",
            found: TokenKind::String(
                "blue".into(),
            ),
        },
    );
}

#[test]
fn rejects_unclosed_enum() {
    let error = parse(
        r#"
        enum Color {
            blue,
            red,
        "#,
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`}`",
            found: TokenKind::Eof,
        },
    );
}