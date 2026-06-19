use kome_ast::{
    AstNode,
    declarations::{Declaration, ExtensionMember},
    expressions::{Expression, LiteralKind},
    statements::Statement,
    types::Type,
};

use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse};

#[test]
fn parses_empty_extension() {
    let module = parse("extension View {}").unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    assert!(matches!(
        &extension.target,
        Type::Named(named)
            if named.name == "View"
    ));

    assert!(extension.members.is_empty());
}

#[test]
fn parses_bodyless_extension_function() {
    let module = parse(
        r#"
        extension View {
            fn padding(
                value: Number,
            ) -> View
        }
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    assert_eq!(extension.members.len(), 1);

    let ExtensionMember::Function(function) = &extension.members[0];

    assert_eq!(function.name, "padding");
    assert_eq!(function.params.len(), 1);
    assert!(function.body.is_none());

    assert!(matches!(
        &function.return_type,
        Some(Type::Named(named))
            if named.name == "View"
    ));
}

#[test]
fn parses_native_modifier_function() {
    let module = parse(
        r#"
        extension View {
            @nativeModifier("Padding")
            fn padding(
                value: Number,
            ) -> View
        }
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    let ExtensionMember::Function(function) = &extension.members[0];

    assert_eq!(function.name, "padding");
    assert_eq!(function.attributes.len(), 1);

    let attribute = &function.attributes[0];

    assert_eq!(attribute.name, "nativeModifier",);

    assert_eq!(attribute.args.len(), 1);

    assert!(matches!(
        &attribute.args[0],
        Expression::Literal(literal)
            if literal.kind
                == LiteralKind::String(
                    "Padding".into()
                )
    ));
}

#[test]
fn parses_multiple_extension_functions() {
    let module = parse(
        r#"
        extension View {
            fn padding(
                value: Number,
            ) -> View

            fn opacity(
                value: Number,
            ) -> View
        }
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    assert_eq!(extension.members.len(), 2);

    let ExtensionMember::Function(padding) = &extension.members[0];

    let ExtensionMember::Function(opacity) = &extension.members[1];

    assert_eq!(padding.name, "padding");
    assert_eq!(opacity.name, "opacity");
}

#[test]
fn parses_extension_function_with_body() {
    let module = parse(
        r#"
        extension View {
            fn standardPadding() -> View {
                return padding(16)
            }
        }
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    let ExtensionMember::Function(function) = &extension.members[0];

    let body = function.body.as_ref().expect("expected function body");

    assert_eq!(body.statements.len(), 1);

    assert!(matches!(&body.statements[0], Statement::Return(_)));
}

#[test]
fn parses_generic_extension_target() {
    let module = parse(
        r#"
        extension List<Item> {
            fn first() -> Item?
        }
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    let Type::Named(target) = &extension.target else {
        panic!("expected named target type");
    };

    assert_eq!(target.name, "List");
    assert_eq!(target.type_arguments.len(), 1);

    assert!(matches!(
        &target.type_arguments[0],
        Type::Named(named)
            if named.name == "Item"
    ));
}

#[test]
fn parses_attributed_extension() {
    let module = parse(
        r#"
        @available
        extension View {}
        "#,
    )
    .unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    assert_eq!(extension.attributes.len(), 1);

    assert_eq!(extension.attributes[0].name, "available",);
}

#[test]
fn extension_span_includes_body() {
    let source = "extension View { fn padding() }";

    let module = parse(source).unwrap();

    let Declaration::Extension(extension) = &module.declarations[0] else {
        panic!("expected extension declaration");
    };

    assert_eq!(extension.span().start, 0);
    assert_eq!(extension.span().end, source.len(),);
}

#[test]
fn rejects_non_function_extension_member() {
    let error = parse(
        r#"
        extension View {
            let value = 1
        }
        "#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a function declaration in an extension",
            found: TokenKind::Let,
        },
    );
}

#[test]
fn rejects_extension_member_attribute_without_function() {
    let error = parse(
        r#"
        extension View {
            @nativeModifier("Padding")
            let value = 1
        }
        "#,
    )
    .unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a function declaration after extension member attributes",
            found: TokenKind::Let,
        },
    );
}

#[test]
fn rejects_unclosed_extension() {
    let error = parse(
        r#"
        extension View {
            fn padding() -> View
        "#,
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
