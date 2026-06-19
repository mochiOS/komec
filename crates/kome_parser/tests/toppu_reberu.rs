use kome_ast::{
    Span,
    declarations::Declaration,
    expressions::{BinaryOp, Expression, LiteralKind, NumberLiteral},
    patterns::Pattern,
    types::{PrimitiveType, PrimitiveTypeKind, Type},
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse};

#[test]
fn parses_top_level_let() {
    let source = "let version: Number = 1";
    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 1);

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert!(!binding.mutable);

    let Pattern::Ident(pattern) = &binding.pattern else {
        panic!("expected identifier pattern");
    };

    assert_eq!(pattern.name, "version");

    assert_eq!(
        binding.type_annotation,
        Some(Type::Primitive(PrimitiveType {
            span: Span::new(13, 19),
            kind: PrimitiveTypeKind::Number,
        })),
    );

    assert!(matches!(
        &binding.init,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::Number(
                    NumberLiteral("1".into())
                )
    ));

    assert_eq!(binding.span, Span::new(0, source.len()),);
}

#[test]
fn parses_mutable_top_level_let() {
    let source = "let mut counter = 0";
    let module = parse(source).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert!(binding.mutable);

    let Pattern::Ident(pattern) = &binding.pattern else {
        panic!("expected identifier pattern");
    };

    assert_eq!(pattern.name, "counter");

    assert!(matches!(
        &binding.init,
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::Number(
                    NumberLiteral("0".into())
                )
    ));
}

#[test]
fn parses_top_level_let_without_initializer() {
    let source = "let applicationName: String";
    let module = parse(source).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert!(!binding.mutable);
    assert!(binding.init.is_none());

    assert!(matches!(
        &binding.type_annotation,
        Some(Type::Primitive(primitive))
            if primitive.kind
                == PrimitiveTypeKind::String
    ));
}

#[test]
fn parses_top_level_let_expression() {
    let source = "let size = 8 * 2 + 4";
    let module = parse(source).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    let Some(Expression::Binary(addition)) = &binding.init else {
        panic!("expected binary initializer");
    };

    assert_eq!(addition.op, BinaryOp::Add);

    assert!(matches!(
        addition.left.as_ref(),
        Expression::Binary(multiplication)
            if multiplication.op == BinaryOp::Mul
    ));
}

#[test]
fn parses_multiple_top_level_bindings() {
    let source = r#"
let name = "Kome"
let mut launchCount = 0
let version: Number = 1
"#;

    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 3);

    assert!(matches!(
        &module.declarations[0],
        Declaration::Let(binding)
            if !binding.mutable
    ));

    assert!(matches!(
        &module.declarations[1],
        Declaration::Let(binding)
            if binding.mutable
    ));

    assert!(matches!(
        &module.declarations[2],
        Declaration::Let(binding)
            if !binding.mutable
    ));
}

#[test]
fn parses_attributed_top_level_let() {
    let source = r#"@export("applicationName")
let applicationName = "Kome""#;

    let module = parse(source).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert_eq!(binding.attributes.len(), 1);

    let attribute = &binding.attributes[0];

    assert_eq!(attribute.name, "export");
    assert_eq!(attribute.args.len(), 1);

    assert!(matches!(
        &attribute.args[0],
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "applicationName".into()
                )
    ));

    assert_eq!(binding.span.start, 0);
    assert_eq!(binding.span.end, source.len());
}

#[test]
fn parses_attribute_without_arguments() {
    let source = r#"@application
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.attributes.len(), 1);

    let attribute = &component.attributes[0];

    assert_eq!(attribute.name, "application");
    assert!(attribute.args.is_empty());

    assert_eq!(
        &source[attribute.span.start..attribute.span.end],
        "@application",
    );
}

#[test]
fn parses_empty_attribute_arguments() {
    let source = r#"@generated()
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let attribute = &component.attributes[0];

    assert_eq!(attribute.name, "generated");
    assert!(attribute.args.is_empty());

    assert_eq!(
        &source[attribute.span.start..attribute.span.end],
        "@generated()",
    );
}

#[test]
fn parses_attribute_arguments() {
    let source = r#"@nativeComponent("Text", 1 + 2)
component Text() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let attribute = &component.attributes[0];

    assert_eq!(attribute.name, "nativeComponent");
    assert_eq!(attribute.args.len(), 2);

    assert!(matches!(
        &attribute.args[0],
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String("Text".into())
    ));

    assert!(matches!(
        &attribute.args[1],
        Expression::Binary(expression)
            if expression.op == BinaryOp::Add
    ));

    assert_eq!(
        &source[attribute.span.start..attribute.span.end],
        r#"@nativeComponent("Text", 1 + 2)"#,
    );
}

#[test]
fn parses_attribute_trailing_comma() {
    let source = r#"@available(
    "linux",
    "mochiOS",
)
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let attribute = &component.attributes[0];

    assert_eq!(attribute.name, "available");
    assert_eq!(attribute.args.len(), 2);
}

#[test]
fn parses_multiple_attributes_with_arguments() {
    let source = r#"@nativeComponent("Text")
@available("linux")
component Text() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.attributes.len(), 2);

    assert_eq!(component.attributes[0].name, "nativeComponent",);

    assert_eq!(component.attributes[1].name, "available",);

    assert_eq!(component.attributes[0].args.len(), 1,);

    assert_eq!(component.attributes[1].args.len(), 1,);
}

#[test]
fn rejects_attribute_without_closing_parenthesis() {
    let error = parse(
        r#"@nativeComponent("Text"
component Text() {}"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`)`",
            found: TokenKind::Component,
        },
    );
}

#[test]
fn rejects_missing_attribute_argument() {
    let error = parse(
        r#"@nativeComponent(,)
component Text() {}"#,
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
