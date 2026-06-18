use kome_ast::{
    AstNode, Span,
    declarations::{ComponentMember, Declaration},
    expressions::{
        BinaryOp, CallArg, Expression, LiteralKind,
    },
    statements::Statement,
};

use kome_parser::{
    FrontendError, ParseErrorKind, TokenKind, parse,
    parse_expression,
};

#[test]
fn parses_empty_block_expression() {
    let expression =
        parse_expression("{}").unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.span, Span::new(0, 2));
    assert!(block.statements.is_empty());
    assert_eq!(block.tail, None);
}

#[test]
fn parses_block_expression_with_tail() {
    let expression =
        parse_expression("{ value + 1 }").unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert!(block.statements.is_empty());

    let Some(tail) = block.tail else {
        panic!("expected tail expression");
    };

    let Expression::Binary(addition) = tail.as_ref() else {
        panic!("expected addition expression");
    };

    assert_eq!(addition.op, BinaryOp::Add);
}

#[test]
fn parses_block_expression_with_multiple_expressions() {
    let expression = parse_expression(
        "{ prepare() render() }",
    )
        .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.statements.len(), 1);

    let Statement::Expression(statement) =
        &block.statements[0]
    else {
        panic!("expected expression statement");
    };

    assert!(matches!(
        &statement.expression,
        Expression::Call(call)
            if matches!(
                call.callee.as_ref(),
                Expression::Ident(identifier)
                    if identifier.name == "prepare"
            )
    ));

    let Some(tail) = &block.tail else {
        panic!("expected tail expression");
    };

    assert!(matches!(
        tail.as_ref(),
        Expression::Call(call)
            if matches!(
                call.callee.as_ref(),
                Expression::Ident(identifier)
                    if identifier.name == "render"
            )
    ));
}

#[test]
fn parses_component_expression_without_arguments() {
    let source =
        r#"VStack { Text("Hello") }"#;

    let expression =
        parse_expression(source).unwrap();

    let Expression::Component(component) = expression else {
        panic!("expected component expression");
    };

    assert_eq!(component.span, Span::new(0, source.len()));
    assert_eq!(component.name, "VStack");
    assert!(component.args.is_empty());
    assert_eq!(component.children.len(), 1);

    let Expression::Call(text) =
        &component.children[0]
    else {
        panic!("expected Text call");
    };

    assert!(matches!(
        text.callee.as_ref(),
        Expression::Ident(identifier)
            if identifier.name == "Text"
    ));

    assert!(matches!(
        &text.args[0],
        CallArg::Positional(
            Expression::Literal(literal)
        ) if literal.kind
            == LiteralKind::String("Hello".into())
    ));
}

#[test]
fn parses_component_expression_with_arguments() {
    let source = r#"VStack(
    spacing: .large,
    alignment: .center,
) {
    Text("Hello")
    Button("increment")
}"#;

    let expression =
        parse_expression(source).unwrap();

    let Expression::Component(component) = expression else {
        panic!("expected component expression");
    };

    assert_eq!(component.name, "VStack");
    assert_eq!(component.args.len(), 2);
    assert_eq!(component.children.len(), 2);

    let CallArg::Named {
        name,
        value,
        ..
    } = &component.args[0]
    else {
        panic!("expected named argument");
    };

    assert_eq!(name, "spacing");

    assert!(matches!(
        value.as_ref(),
        Expression::DotIdent(identifier)
            if identifier.name == "large"
    ));

    let CallArg::Named {
        name,
        value,
        ..
    } = &component.args[1]
    else {
        panic!("expected named argument");
    };

    assert_eq!(name, "alignment");

    assert!(matches!(
        value.as_ref(),
        Expression::DotIdent(identifier)
            if identifier.name == "center"
    ));
}

#[test]
fn parses_nested_component_expression() {
    let source = r#"VStack {
    HStack {
        Text("left")
        Text("right")
    }
}"#;

    let expression =
        parse_expression(source).unwrap();

    let Expression::Component(vstack) = expression else {
        panic!("expected VStack component");
    };

    assert_eq!(vstack.children.len(), 1);

    let Expression::Component(hstack) =
        &vstack.children[0]
    else {
        panic!("expected HStack component");
    };

    assert_eq!(hstack.name, "HStack");
    assert_eq!(hstack.children.len(), 2);
}

#[test]
fn parses_component_modifier_chain() {
    let source =
        r#"VStack { Text("Hello") }.padding(24)"#;

    let expression =
        parse_expression(source).unwrap();

    assert_eq!(
        expression.span(),
        Span::new(0, source.len()),
    );

    let Expression::Call(call) = expression else {
        panic!("expected padding call");
    };

    let Expression::Member(member) =
        call.callee.as_ref()
    else {
        panic!("expected padding member");
    };

    assert_eq!(member.property, "padding");

    let Expression::Component(component) =
        member.object.as_ref()
    else {
        panic!("expected component expression");
    };

    assert_eq!(component.name, "VStack");
    assert_eq!(component.children.len(), 1);
}

#[test]
fn parses_body_binding_with_component_tree() {
    let source = r#"@application
component App() {
    @body
    let body: View = {
        VStack(
            spacing: .large,
            alignment: .center,
        ) {
            Text("Hello")
            Button("increment")
        }
        .padding(24)
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.attributes.len(), 1);
    assert_eq!(component.body.len(), 1);

    let ComponentMember::Let(binding) =
        &component.body[0]
    else {
        panic!("expected body binding");
    };

    assert_eq!(binding.attributes.len(), 1);
    assert_eq!(binding.attributes[0].name, "body");

    let Some(Expression::Block(block)) = &binding.init else {
        panic!("expected block initializer");
    };

    assert!(block.statements.is_empty());

    let Some(tail) = &block.tail else {
        panic!("expected block tail");
    };

    let Expression::Call(padding_call) =
        tail.as_ref()
    else {
        panic!("expected padding call");
    };

    let Expression::Member(padding_member) =
        padding_call.callee.as_ref()
    else {
        panic!("expected padding member");
    };

    assert_eq!(padding_member.property, "padding");

    let Expression::Component(vstack) =
        padding_member.object.as_ref()
    else {
        panic!("expected VStack component");
    };

    assert_eq!(vstack.name, "VStack");
    assert_eq!(vstack.args.len(), 2);
    assert_eq!(vstack.children.len(), 2);
}

#[test]
fn rejects_unclosed_component_children() {
    let error =
        parse_expression("VStack { Text(\"Hello\")")
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
fn rejects_unclosed_block_expression() {
    let error =
        parse_expression("{ value + 1")
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