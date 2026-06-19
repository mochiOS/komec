use kome_ast::{
    AstNode,
    declarations::Declaration,
    expressions::{
        Expression, LiteralKind,
    },
    types::{
        PrimitiveTypeKind, Type,
    },
};

use kome_parser::parse;

#[test]
fn parses_bodyless_component() {
    let module = parse(
        r#"
        component Text(
            content: String,
            size: Number = 16,
        )
        "#,
    )
        .unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.name, "Text");
    assert_eq!(component.params.len(), 2);
    assert!(component.body.is_none());
}

#[test]
fn parses_attributed_bodyless_component() {
    let module = parse(
        r#"
        @nativeComponent("Text")
        component Text(
            content: String,
        )
        "#,
    )
        .unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component declaration");
    };

    assert_eq!(component.name, "Text");
    assert!(component.body.is_none());

    assert_eq!(
        component.attributes.len(),
        1,
    );

    assert_eq!(
        component.attributes[0].name,
        "nativeComponent",
    );

    assert!(matches!(
        &component.attributes[0].args[0],
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "Text".into()
                )
    ));
}

#[test]
fn distinguishes_empty_body_from_no_body() {
    let module = parse(
        r#"
        component NativeText()

        component Empty() {}
        "#,
    )
        .unwrap();

    let Declaration::Component(native) =
        &module.declarations[0]
    else {
        panic!("expected first component");
    };

    let Declaration::Component(empty) =
        &module.declarations[1]
    else {
        panic!("expected second component");
    };

    assert!(native.body.is_none());

    assert!(matches!(
        &empty.body,
        Some(body) if body.is_empty()
    ));
}

#[test]
fn parses_declaration_after_bodyless_component() {
    let module = parse(
        r#"
        component Text(
            content: String,
        )

        enum Color {
            primary,
            secondary,
        }
        "#,
    )
        .unwrap();

    assert_eq!(module.declarations.len(), 2);

    assert!(matches!(
        &module.declarations[0],
        Declaration::Component(component)
            if component.body.is_none()
    ));

    assert!(matches!(
        &module.declarations[1],
        Declaration::Enum(_)
    ));
}

#[test]
fn bodyless_component_span_ends_at_parenthesis() {
    let source =
        "component Text(content: String)";

    let module = parse(source).unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component");
    };

    assert_eq!(
        component.span().start,
        0,
    );

    assert_eq!(
        component.span().end,
        source.len(),
    );
}

#[test]
fn bodyless_component_preserves_parameters() {
    let module = parse(
        r#"
        component Button(
            title: String,
            enabled: Boolean = true,
        )
        "#,
    )
        .unwrap();

    let Declaration::Component(component) =
        &module.declarations[0]
    else {
        panic!("expected component");
    };

    assert_eq!(component.params.len(), 2);

    assert_eq!(
        component.params[0].name,
        "title",
    );

    assert!(matches!(
        &component.params[0].type_,
        Type::Primitive(primitive)
            if primitive.kind
                == PrimitiveTypeKind::String
    ));

    assert_eq!(
        component.params[1].name,
        "enabled",
    );

    assert!(component.params[1]
        .default
        .is_some());

    assert!(component.body.is_none());
}