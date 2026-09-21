use kome_parser::{FrontendError, ParseErrorKind, TokenKind, parse, tokenize};

#[test]
fn extension_is_no_longer_a_keyword() {
    let tokens = tokenize("extension View {}").unwrap();

    assert_eq!(tokens[0].kind, TokenKind::Ident("extension".into()));
}

#[test]
fn rejects_removed_extension_declaration() {
    let error = parse("extension View {}").unwrap_err();
    let FrontendError::Parse(error) = error else {
        panic!("expected parse error");
    };

    assert_eq!(
        error.kind,
        ParseErrorKind::Expected {
            expected: "a top-level declaration",
            found: TokenKind::Ident("extension".into()),
        }
    );
}
