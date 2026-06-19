use kome_ast::Span;
use kome_parser::token::TemplateTokenPart;
use kome_parser::{LexErrorKind, Lexer, Token, TokenKind, tokenize};

#[test]
fn tokenizes_component_source() {
    let source = r#"@application
component App() {
    state ratio = 50%
    let mut count = 1

    fn add(value: Number) -> Number {
        return count + value
    }
}"#;

    let tokens = Lexer::new(source).tokenize().unwrap();

    let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::At,
            TokenKind::Ident("application".into()),
            TokenKind::Component,
            TokenKind::Ident("App".into()),
            TokenKind::LParen,
            TokenKind::RParen,
            TokenKind::LBrace,
            TokenKind::State,
            TokenKind::Ident("ratio".into()),
            TokenKind::Assign,
            TokenKind::Percent("50".into()),
            TokenKind::Let,
            TokenKind::Mut,
            TokenKind::Ident("count".into()),
            TokenKind::Assign,
            TokenKind::Number("1".into()),
            TokenKind::Fn,
            TokenKind::Ident("add".into()),
            TokenKind::LParen,
            TokenKind::Ident("value".into()),
            TokenKind::Colon,
            TokenKind::Ident("Number".into()),
            TokenKind::RParen,
            TokenKind::ThinArrow,
            TokenKind::Ident("Number".into()),
            TokenKind::LBrace,
            TokenKind::Return,
            TokenKind::Ident("count".into()),
            TokenKind::Plus,
            TokenKind::Ident("value".into()),
            TokenKind::RBrace,
            TokenKind::RBrace,
            TokenKind::Eof,
        ],
    );
}

#[test]
fn skips_comments_and_whitespace() {
    let source = "let value = 1 // ignored\nvalue += 2";

    let tokens = Lexer::new(source).tokenize().unwrap();

    let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Let,
            TokenKind::Ident("value".into()),
            TokenKind::Assign,
            TokenKind::Number("1".into()),
            TokenKind::Ident("value".into()),
            TokenKind::PlusAssign,
            TokenKind::Number("2".into()),
            TokenKind::Eof,
        ],
    );
}

#[test]
fn tokenizes_numbers_and_percentages() {
    let source = "10 12.5 50% 0.25%";

    let tokens = Lexer::new(source).tokenize().unwrap();

    assert_eq!(
        tokens,
        vec![
            Token::new(TokenKind::Number("10".into()), Span::new(0, 2),),
            Token::new(TokenKind::Number("12.5".into()), Span::new(3, 7),),
            Token::new(TokenKind::Percent("50".into()), Span::new(8, 11),),
            Token::new(TokenKind::Percent("0.25".into()), Span::new(12, 17),),
            Token::eof(17),
        ],
    );
}

#[test]
fn decodes_string_escapes() {
    let source = r#""hello\n\"kome\"""#;

    let tokens = Lexer::new(source).tokenize().unwrap();

    assert_eq!(
        tokens[0],
        Token::new(
            TokenKind::String("hello\n\"kome\"".into()),
            Span::new(0, source.len()),
        ),
    );
}

#[test]
fn lexes_template_string() {
    let source = r#""Hello, {name}""#;

    let tokens = tokenize(source).unwrap();

    assert_eq!(tokens.len(), 2);

    let TokenKind::Template(parts) = &tokens[0].kind else {
        panic!("expected template token, got {:?}", tokens[0].kind);
    };

    assert_eq!(tokens[0].span, Span::new(0, source.len()),);

    assert_eq!(parts.len(), 2);

    assert!(matches!(
        &parts[0],
        TemplateTokenPart::String {
            value,
            span,
        } if value == "Hello, "
            && *span == Span::new(1, 8)
    ));

    let TemplateTokenPart::Expression {
        tokens: expression_tokens,
        span,
    } = &parts[1]
    else {
        panic!("expected template expression part");
    };

    assert_eq!(*span, Span::new(8, 14),);

    assert_eq!(
        expression_tokens,
        &[
            Token::new(TokenKind::Ident("name".into()), Span::new(9, 13),),
            Token::eof(13),
        ],
    );

    assert_eq!(tokens[1], Token::eof(source.len()),);
}

#[test]
fn keeps_utf8_byte_spans() {
    let source = "let 名前 = \"米\"";

    let tokens = Lexer::new(source).tokenize().unwrap();

    assert_eq!(tokens[1].kind, TokenKind::Ident("名前".into()),);

    assert_eq!(&source[tokens[1].span.start..tokens[1].span.end], "名前",);

    assert_eq!(&source[tokens[3].span.start..tokens[3].span.end], "\"米\"",);
}

#[test]
fn tokenizes_multi_character_operators() {
    let source = "a == b != c <= d >= e && f || g += h -> i => j";

    let tokens = Lexer::new(source).tokenize().unwrap();

    let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident("a".into()),
            TokenKind::Eq,
            TokenKind::Ident("b".into()),
            TokenKind::NotEq,
            TokenKind::Ident("c".into()),
            TokenKind::Lte,
            TokenKind::Ident("d".into()),
            TokenKind::Gte,
            TokenKind::Ident("e".into()),
            TokenKind::And,
            TokenKind::Ident("f".into()),
            TokenKind::Or,
            TokenKind::Ident("g".into()),
            TokenKind::PlusAssign,
            TokenKind::Ident("h".into()),
            TokenKind::ThinArrow,
            TokenKind::Ident("i".into()),
            TokenKind::FatArrow,
            TokenKind::Ident("j".into()),
            TokenKind::Eof,
        ],
    );
}

#[test]
fn rejects_single_ampersand() {
    let error = Lexer::new("a & b").tokenize().unwrap_err();

    assert_eq!(error.kind, LexErrorKind::UnexpectedCharacter('&'),);

    assert_eq!(error.span, Span::new(2, 3),);
}

#[test]
fn rejects_invalid_escape_sequence() {
    let error = Lexer::new(r#""hello\q""#).tokenize().unwrap_err();

    assert_eq!(error.kind, LexErrorKind::InvalidEscape('q'),);

    assert_eq!(error.span, Span::new(6, 8),);
}

#[test]
fn rejects_unterminated_string() {
    let source = "\"hello";

    let error = Lexer::new(source).tokenize().unwrap_err();

    assert_eq!(error.kind, LexErrorKind::UnterminatedString,);

    assert_eq!(error.span, Span::new(0, source.len()),);
}

#[test]
fn lexes_question_mark() {
    let tokens = tokenize("String?").unwrap();

    assert_eq!(
        tokens,
        vec![
            Token::new(TokenKind::Ident("String".into(),), Span::new(0, 6),),
            Token::new(TokenKind::Question, Span::new(6, 7),),
            Token::eof(7),
        ],
    );
}

#[test]
fn tokenizes_extension_declaration() {
    let source = r#"
extension View {
    fn padding(value: Number) -> View
}
"#;

    let tokens = Lexer::new(source).tokenize().unwrap();

    let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Extension,
            TokenKind::Ident("View".into()),
            TokenKind::LBrace,
            TokenKind::Fn,
            TokenKind::Ident("padding".into()),
            TokenKind::LParen,
            TokenKind::Ident("value".into()),
            TokenKind::Colon,
            TokenKind::Ident("Number".into()),
            TokenKind::RParen,
            TokenKind::ThinArrow,
            TokenKind::Ident("View".into()),
            TokenKind::RBrace,
            TokenKind::Eof,
        ],
    );
}
