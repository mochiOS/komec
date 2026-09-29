use kome_ast::expressions::Expression;

#[test]
fn parses_task_and_wait_expressions() {
    assert!(matches!(
        kome_parser::parse_expression("task foo()").unwrap(),
        Expression::Task(_)
    ));
    assert!(matches!(
        kome_parser::parse_expression("wait pending").unwrap(),
        Expression::Wait(_)
    ));

    let Expression::Wait(wait) = kome_parser::parse_expression("wait task foo()").unwrap() else {
        panic!("expected wait expression")
    };
    assert!(matches!(wait.argument.as_ref(), Expression::Task(_)));
}

#[test]
fn task_and_wait_bind_more_tightly_than_addition() {
    let Expression::Binary(addition) =
        kome_parser::parse_expression("wait task calculate() + 1").unwrap()
    else {
        panic!("expected addition")
    };
    let Expression::Wait(wait) = addition.left.as_ref() else {
        panic!("expected wait on the left")
    };
    assert!(matches!(wait.argument.as_ref(), Expression::Task(_)));
}

#[test]
fn parses_cancel_as_a_prefix_expression() {
    let Expression::Cancel(cancel) = kome_parser::parse_expression("cancel pending").unwrap()
    else {
        panic!("expected cancel expression")
    };
    assert!(matches!(cancel.argument.as_ref(), Expression::Ident(_)));
}

#[test]
fn parses_all_result_list_annotations() {
    let module = kome_parser::parse("fn take(values: Number[]) {}").unwrap();
    let kome_ast::declarations::Declaration::Function(function) = &module.declarations[0] else {
        panic!("expected function")
    };
    let kome_ast::patterns::Pattern::Ident(parameter) = &function.params[0] else {
        panic!("expected identifier parameter")
    };
    assert!(matches!(
        &parameter.type_annotation,
        Some(kome_ast::types::Type::List(_))
    ));
}
