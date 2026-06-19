use kome_ast::statements::Statement;
use kome_ast::{
    Span,
    declarations::Declaration,
    expressions::{BinaryOp, Expression, LiteralKind, NumberLiteral, ObjectProperty, PropertyKey},
};
use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse, parse_expression};

#[test]
fn parses_object_literal() {
    let source = r#"{
    name: "Kome",
    age: 1,
    enabled: true,
}"#;

    let expression = parse_expression(source).unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    assert_eq!(object.span, Span::new(0, source.len()),);

    assert_eq!(object.props.len(), 3);

    let ObjectProperty::KeyValue(name) = &object.props[0];

    assert!(matches!(
        &name.key,
        PropertyKey::Ident {
            name,
            ..
        } if name == "name"
    ));

    assert!(matches!(
        name.value.as_ref(),
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "Kome".into()
                )
    ));

    let ObjectProperty::KeyValue(age) = &object.props[1];

    assert!(matches!(
        &age.key,
        PropertyKey::Ident {
            name,
            ..
        } if name == "age"
    ));

    assert!(matches!(
        age.value.as_ref(),
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::Number(
                    NumberLiteral("1".into())
                )
    ));

    let ObjectProperty::KeyValue(enabled) = &object.props[2];

    assert!(matches!(
        enabled.value.as_ref(),
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::Boolean(true)
    ));
}

#[test]
fn parses_string_property_key() {
    let expression = parse_expression(r#"{"display-name": "Kome"}"#).unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    let ObjectProperty::KeyValue(property) = &object.props[0];

    assert!(matches!(
        &property.key,
        PropertyKey::String {
            value,
            ..
        } if value == "display-name"
    ));
}

#[test]
fn parses_number_property_key() {
    let expression = parse_expression(r#"{1: "first"}"#).unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    let ObjectProperty::KeyValue(property) = &object.props[0];

    assert!(matches!(
        &property.key,
        PropertyKey::Number {
            value,
            ..
        } if value == "1"
    ));
}

#[test]
fn parses_computed_property_key() {
    let expression = parse_expression(r#"{[prefix + suffix]: value}"#).unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    let ObjectProperty::KeyValue(property) = &object.props[0];

    let PropertyKey::Computed { expression, .. } = &property.key else {
        panic!("expected computed property key");
    };

    assert!(matches!(
        expression.as_ref(),
        Expression::Binary(binary)
            if binary.op == BinaryOp::Add
    ));

    assert!(matches!(
        property.value.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "value"
    ));
}

#[test]
fn parses_nested_object_literal() {
    let expression = parse_expression(
        r#"{
            settings: {
                enabled: true,
                size: 16,
            },
        }"#,
    )
    .unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    let ObjectProperty::KeyValue(settings) = &object.props[0];

    let Expression::Object(settings_object) = settings.value.as_ref() else {
        panic!("expected nested object");
    };

    assert_eq!(settings_object.props.len(), 2,);
}

#[test]
fn parses_object_as_top_level_initializer() {
    let source = r#"let config = {
    name: "Kome",
    enabled: true,
}"#;

    let module = parse(source).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert!(matches!(
        &binding.init,
        Some(Expression::Object(object))
            if object.props.len() == 2
    ));
}

#[test]
fn parses_object_value_expression() {
    let expression = parse_expression(r#"{size: base * 2 + padding}"#).unwrap();

    let Expression::Object(object) = expression else {
        panic!("expected object expression");
    };

    let ObjectProperty::KeyValue(property) = &object.props[0];

    let Expression::Binary(addition) = property.value.as_ref() else {
        panic!("expected addition expression");
    };

    assert_eq!(addition.op, BinaryOp::Add);

    assert!(matches!(
        addition.left.as_ref(),
        Expression::Binary(multiplication)
            if multiplication.op == BinaryOp::Mul
    ));
}

#[test]
fn empty_braces_remain_block_expression() {
    let expression = parse_expression("{}").unwrap();

    assert!(matches!(expression, Expression::Block(_)));
}

#[test]
fn braces_without_colon_remain_block_expression() {
    let expression = parse_expression(r#"{name "Kome"}"#).unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.statements.len(), 1);

    assert!(matches!(
        &block.statements[0],
        Statement::Expression(statement)
            if matches!(
                &statement.expression,
                Expression::Ident(identifier)
                    if identifier.name == "name"
            )
    ));

    assert!(matches!(
        block.tail.as_deref(),
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::String("Kome".into())
    ));
}

#[test]
fn rejects_missing_property_comma() {
    let error = parse_expression(r#"{name: "Kome" age: 1}"#).unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`,` or `}` after an object property",
            found: TokenKind::Ident("age".into()),
        },
    );
}

#[test]
fn rejects_unclosed_object_literal() {
    let error = parse_expression(r#"{name: "Kome""#).unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`,` or `}` after an object property",
            found: TokenKind::Eof,
        },
    );
}
