use kome_ast::{
    Span,
    declarations::{ComponentMember, Declaration},
    expressions::{AssignOp, Expression},
    statements::Statement,
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse};

#[test]
fn parses_empty_recipe() {
    let source = r#"component App() {
    recipe initialize {}
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };


    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    assert_eq!(body.len(), 1);

    let ComponentMember::Recipe(recipe) = &body[0] else {
        panic!("expected recipe declaration");
    };

    assert_eq!(recipe.name, "initialize");
    assert_eq!(recipe.event_source, None);
    assert!(recipe.attributes.is_empty());
    assert!(recipe.body.statements.is_empty());

    assert_eq!(
        &source[recipe.span.start..recipe.span.end],
        "recipe initialize {}",
    );
}

#[test]
fn parses_recipe_body() {
    let source = r#"component App() {
    recipe update {
        counter += 1
        render()
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Recipe(recipe) = &body[0] else {
        panic!("expected recipe declaration");
    };

    assert_eq!(recipe.name, "update");
    assert_eq!(recipe.body.statements.len(), 2);

    let Statement::Expression(statement) = &recipe.body.statements[0] else {
        panic!("expected assignment statement");
    };

    let Expression::Assign(assignment) = &statement.expression else {
        panic!("expected assignment expression");
    };

    assert_eq!(assignment.op, AssignOp::AddAssign,);

    assert!(matches!(
        &recipe.body.statements[1],
        Statement::Expression(statement)
            if matches!(
                &statement.expression,
                Expression::Call(_)
            )
    ));
}

#[test]
fn parses_recipe_event_source() {
    let source = r#"component App() {
    recipe submit: input {
        print(input.value)
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Recipe(recipe) = &body[0] else {
        panic!("expected recipe declaration");
    };

    assert_eq!(recipe.name, "submit");

    assert_eq!(recipe.event_source.as_deref(), Some("input"),);

    assert_eq!(recipe.body.statements.len(), 1,);
}

#[test]
fn parses_attributed_recipe() {
    let source = r#"component App() {
    @startup
    @once
    recipe initialize {
        load()
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Recipe(recipe) = &body[0] else {
        panic!("expected recipe declaration");
    };

    assert_eq!(recipe.attributes.len(), 2);

    assert_eq!(recipe.attributes[0].name, "startup",);

    assert_eq!(recipe.attributes[1].name, "once",);

    assert_eq!(recipe.span.start, recipe.attributes[0].span.start,);

    assert_eq!(recipe.span.end, recipe.body.span.end,);
}

#[test]
fn parses_multiple_recipes() {
    let source = r#"component App() {
    recipe load {
        fetch()
    }

    recipe submit: input {
        save(input.value)
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    assert_eq!(body.len(), 2);

    assert!(matches!(
        &body[0],
        ComponentMember::Recipe(recipe)
            if recipe.name == "load"
    ));

    assert!(matches!(
        &body[1],
        ComponentMember::Recipe(recipe)
            if recipe.name == "submit"
    ));
}

#[test]
fn rejects_recipe_without_name() {
    let error = parse(
        r#"component App() {
    recipe {}
}"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a recipe name",
            found: TokenKind::LBrace,
        },
    );
}

#[test]
fn rejects_recipe_without_body() {
    let error = parse(
        r#"component App() {
    recipe update
}"#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`{`",
            found: TokenKind::RBrace,
        },
    );
}

#[test]
fn rejects_recipe_without_event_source() {
    let error = parse(
        r#"component App() {
    recipe submit: {
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
            expected: "an event source after `:`",
            found: TokenKind::LBrace,
        },
    );
}

#[test]
fn recipe_span_includes_attribute_and_body() {
    let source = r#"component App() {
    @startup
    recipe initialize {
        load()
    }
}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    let body = component
        .body
        .as_ref()
        .expect("expected component body");

    let ComponentMember::Recipe(recipe) = &body[0] else {
        panic!("expected recipe declaration");
    };

    let recipe_source = &source[recipe.span.start..recipe.span.end];

    assert!(recipe_source.starts_with("@startup"));
    assert!(recipe_source.ends_with('}'));
    assert_eq!(
        recipe.span,
        Span::new(recipe.attributes[0].span.start, recipe.body.span.end,),
    );
}
