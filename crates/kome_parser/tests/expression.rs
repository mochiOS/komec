use kome_ast::expressions::NumberLiteral;
use kome_ast::{
    AstNode, Span,
    declarations::{ComponentMember, Declaration},
    expressions::{AssignOp, BinaryOp, CallArg, Expression, LiteralKind, UnaryOp},
};
use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse, parse_expression};

#[test]
fn parses_binary_operator_precedence() {
    let expression = parse_expression("a + b * c").unwrap();

    let Expression::Binary(addition) = expression else {
        panic!("expected addition expression");
    };

    assert_eq!(addition.op, BinaryOp::Add);
    assert_eq!(addition.span, Span::new(0, 9));

    assert!(matches!(
        addition.left.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "a"
    ));

    let Expression::Binary(multiplication) = addition.right.as_ref() else {
        panic!("expected multiplication expression");
    };

    assert_eq!(multiplication.op, BinaryOp::Mul);
    assert_eq!(multiplication.span, Span::new(4, 9));

    assert!(matches!(
        multiplication.left.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "b"
    ));

    assert!(matches!(
        multiplication.right.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "c"
    ));
}

#[test]
fn parses_logical_and_comparison_precedence() {
    let expression = parse_expression("a > 0 && b < 10 || enabled").unwrap();

    let Expression::Binary(or) = expression else {
        panic!("expected or expression");
    };

    assert_eq!(or.op, BinaryOp::Or);

    let Expression::Binary(and) = or.left.as_ref() else {
        panic!("expected and expression");
    };

    assert_eq!(and.op, BinaryOp::And);

    let Expression::Binary(greater) = and.left.as_ref() else {
        panic!("expected comparison expression");
    };

    assert_eq!(greater.op, BinaryOp::Gt);

    let Expression::Binary(less) = and.right.as_ref() else {
        panic!("expected comparison expression");
    };

    assert_eq!(less.op, BinaryOp::Lt);
}

#[test]
fn parses_right_associative_assignment() {
    let expression = parse_expression("a = b = 1").unwrap();

    let Expression::Assign(first) = expression else {
        panic!("expected assignment expression");
    };

    assert_eq!(first.op, AssignOp::Assign);

    assert!(matches!(
        first.target.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "a"
    ));

    let Expression::Assign(second) = first.value.as_ref() else {
        panic!("expected nested assignment expression");
    };

    assert_eq!(second.op, AssignOp::Assign);

    assert!(matches!(
        second.target.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "b"
    ));

    assert!(matches!(
        second.value.as_ref(),
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::Number(NumberLiteral("1".into()))
    ));
}

#[test]
fn parses_add_assignment() {
    let expression = parse_expression("counter += 1").unwrap();

    let Expression::Assign(assignment) = expression else {
        panic!("expected assignment expression");
    };

    assert_eq!(assignment.op, AssignOp::AddAssign);
    assert_eq!(assignment.span, Span::new(0, 12));
}

#[test]
fn parses_unary_not() {
    let expression = parse_expression("!!enabled").unwrap();

    let Expression::Unary(first) = expression else {
        panic!("expected unary expression");
    };

    assert_eq!(first.op, UnaryOp::Not);
    assert_eq!(first.span, Span::new(0, 9));

    let Expression::Unary(second) = first.argument.as_ref() else {
        panic!("expected nested unary expression");
    };

    assert_eq!(second.op, UnaryOp::Not);
    assert_eq!(second.span, Span::new(1, 9));
}

#[test]
fn parses_group_expression() {
    let expression = parse_expression("(a + b) * c").unwrap();

    let Expression::Binary(multiplication) = expression else {
        panic!("expected multiplication expression");
    };

    assert_eq!(multiplication.op, BinaryOp::Mul);

    let Expression::Group(group) = multiplication.left.as_ref() else {
        panic!("expected group expression");
    };

    assert_eq!(group.span, Span::new(0, 7));

    let Expression::Binary(addition) = group.expression.as_ref() else {
        panic!("expected addition expression");
    };

    assert_eq!(addition.op, BinaryOp::Add);
}

