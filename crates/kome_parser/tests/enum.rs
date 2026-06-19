use kome_ast::types::Type;
use kome_ast::{
    Span,
    declarations::Declaration,
    expressions::{BinaryOp, Expression, LiteralKind, NumberLiteral},
};
use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse, tokenize};

#[test]
fn lexes_enum_keyword() {
    let tokens = tokenize("enum Color {}").unwrap();

    assert_eq!(tokens[0].kind, TokenKind::Enum,);

    assert_eq!(tokens[0].span, Span::new(0, 4),);
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

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.name, "Color");
    assert_eq!(declaration.cases.len(), 3);

    assert_eq!(declaration.cases[0].name, "blue",);

    assert_eq!(declaration.cases[1].name, "red",);

    assert_eq!(declaration.cases[2].name, "green",);
}

#[test]
fn parses_empty_enum() {
    let module = parse("enum Empty {}").unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
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

    let Declaration::Enum(declaration) = &module.declarations[0] else {
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

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.attributes.len(), 1);

    let attribute = &declaration.attributes[0];

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

    let Declaration::Enum(color) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert_eq!(color.name, "Color");

    let Declaration::Component(text) = &module.declarations[1] else {
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
            expected: "`,` or `}` after an enum case",
            found: TokenKind::Ident("red".into(),),
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
            found: TokenKind::String("blue".into(),),
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

#[test]
fn parses_number_valued_enum() {
    let module = parse(
        r#"
        enum HttpStatus {
            ok = 200,
            notFound = 404,
        }
        "#,
    )
    .unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.name, "HttpStatus");
    assert_eq!(declaration.cases.len(), 2);

    let ok = &declaration.cases[0];

    assert_eq!(ok.name, "ok");

    assert!(matches!(
        &ok.value,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::Number(
                    NumberLiteral("200".into())
                )
    ));

    let not_found = &declaration.cases[1];

    assert_eq!(not_found.name, "notFound");

    assert!(matches!(
        &not_found.value,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::Number(
                    NumberLiteral("404".into())
                )
    ));
}

#[test]
fn parses_string_valued_enum() {
    let module = parse(
        r##"
        enum Color {
            blue = "#007aff",
            red = "#ff3b30",
        }
        "##,
    )
    .unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert!(matches!(
        &declaration.cases[0].value,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::String(
                    "#007aff".into()
                )
    ));

    assert!(matches!(
        &declaration.cases[1].value,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::String(
                    "#ff3b30".into()
                )
    ));
}

#[test]
fn parses_enum_value_expression() {
    let module = parse(
        r#"
        enum Size {
            small = 8,
            medium = 8 * 2,
            large = 8 * 4,
        }
        "#,
    )
    .unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert!(matches!(
        &declaration.cases[1].value,
        Some(Expression::Binary(binary))
            if binary.op == BinaryOp::Mul
    ));

    assert!(matches!(
        &declaration.cases[2].value,
        Some(Expression::Binary(binary))
            if binary.op == BinaryOp::Mul
    ));
}

#[test]
fn parses_mixed_valued_and_plain_cases() {
    let module = parse(
        r##"
        enum Color {
            primary,
            accent = "#007aff",
            destructive = "#ff3b30",
        }
        "##,
    )
    .unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    assert_eq!(declaration.cases.len(), 3);

    assert!(declaration.cases[0].value.is_none());

    assert!(declaration.cases[1].value.is_some());

    assert!(declaration.cases[2].value.is_some());
}

#[test]
fn enum_case_span_includes_value() {
    let source = r#"enum Value { answer = 42 }"#;

    let module = parse(source).unwrap();

    let Declaration::Enum(declaration) = &module.declarations[0] else {
        panic!("expected enum declaration");
    };

    let case = &declaration.cases[0];

    assert_eq!(&source[case.span.start..case.span.end], "answer = 42",);
}

#[test]
fn rejects_missing_enum_case_value() {
    let error = parse(
        r#"
        enum Color {
            blue = ,
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
            expected: "an expression",
            found: TokenKind::Comma,
        },
    );
}

#[test]
fn rejects_missing_comma_after_valued_case() {
    let error = parse(
        r#"
        enum HttpStatus {
            ok = 200
            notFound = 404
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
            expected: "`,` or `}` after an enum case",
            found: TokenKind::Ident("notFound".into(),),
        },
    );
}
