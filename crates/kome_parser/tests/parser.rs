use kome_ast::{
    AstNode, Span,
    declarations::{Declaration, UseSpecifier},
};

use kome_parser::{
    FrontendError, ParseErrorKind, TokenKind, parse,
};

#[test]
fn parses_empty_module() {
    let module = parse("").unwrap();

    assert!(module.declarations.is_empty());
    assert_eq!(module.span, Span::new(0, 0));
}

#[test]
fn parses_named_use_declaration() {
    let source = "use viewKit";
    let module = parse(source).unwrap();

    assert_eq!(module.span, Span::new(0, source.len()));
    assert_eq!(module.declarations.len(), 1);

    let Declaration::Use(declaration) = &module.declarations[0] else {
        panic!("expected use declaration");
    };

    assert_eq!(declaration.span, Span::new(0, source.len()));
    assert_eq!(declaration.source, None);

    assert_eq!(
        declaration.specifiers,
        vec![UseSpecifier::Named {
            name: "viewKit".into(),
            span: Span::new(4, 11),
        }],
    );
}

#[test]
fn parses_wildcard_use_declaration() {
    let module = parse("use *").unwrap();

    let Declaration::Use(declaration) = &module.declarations[0] else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.specifiers,
        vec![UseSpecifier::Wildcard {
            span: Span::new(4, 5),
        }],
    );
}

#[test]
fn parses_multiple_use_declarations() {
    let source = r#"use viewKit
use *
use collections, io"#;

    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 3);
    assert_eq!(module.span().end, source.len());

    let Declaration::Use(declaration) = &module.declarations[2] else {
        panic!("expected use declaration");
    };

    assert_eq!(
        declaration.specifiers,
        vec![
            UseSpecifier::Named {
                name: "collections".into(),
                span: Span::new(22, 33),
            },
            UseSpecifier::Named {
                name: "io".into(),
                span: Span::new(35, 37),
            },
        ],
    );

    assert_eq!(&source[22..33], "collections");
    assert_eq!(&source[35..37], "io");
}

#[test]
fn parses_empty_component() {
    let source = "component App() {}";
    let module = parse(source).unwrap();

    assert_eq!(module.declarations.len(), 1);

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.span, Span::new(0, source.len()));
    assert_eq!(component.name, "App");
    assert!(component.params.is_empty());
    assert!(component.attributes.is_empty());
    assert!(component.body.is_empty());
}

#[test]
fn parses_component_with_attribute() {
    let source = r#"@application
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.span, Span::new(0, source.len()));
    assert_eq!(component.name, "App");
    assert_eq!(component.attributes.len(), 1);

    let attribute = &component.attributes[0];

    assert_eq!(attribute.span, Span::new(0, 12));
    assert_eq!(attribute.name, "application");
    assert!(attribute.args.is_empty());

    assert_eq!(
        &source[attribute.span.start..attribute.span.end],
        "@application",
    );
}

#[test]
fn parses_component_with_multiple_attributes() {
    let source = r#"@application
@preview
component App() {}"#;

    let module = parse(source).unwrap();

    let Declaration::Component(component) = &module.declarations[0] else {
        panic!("expected component declaration");
    };

    assert_eq!(component.attributes.len(), 2);

    assert_eq!(component.attributes[0].name, "application");
    assert_eq!(component.attributes[1].name, "preview");

    assert_eq!(
        &source[
            component.attributes[0].span.start
                ..component.attributes[0].span.end
        ],
        "@application",
    );

    assert_eq!(
        &source[
            component.attributes[1].span.start
                ..component.attributes[1].span.end
        ],
        "@preview",
    );
}

#[test]
fn rejects_missing_use_specifier() {
    let error = parse("use").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "an import name or `*`",
            found: TokenKind::Eof,
        },
    );
}

#[test]
fn rejects_attribute_without_declaration() {
    let error = parse("@application").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a declaration after attributes",
            found: TokenKind::Eof,
        },
    );
}

#[test]
fn rejects_attribute_on_use_declaration() {
    let error = parse("@application use viewKit").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a component or function declaration after attributes",
            found: TokenKind::Use,
        },
    );
}

#[test]
fn rejects_component_parameters_for_now() {
    let error = parse("component App(title: String) {}").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`)`",
            found: TokenKind::Ident("title".into()),
        },
    );
}

#[test]
fn rejects_component_members_for_now() {
    let error = parse("component App() { state counter = 0 }").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "`}`",
            found: TokenKind::State,
        },
    );
}