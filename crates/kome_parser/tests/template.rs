use kome_ast::{
    Span,
    declarations::Declaration,
    expressions::{BinaryOp, Expression, LiteralKind, TemplatePart},
};

use kome_parser::{FrontendError, LexErrorKind, ParseErrorKind, TokenKind, parse, parse_expression, tokenize, Token};

#[test]
fn plain_string_remains_string_literal() {
    let expression = parse_expression(r#""Hello""#).unwrap();

    assert!(matches!(
        expression,
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "Hello".into()
                )
    ));
}

#[test]
fn parses_simple_template() {
    let source = r#""Hello, {name}!""#;

    let expression = parse_expression(source).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert_eq!(template.span, Span::new(0, source.len()),);

    assert_eq!(template.parts.len(), 3);

    assert!(matches!(
        &template.parts[0],
        TemplatePart::String {
            value,
            span,
        } if value == "Hello, "
            && *span == Span::new(1, 8)
    ));

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            span,
        } if matches!(
            expression.as_ref(),
            Expression::Ident(identifier)
                if identifier.name == "name"
                && identifier.span
                    == Span::new(9, 13)
        ) && *span == Span::new(8, 14)
    ));

    assert!(matches!(
        &template.parts[2],
        TemplatePart::String {
            value,
            span,
        } if value == "!"
            && *span == Span::new(14, 15)
    ));
}

#[test]
fn parses_template_expression() {
    let expression = parse_expression(r#""count: {count + 1}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert_eq!(template.parts.len(), 2);

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            ..
        } if matches!(
            expression.as_ref(),
            Expression::Binary(binary)
                if binary.op == BinaryOp::Add
        )
    ));
}

#[test]
fn parses_multiple_interpolations() {
    let expression = parse_expression(r#""{first} + {second} = {result}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert_eq!(
        template
            .parts
            .iter()
            .filter(|part| { matches!(part, TemplatePart::Expression { .. }) })
            .count(),
        3,
    );
}

#[test]
fn parses_member_expression_in_template() {
    let expression = parse_expression(r#""user: {user.name}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            ..
        } if matches!(
            expression.as_ref(),
            Expression::Member(member)
                if member.property == "name"
        )
    ));
}

#[test]
fn parses_call_expression_in_template() {
    let expression = parse_expression(r#""value: {format(value)}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            ..
        } if matches!(
            expression.as_ref(),
            Expression::Call(_)
        )
    ));
}

#[test]
fn parses_string_inside_interpolation() {
    let expression = parse_expression(r#""value: {format("Kome")}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            ..
        } if matches!(
            expression.as_ref(),
            Expression::Call(_)
        )
    ));
}

#[test]
fn parses_nested_object_in_interpolation() {
    let expression = parse_expression(r#""config: {format({name: "Kome"})}""#).unwrap();

    let Expression::Template(template) = expression else {
        panic!("expected template expression");
    };

    assert!(matches!(
        &template.parts[1],
        TemplatePart::Expression {
            expression,
            ..
        } if matches!(
            expression.as_ref(),
            Expression::Call(_)
        )
    ));
}

#[test]
fn escaped_braces_remain_plain_string() {
    let expression = parse_expression(r#""literal: \{name\}""#).unwrap();

    assert!(matches!(
        expression,
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "literal: {name}".into()
                )
    ));
}

#[test]
fn template_can_initialize_top_level_let() {
    let module = parse(r#"let message = "Hello, {name}!""#).unwrap();

    let Declaration::Let(binding) = &module.declarations[0] else {
        panic!("expected top-level let");
    };

    assert!(matches!(&binding.init, Some(Expression::Template(_))));
}

#[test]
fn interpolation_tokens_keep_absolute_spans() {
    let source = r#""Hello, {user.name}!""#;

    let tokens = tokenize(source).unwrap();

    let TokenKind::Template(parts) = &tokens[0].kind else {
        panic!("expected template token");
    };

    let kome_parser::token::TemplateTokenPart::Expression { tokens, span } = &parts[1] else {
        panic!("expected expression part");
    };

    assert_eq!(*span, Span::new(8, 19));

    assert_eq!(tokens[0].span, Span::new(9, 13),);

    assert_eq!(tokens[1].span, Span::new(13, 14),);

    assert_eq!(tokens[2].span, Span::new(14, 18),);
}

#[test]
fn rejects_unterminated_interpolation() {
    let error = parse_expression(r#""Hello, {name""#).unwrap_err();

    let FrontendError::Lex(error) = error else {
        panic!("expected lexer error");
    };

    assert_eq!(error.kind, LexErrorKind::UnterminatedInterpolation,);
}

#[test]
fn rejects_empty_interpolation() {
    let error = parse_expression(r#""Hello, {}""#).unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parser error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an expression",
            found: TokenKind::Eof,
        },
    );
}

#[test]
fn preserves_escaped_template_braces_in_string_token() {
    let source = r#""Hello, \{name\}""#;

    let tokens = tokenize(source).unwrap();

    assert_eq!(
        tokens,
        vec![
            Token::new(
                TokenKind::String(
                    "Hello, {name}".into(),
                ),
                Span::new(0, source.len()),
            ),
            Token::eof(source.len()),
        ],
    );
}