use kome_ast::{
    declarations::{ComponentMember, Declaration},
    expressions::{BinaryOp, Expression},
    statements::Statement,
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse, parse_expression};

#[test]
fn parses_let_statement_and_block_tail() {
    let expression = parse_expression(
        r#"{
            let value = 1
            value + 2
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.statements.len(), 1);

    let Statement::Let(binding) = &block.statements[0] else {
        panic!("expected let statement");
    };

    assert!(binding.init.is_some());

    let Some(tail) = &block.tail else {
        panic!("expected block tail");
    };

    let Expression::Binary(binary) = tail.as_ref() else {
        panic!("expected binary tail expression");
    };

    assert_eq!(binary.op, BinaryOp::Add);
}

#[test]
fn parses_if_statement() {
    let expression = parse_expression(
        r#"{
            if enabled {
                counter += 1
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.statements.len(), 1);
    assert!(block.tail.is_none());

    let Statement::If(statement) = &block.statements[0] else {
        panic!("expected if statement");
    };

    assert!(matches!(
        &statement.test,
        Expression::Ident(identifier)
            if identifier.name == "enabled"
    ));

    let Statement::Block(consequent) = statement.consequent.as_ref() else {
        panic!("expected consequent block");
    };

    assert_eq!(consequent.statements.len(), 1);

    assert!(matches!(
        &consequent.statements[0],
        Statement::Expression(
            expression_statement
        ) if matches!(
            &expression_statement.expression,
            Expression::Assign(_)
        )
    ));
}

#[test]
fn parses_if_else_statement() {
    let expression = parse_expression(
        r#"{
            if enabled {
                show()
            } else {
                hide()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::If(statement) = &block.statements[0] else {
        panic!("expected if statement");
    };

    assert!(statement.alternative.is_some());

    let Some(alternative) = &statement.alternative else {
        panic!("expected alternative");
    };

    let Statement::Block(alternative) = alternative.as_ref() else {
        panic!("expected alternative block");
    };

    assert_eq!(alternative.statements.len(), 1);
}

#[test]
fn parses_else_if_statement() {
    let expression = parse_expression(
        r#"{
            if first {
                select_first()
            } else if second {
                select_second()
            } else {
                select_other()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::If(first) = &block.statements[0] else {
        panic!("expected first if statement");
    };

    let Some(alternative) = &first.alternative else {
        panic!("expected else-if");
    };

    let Statement::If(second) = alternative.as_ref() else {
        panic!("expected nested if statement");
    };

    assert!(matches!(
        &second.test,
        Expression::Ident(identifier)
            if identifier.name == "second"
    ));

    assert!(second.alternative.is_some());
}

#[test]
fn parses_while_statement_with_break() {
    let expression = parse_expression(
        r#"{
            while running {
                update()
                break
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::While(statement) = &block.statements[0] else {
        panic!("expected while statement");
    };

    let Statement::Block(body) = statement.body.as_ref() else {
        panic!("expected while block");
    };

    assert_eq!(body.statements.len(), 2);

    assert!(matches!(&body.statements[0], Statement::Expression(_)));

    assert!(matches!(&body.statements[1], Statement::Break(_)));
}

#[test]
fn parses_continue_statement() {
    let expression = parse_expression(
        r#"{
            while running {
                continue
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::While(statement) = &block.statements[0] else {
        panic!("expected while statement");
    };

    let Statement::Block(body) = statement.body.as_ref() else {
        panic!("expected while block");
    };

    assert!(matches!(&body.statements[0], Statement::Continue(_)));
}

#[test]
fn parses_return_with_value() {
    let expression = parse_expression(
        r#"{
            return value + 1
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Return(statement) = &block.statements[0] else {
        panic!("expected return statement");
    };

    assert!(matches!(
        &statement.argument,
        Some(Expression::Binary(binary))
            if binary.op == BinaryOp::Add
    ));
}

#[test]
fn parses_return_without_value() {
    let expression = parse_expression(
        r#"{
            return
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Return(statement) = &block.statements[0] else {
        panic!("expected return statement");
    };

    assert!(statement.argument.is_none());
}

#[test]
fn parses_if_statement_in_body_binding() {
    let source = r#"component App() {
    @body
    let body: View = {
        if enabled {
            Text("enabled")
        }

        Text("body")
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component.body.as_ref().expect("expected component body");

    let ComponentMember::Let(binding) = &body[0] else {
        panic!("expected let member");
    };

    let Some(Expression::Block(block)) = &binding.init else {
        panic!("expected block initializer");
    };

    assert_eq!(block.statements.len(), 1);
    assert!(matches!(&block.statements[0], Statement::If(_)));

    assert!(matches!(block.tail.as_deref(), Some(Expression::Call(_))));
}

#[test]
fn condition_does_not_consume_statement_block_as_component() {
    let expression = parse_expression(
        r#"{
            if enabled {
                run()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::If(statement) = &block.statements[0] else {
        panic!("expected if statement");
    };

    assert!(matches!(
        &statement.test,
        Expression::Ident(identifier)
            if identifier.name == "enabled"
    ));
}

#[test]
fn rejects_else_without_block_or_if() {
    let error = parse_expression(
        r#"{
            if enabled {
                run()
            } else value
        }"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`if` or `{` after `else`",
            found: TokenKind::Ident("value".into()),
        },
    );
}

#[test]
fn rejects_unclosed_statement_block() {
    let error = parse_expression(
        r#"{
            while running {
                update()
        }"#,
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
