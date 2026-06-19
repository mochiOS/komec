use kome_ast::{
    declarations::Declaration,
    expressions::{
        BinaryOp, Expression,
    },
    patterns::Pattern,
    statements::Statement,
    types::{
        PrimitiveTypeKind, Type,
    },
};

use kome_parser::{
    FrontendError, ParseErrorKind, TokenKind,
    parse, parse_expression,
};

#[test]
fn parses_single_parameter_closure() {
    let expression =
        parse_expression(
            "|value| value * 2",
        )
            .unwrap();

    let Expression::Closure(closure) =
        expression
    else {
        panic!("expected closure expression");
    };

    assert_eq!(closure.params.len(), 1);

    let Pattern::Ident(parameter) =
        &closure.params[0]
    else {
        panic!("expected identifier parameter");
    };

    assert_eq!(parameter.name, "value");
    assert!(parameter.type_annotation.is_none());
    assert!(parameter.default.is_none());

    let Expression::Binary(body) =
        closure.body.as_ref()
    else {
        panic!("expected binary body");
    };

    assert_eq!(body.op, BinaryOp::Mul);
}

#[test]
fn parses_multiple_parameter_closure() {
    let expression =
        parse_expression(
            "|left, right| left + right",
        )
            .unwrap();

    let Expression::Closure(closure) =
        expression
    else {
        panic!("expected closure expression");
    };

    assert_eq!(closure.params.len(), 2);

    let Pattern::Ident(left) =
        &closure.params[0]
    else {
        panic!("expected left parameter");
    };

    let Pattern::Ident(right) =
        &closure.params[1]
    else {
        panic!("expected right parameter");
    };

    assert_eq!(left.name, "left");
    assert_eq!(right.name, "right");

    assert!(matches!(
        closure.body.as_ref(),
        Expression::Binary(binary)
            if binary.op == BinaryOp::Add
    ));
}

#[test]
fn parses_typed_closure_parameter() {
    let expression =
        parse_expression(
            "|value: Number| value * 2",
        )
            .unwrap();

    let Expression::Closure(closure) =
        expression
    else {
        panic!("expected closure expression");
    };

    let Pattern::Ident(parameter) =
        &closure.params[0]
    else {
        panic!("expected identifier parameter");
    };

    assert!(matches!(
        &parameter.type_annotation,
        Some(Type::Primitive(primitive))
            if primitive.kind
                == PrimitiveTypeKind::Number
    ));
}

#[test]
fn parses_closure_with_block_body() {
    let expression = parse_expression(
        r#"|value| {
            let doubled = value * 2
            doubled
        }"#,
    )
        .unwrap();

    let Expression::Closure(closure) =
        expression
    else {
        panic!("expected closure expression");
    };

    let Expression::Block(body) =
        closure.body.as_ref()
    else {
        panic!("expected block body");
    };

    assert_eq!(body.statements.len(), 1);

    assert!(matches!(
        &body.statements[0],
        Statement::Let(_)
    ));

    assert!(matches!(
        body.tail.as_deref(),
        Some(Expression::Ident(identifier))
            if identifier.name == "doubled"
    ));
}

#[test]
fn parses_closure_as_call_argument() {
    let expression = parse_expression(
        "items.map(|item| item.name)",
    )
        .unwrap();

    let Expression::Call(call) = expression else {
        panic!("expected call expression");
    };

    assert_eq!(call.args.len(), 1);

    assert!(matches!(
        &call.args[0],
        kome_ast::expressions::CallArg::Positional(
            Expression::Closure(closure)
        ) if closure.params.len() == 1
    ));
}

#[test]
fn parses_closure_as_top_level_initializer() {
    let module = parse(
        "let double = |value| value * 2",
    )
        .unwrap();

    let Declaration::Let(binding) =
        &module.declarations[0]
    else {
        panic!("expected top-level let");
    };

    assert!(matches!(
        &binding.init,
        Some(Expression::Closure(_))
    ));
}

#[test]
fn parses_nested_closure() {
    let expression = parse_expression(
        "|value| |other| value + other",
    )
        .unwrap();

    let Expression::Closure(outer) =
        expression
    else {
        panic!("expected outer closure");
    };

    let Expression::Closure(inner) =
        outer.body.as_ref()
    else {
        panic!("expected inner closure");
    };

    assert_eq!(outer.params.len(), 1);
    assert_eq!(inner.params.len(), 1);
}

#[test]
fn rejects_closure_without_parameter() {
    let error = parse_expression(
        "| | value",
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a closure parameter",
            found: TokenKind::Pipe,
        },
    );
}

#[test]
fn rejects_closure_without_closing_pipe() {
    let error = parse_expression(
        "|value value * 2",
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "the closing `|`",
            found: TokenKind::Ident(
                "value".into(),
            ),
        },
    );
}

#[test]
fn rejects_trailing_parameter_comma() {
    let error = parse_expression(
        "|left,| left",
    )
        .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected:
                "a closure parameter after `,`",
            found: TokenKind::Pipe,
        },
    );
}

#[test]
fn logical_or_remains_binary_expression() {
    let expression =
        parse_expression("left || right")
            .unwrap();

    assert!(matches!(
        expression,
        Expression::Binary(binary)
            if binary.op == BinaryOp::Or
    ));
}