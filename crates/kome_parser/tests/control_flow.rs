use kome_ast::{
    expressions::{Expression, LiteralKind},
    patterns::{IsPattern, Pattern},
    statements::Statement,
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse_expression};

#[test]
fn parses_for_in_statement() {
    let expression = parse_expression(
        r#"{
            for item in items {
                print(item)
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    assert_eq!(block.statements.len(), 1);

    let Statement::ForIn(statement) = &block.statements[0] else {
        panic!("expected for-in statement");
    };

    let Pattern::Ident(pattern) = &statement.pattern else {
        panic!("expected identifier pattern");
    };

    assert_eq!(pattern.name, "item");

    assert!(matches!(
        &statement.right,
        Expression::Ident(identifier)
            if identifier.name == "items"
    ));

    let Statement::Block(body) = statement.body.as_ref() else {
        panic!("expected for-in body");
    };

    assert_eq!(body.statements.len(), 1);
}

#[test]
fn parses_for_in_call_expression() {
    let expression = parse_expression(
        r#"{
            for item in items.filter(active) {
                render(item)
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::ForIn(statement) = &block.statements[0] else {
        panic!("expected for-in statement");
    };

    assert!(matches!(&statement.right, Expression::Call(_)));
}

#[test]
fn parses_is_statement_with_dot_pattern() {
    let expression = parse_expression(
        r#"{
            is input.event .entered => {
                handle_enter()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Is(statement) = &block.statements[0] else {
        panic!("expected is statement");
    };

    assert!(matches!(
        &statement.value,
        Some(Expression::Member(member))
            if member.property == "event"
    ));

    assert!(matches!(
        &statement.pattern,
        IsPattern::DotIdent(pattern)
            if pattern.name == "entered"
    ));

    let Statement::Block(body) = statement.body.as_ref() else {
        panic!("expected is body");
    };

    assert_eq!(body.statements.len(), 1);
}

#[test]
fn parses_is_statement_with_implicit_value() {
    let expression = parse_expression(
        r#"{
            is .submitted => {
                submit()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Is(statement) = &block.statements[0] else {
        panic!("expected is statement");
    };

    assert!(statement.value.is_none());

    assert!(matches!(
        &statement.pattern,
        IsPattern::DotIdent(pattern)
            if pattern.name == "submitted"
    ));
}

#[test]
fn parses_is_statement_with_literal_pattern() {
    let expression = parse_expression(
        r#"{
            is status "ready" => {
                start()
            }
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Is(statement) = &block.statements[0] else {
        panic!("expected is statement");
    };

    assert!(matches!(
        &statement.value,
        Some(Expression::Ident(identifier))
            if identifier.name == "status"
    ));

    assert!(matches!(
        &statement.pattern,
        IsPattern::Literal(pattern)
            if pattern.value
                == LiteralKind::String("ready".into())
    ));
}

#[test]
fn parses_is_statement_without_braced_body() {
    let expression = parse_expression(
        r#"{
            is .entered => handle()
        }"#,
    )
    .unwrap();

    let Expression::Block(block) = expression else {
        panic!("expected block expression");
    };

    let Statement::Is(statement) = &block.statements[0] else {
        panic!("expected is statement");
    };

    assert!(matches!(statement.body.as_ref(), Statement::Expression(_)));
}

#[test]
fn rejects_for_without_in() {
    let error = parse_expression(
        r#"{
            for item items {
                render(item)
            }
        }"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`in`",
            found: TokenKind::Ident("items".into()),
        },
    );
}

#[test]
fn rejects_for_without_binding() {
    let error = parse_expression(
        r#"{
            for in items {}
        }"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a loop binding",
            found: TokenKind::In,
        },
    );
}

#[test]
fn rejects_is_without_pattern() {
    let error = parse_expression(
        r#"{
            is =>
        }"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an `is` pattern",
            found: TokenKind::FatArrow,
        },
    );
}

#[test]
fn rejects_is_without_arrow() {
    let error = parse_expression(
        r#"{
            is input.event .entered {
                handle()
            }
        }"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`=>`",
            found: TokenKind::RBrace,
        },
    );
}
