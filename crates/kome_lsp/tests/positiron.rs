use kome_ast::Span;
use kome_lsp::position::{byte_offset_to_position, span_to_range};
use tower_lsp::lsp_types::{Position, Range};

#[test]
fn converts_ascii_byte_offset() {
    let source = "hello\nworld";

    assert_eq!(
        byte_offset_to_position(source, 8),
        Position {
            line: 1,
            character: 2,
        },
    );
}

#[test]
fn converts_utf8_offset_to_utf16_position() {
    let source = "猫🐈x";
    let offset = "猫🐈".len();

    assert_eq!(
        byte_offset_to_position(source, offset),
        Position {
            line: 0,
            character: 3,
        },
    );
}

#[test]
fn converts_multiline_span() {
    let source = "猫\nhello";

    assert_eq!(
        span_to_range(source, Span::new("猫\n".len(), source.len(),),),
        Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 5,
            },
        },
    );
}

#[test]
fn expands_empty_eof_span() {
    let source = "enum Color {";

    assert_eq!(
        span_to_range(source, Span::new(source.len(), source.len(),),),
        Range {
            start: Position {
                line: 0,
                character: 11,
            },
            end: Position {
                line: 0,
                character: 12,
            },
        },
    );
}

#[test]
fn expands_empty_span_after_multibyte_character() {
    let source = "猫";

    assert_eq!(
        span_to_range(source, Span::new(source.len(), source.len(),),),
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 1,
            },
        },
    );
}

#[test]
fn leaves_empty_source_at_zero() {
    assert_eq!(
        span_to_range("", Span::new(0, 0),),
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        },
    );
}

#[test]
fn clamps_offset_inside_utf8_character() {
    let source = "猫x";

    assert_eq!(
        byte_offset_to_position(source, 2),
        Position {
            line: 0,
            character: 0,
        },
    );
}