#[test]
fn parses_function_call() {
    let expression =
        parse_expression(r#"Button("increment", color: .blue, onClick: handle_click)"#).unwrap();

    let Expression::Call(call) = expression else {
        panic!("expected call expression");
    };

    assert_eq!(call.args.len(), 3);

    assert!(matches!(
        call.callee.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "Button"
    ));

    assert!(matches!(
        &call.args[0],
        CallArg::Positional(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::String("increment".into())
    ));

    let CallArg::Named { name, value, .. } = &call.args[1] else {
        panic!("expected named argument");
    };

    assert_eq!(name, "color");

    assert!(matches!(
        value.as_ref(),
        Expression::DotIdent(identifier)
            if identifier.name == "blue"
    ));

    let CallArg::Named { name, value, .. } = &call.args[2] else {
        panic!("expected named argument");
    };

    assert_eq!(name, "onClick");

    assert!(matches!(
        value.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "handle_click"
    ));
}

#[test]
fn parses_chained_postfix_expression() {
    let expression = parse_expression("items[index].title.trim()").unwrap();

    let Expression::Call(call) = expression else {
        panic!("expected call expression");
    };

    let Expression::Member(trim) = call.callee.as_ref() else {
        panic!("expected trim member expression");
    };

    assert_eq!(trim.property, "trim");

    let Expression::Member(title) = trim.object.as_ref() else {
        panic!("expected title member expression");
    };

    assert_eq!(title.property, "title");

    let Expression::Index(index) = title.object.as_ref() else {
        panic!("expected index expression");
    };

    assert!(matches!(
        index.object.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "items"
    ));

    assert!(matches!(
        index.index.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "index"
    ));
}

#[test]
fn parses_list_expression() {
    let expression = parse_expression("[1, value, .large,]").unwrap();

    let Expression::List(list) = expression else {
        panic!("expected list expression");
    };

    assert_eq!(list.elems.len(), 3);
    assert_eq!(list.span, Span::new(0, 19));

    assert!(matches!(
        &list.elems[0],
        Some(Expression::Literal(literal))
            if literal.kind
                == LiteralKind::Number(NumberLiteral("1".into()))
    ));

    assert!(matches!(
        &list.elems[1],
        Some(Expression::Ident(identifier))
            if identifier.name == "value"
    ));

    assert!(matches!(
        &list.elems[2],
        Some(Expression::DotIdent(identifier))
            if identifier.name == "large"
    ));
}

#[test]
fn parses_general_expression_in_state_binding() {
    let source = r#"component App() {
    state counter = initial + step * 2
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component.body.as_ref().expect("expected component body");

    let ComponentMember::State(binding) = &body[0] else {
        panic!("expected state member");
    };

    let Some(Expression::Binary(addition)) = &binding.init else {
        panic!("expected addition initializer");
    };

    assert_eq!(addition.op, BinaryOp::Add);

    let Expression::Binary(multiplication) = addition.right.as_ref() else {
        panic!("expected multiplication expression");
    };

    assert_eq!(multiplication.op, BinaryOp::Mul);
}

#[test]
fn parses_expression_as_component_default() {
    let source = "component App(count: Number = initial + 1) {}";

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let Some(Expression::Binary(addition)) = &component.params[0].default else {
        panic!("expected addition default value");
    };

    assert_eq!(addition.op, BinaryOp::Add);
}

#[test]
fn rejects_incomplete_binary_expression() {
    let error = parse_expression("value +").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
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
fn rejects_missing_call_argument() {
    let error = parse_expression("call(value,) extra").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "the end of the expression",
            found: TokenKind::Ident("extra".into()),
        },
    );
}

#[test]
fn expression_span_covers_source() {
    let source = "items[index].title.trim()";

    let expression = parse_expression(source).unwrap();

    assert_eq!(expression.span(), Span::new(0, source.len()),);
}
