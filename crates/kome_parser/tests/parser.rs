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
    assert_eq!(module.span.start, 0);
    assert_eq!(module.span.end, 0);
}

#[test]
fn parses_named_use_declaration() {
    let source = "use viewKit";
    let module = parse(source).unwrap();

    assert_eq!(module.span.start, 0);
    assert_eq!(module.span.end, source.len());
    assert_eq!(module.declarations.len(), 1);

    let Declaration::Use(declaration) = &module.declarations[0] else {
        panic!("expected use declaration");
    };

    assert_eq!(declaration.span.start, 0);
    assert_eq!(declaration.span.end, source.len());
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

    assert_eq!(
        &source[22..33],
        "collections",
    );

    assert_eq!(
        &source[35..37],
        "io",
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
fn rejects_unsupported_top_level_declaration() {
    let error = parse("fn main() {}").unwrap_err();

    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a top-level declaration",
            found: TokenKind::Fn,
        },
    );
}