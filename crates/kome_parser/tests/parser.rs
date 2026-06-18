use kome_ast::{
    AstNode, Span,
    declarations::{Declaration, UseSpecifier},
    expressions::{
        DotIdentifierExpression, Expression, LiteralExpression,
        LiteralKind, NumberLiteral,
    },
    types::{
        NamedType, PrimitiveType, PrimitiveTypeKind, Type,
    },
};

use kome_parser::{
    FrontendError, ParseErrorKind, TokenKind, parse,
};

#[test]
fn parses_empty_module() {
    let module = parse("").unwrap();

    assert!(module.declarations.is_empty());
    assert_eq!(module.span, Span::new(0, 0));
}

#[test]
fn parses_named_use_declaration() {
    let source = "use viewKit";
    let module = parse(source).unwrap();

    assert_eq!(module.span, Span::new(0, source.len()));
    assert_eq!(module.declarations.len(), 1);

    let Declaration::Use(declaration) =
        &module.declarations[0]
    else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.span,
        Span::new(0, source.len()),
    );

    assert_eq!(declaration.source, None);

    assert_eq!(
        declaration.specifiers,
        vec![
            UseSpecifier::Named {
                name: "viewKit".into(),
                span: Span::new(4, 11),
            },
        ],
    );
}

#[test]
fn parses_wildcard_use_declaration() {
    let module = parse("use *").unwrap();

    let Declaration::Use(declaration) =
        &module.declarations[0]
    else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.specifiers,
        vec![
            UseSpecifier::Wildcard {
                span: Span::new(4, 5),
            },
        ],
    );
}

#[test]
fn parses_multiple_use_declarations() {
    let source = r#"use viewKit
use *
use collections, io"#;

    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 3);
    assert_eq!(module.span().end, source.len());

    let Declaration::Use(declaration) =
        &module.declarations[2]
    else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.specifiers,
        vec![
            UseSpecifier::Named {
                name: "collections".into(),
                span: Span::new(22, 33),
            },
            UseSpecifier::Named {
                name: "io".into(),
                span: Span::new(35, 37),
            },
        ],
    );

    assert_eq!(&source[22..33], "collections");
    assert_eq!(&source[35..37], "io");
}

#[test]
fn parses_empty_component() {
    let source = "component App() {}";
    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 1);

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(
        component.span,
        Span::new(0, source.len()),
    );

    assert_eq!(component.name, "App");
    assert!(component.params.is_empty());
    assert!(component.attributes.is_empty());
    assert!(component.body.is_empty());
}

#[test]
fn parses_component_with_attribute() {
    let source = r#"@application
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(
        component.span,
        Span::new(0, source.len()),
    );

    assert_eq!(component.name, "App");
    assert_eq!(component.attributes.len(), 1);

    let attribute = &component.attributes[0];

    assert_eq!(
        attribute.span,
        Span::new(0, 12),
    );

    assert_eq!(attribute.name, "application");
    assert!(attribute.args.is_empty());

    assert_eq!(
        &source[
            attribute.span.start
                ..attribute.span.end
        ],
        "@application",
    );
}

#[test]
fn parses_component_with_multiple_attributes() {
    let source = r#"@application
@preview
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.attributes.len(), 2);
    assert_eq!(
        component.attributes[0].name,
        "application",
    );
    assert_eq!(
        component.attributes[1].name,
        "preview",
    );
}

#[test]
fn parses_component_parameter_with_default() {
    let source =
        r#"component App(title: String = "Kome") {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.params.len(), 1);

    let parameter = &component.params[0];

    assert_eq!(
        parameter.span,
        Span::new(14, 36),
    );

    assert_eq!(parameter.name, "title");

    assert_eq!(
        parameter.type_,
        Type::Primitive(PrimitiveType {
            span: Span::new(21, 27),
            kind: PrimitiveTypeKind::String,
        }),
    );

    assert_eq!(
        parameter.default,
        Some(Expression::Literal(
            LiteralExpression {
                span: Span::new(30, 36),
                kind: LiteralKind::String(
                    "Kome".into(),
                ),
            },
        )),
    );

    assert_eq!(
        &source[
            parameter.span.start
                ..parameter.span.end
        ],
        r#"title: String = "Kome""#,
    );
}

#[test]
fn parses_multiple_component_parameters() {
    let source = concat!(
    "component Button(",
    "title: String, ",
    "count: Number = 0, ",
    "enabled: Boolean = true,",
    ") {}",
    );

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.params.len(), 3);

    let title = &component.params[0];

    assert_eq!(title.name, "title");
    assert_eq!(title.span, Span::new(17, 30));
    assert_eq!(title.default, None);

    assert_eq!(
        title.type_,
        Type::Primitive(PrimitiveType {
            span: Span::new(24, 30),
            kind: PrimitiveTypeKind::String,
        }),
    );

    let count = &component.params[1];

    assert_eq!(count.name, "count");
    assert_eq!(count.span, Span::new(32, 49));

    assert_eq!(
        count.default,
        Some(Expression::Literal(
            LiteralExpression {
                span: Span::new(48, 49),
                kind: LiteralKind::Number(
                    NumberLiteral("0".into()),
                ),
            },
        )),
    );

    let enabled = &component.params[2];

    assert_eq!(enabled.name, "enabled");
    assert_eq!(
        enabled.span,
        Span::new(51, 74),
    );

    assert_eq!(
        enabled.default,
        Some(Expression::Literal(
            LiteralExpression {
                span: Span::new(70, 74),
                kind: LiteralKind::Boolean(true),
            },
        )),
    );
}

#[test]
fn parses_named_component_parameter_type() {
    let source = "component App(content: View) {}";
    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(
        component.params[0].type_,
        Type::Named(NamedType {
            span: Span::new(23, 27),
            name: "View".into(),
            type_arguments: Vec::new(),
        }),
    );
}

#[test]
fn parses_dot_identifier_parameter_default() {
    let source =
        "component Stack(spacing: Spacing = .large) {}";

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    let parameter = &component.params[0];

    assert_eq!(
        parameter.default,
        Some(Expression::DotIdent(
            DotIdentifierExpression {
                span: Span::new(35, 41),
                name: "large".into(),
            },
        )),
    );
}

#[test]
fn rejects_missing_use_specifier() {
    let error = parse("use").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an import name or `*`",
            found: TokenKind::Eof,
        },
    );
}

#[test]
fn rejects_attribute_without_declaration() {
    let error = parse("@application").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a declaration after attributes",
            found: TokenKind::Eof,
        },
    );
}

#[test]
fn rejects_attribute_on_use_declaration() {
    let error =
        parse("@application use viewKit").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected:
                "a component or function declaration after attributes",
            found: TokenKind::Use,
        },
    );
}

#[test]
fn rejects_parameter_without_type_annotation() {
    let error =
        parse("component App(title) {}").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`:`",
            found: TokenKind::RParen,
        },
    );
}

#[test]
fn rejects_parameter_without_default_value() {
    let error = parse(
        "component App(title: String = ) {}",
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a parameter default value",
            found: TokenKind::RParen,
        },
    );
}

#[test]
fn rejects_component_members_for_now() {
    let error = parse(
        "component App() { state counter = 0 }",
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`}`",
            found: TokenKind::State,
        },
    );
}