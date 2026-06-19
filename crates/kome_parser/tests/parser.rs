use kome_ast::{
    AstNode, Span,
    declarations::{ComponentMember, Declaration, UseSpecifier},
    expressions::{
        DotIdentifierExpression, Expression, LiteralExpression, LiteralKind, NumberLiteral,
    },
    patterns::{IdentifierPattern, Pattern},
    types::{NamedType, PrimitiveType, PrimitiveTypeKind, Type},
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse};

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

    assert_eq!(module.span, Span::new(0, source.len()),);

    assert_eq!(module.declarations.len(), 1);

    let Declaration::Use(declaration) = &module.declarations[0] else {
        panic!("expected use declaration");
    };

    assert_eq!(declaration.span, Span::new(0, source.len()),);

    assert_eq!(declaration.source, None);

    assert_eq!(
        declaration.specifiers,
        vec![UseSpecifier::Named {
            name: "viewKit".into(),
            span: Span::new(4, 11),
        },],
    );
}

#[test]
fn parses_wildcard_use_declaration() {
    let module = parse("use *").unwrap();

    let Declaration::Use(declaration) = &module.declarations[0] else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.specifiers,
        vec![UseSpecifier::Wildcard {
            span: Span::new(4, 5),
        },],
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

    let Declaration::Use(declaration) = &module.declarations[2] else {
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

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.span, Span::new(0, source.len()),);

    assert_eq!(component.name, "App");
    assert!(component.params.is_empty());
    assert!(component.attributes.is_empty());
    assert!(matches!(
        &component.body,
        Some(body) if body.is_empty()
    ));
}

#[test]
fn parses_component_with_attribute() {
    let source = r#"@application
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.span, Span::new(0, source.len()),);

    assert_eq!(component.attributes.len(), 1);
    assert_eq!(component.attributes[0].span, Span::new(0, 12),);
    assert_eq!(component.attributes[0].name, "application",);
}

#[test]
fn parses_component_parameter_with_default() {
    let source = r#"component App(title: String = "Kome") {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.params.len(), 1);

    let parameter = &component.params[0];

    assert_eq!(parameter.span, Span::new(14, 36),);

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
        Some(Expression::Literal(LiteralExpression {
            span: Span::new(30, 36),
            kind: LiteralKind::String("Kome".into(),),
        },)),
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

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.params.len(), 3);

    assert_eq!(component.params[0].span, Span::new(17, 30),);

    assert_eq!(component.params[1].span, Span::new(32, 49),);

    assert_eq!(component.params[2].span, Span::new(51, 74),);
}

#[test]
fn parses_named_component_parameter_type() {
    let source = "component App(content: View) {}";
    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
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
    let source = "component Stack(spacing: Spacing = .large) {}";

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(
        component.params[0].default,
        Some(Expression::DotIdent(DotIdentifierExpression {
            span: Span::new(35, 41),
            name: "large".into(),
        },)),
    );
}

#[test]
fn parses_component_state_and_let_bindings() {
    let source = r#"component App() {
    state name = "world"
    state counter: Number = 0
    let title: String = "Kome"
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    assert_eq!(body.len(), 3);

    let ComponentMember::State(name) = &body[0] else {
        panic!("expected state binding");
    };

    assert_eq!(name.span, Span::new(22, 42));
    assert!(!name.mutable);
    assert!(name.attributes.is_empty());
    assert_eq!(name.type_annotation, None);

    assert_eq!(
        name.pattern,
        Pattern::Ident(IdentifierPattern {
            span: Span::new(28, 32),
            name: "name".into(),
            type_annotation: None,
            default: None,
        }),
    );

    assert_eq!(
        name.init,
        Some(Expression::Literal(LiteralExpression {
            span: Span::new(35, 42),
            kind: LiteralKind::String("world".into(),),
        },)),
    );

    let ComponentMember::State(counter) = &body[1] else {
        panic!("expected state binding");
    };

    assert_eq!(counter.span, Span::new(47, 72),);

    assert_eq!(
        counter.type_annotation,
        Some(Type::Primitive(PrimitiveType {
            span: Span::new(62, 68),
            kind: PrimitiveTypeKind::Number,
        })),
    );

    assert_eq!(
        counter.init,
        Some(Expression::Literal(LiteralExpression {
            span: Span::new(71, 72),
            kind: LiteralKind::Number(NumberLiteral("0".into()),),
        },)),
    );

    let ComponentMember::Let(title) = &body[2] else {
        panic!("expected let binding");
    };

    assert_eq!(title.span, Span::new(77, 103),);

    assert!(!title.mutable);

    assert_eq!(
        title.type_annotation,
        Some(Type::Primitive(PrimitiveType {
            span: Span::new(88, 94),
            kind: PrimitiveTypeKind::String,
        })),
    );

    assert_eq!(
        &source[title.span.start..title.span.end],
        r#"let title: String = "Kome""#,
    );
}

#[test]
fn parses_mutable_let_binding() {
    let source = r#"component App() {
    let mut count = 1
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Let(binding) = &body[0] else {
        panic!("expected let binding");
    };

    assert_eq!(binding.span, Span::new(22, 39),);

    assert!(binding.mutable);

    assert_eq!(
        binding.pattern,
        Pattern::Ident(IdentifierPattern {
            span: Span::new(30, 35),
            name: "count".into(),
            type_annotation: None,
            default: None,
        }),
    );
}

#[test]
fn parses_attributed_let_binding() {
    let source = r#"component App() {
    @body
    let body: View = root
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Let(binding) = &body[0] else {
        panic!("expected let binding");
    };

    assert_eq!(binding.span, Span::new(22, 53),);

    assert_eq!(binding.attributes.len(), 1);

    assert_eq!(binding.attributes[0].span, Span::new(22, 27),);

    assert_eq!(binding.attributes[0].name, "body",);

    assert_eq!(
        binding.type_annotation,
        Some(Type::Named(NamedType {
            span: Span::new(42, 46),
            name: "View".into(),
            type_arguments: Vec::new(),
        })),
    );

    assert_eq!(
        binding.init,
        Some(Expression::ident("root", Span::new(49, 53),)),
    );
}

#[test]
fn parses_binding_without_initializer() {
    let source = r#"component App() {
    let title: String
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Let(binding) = &body[0] else {
        panic!("expected let binding");
    };

    assert_eq!(binding.init, None);

    assert_eq!(
        binding.type_annotation,
        Some(Type::Primitive(PrimitiveType {
            span: Span::new(33, 39),
            kind: PrimitiveTypeKind::String,
        })),
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
fn rejects_parameter_without_type_annotation() {
    let error = parse("component App(title) {}").unwrap_err();

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
    let error = parse("component App(title: String = ) {}").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an expression",
            found: TokenKind::RParen,
        },
    );
}

#[test]
fn rejects_missing_binding_name() {
    let error = parse("component App() { state = 1 }").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a binding name",
            found: TokenKind::Assign,
        },
    );
}

#[test]
fn rejects_missing_binding_initializer() {
    let error = parse("component App() { state value = }").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an expression",
            found: TokenKind::RBrace,
        },
    );
}

#[test]
fn rejects_attribute_without_component_member() {
    let error = parse("component App() { @body }").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a component member after attributes",
            found: TokenKind::RBrace,
        },
    );
}
