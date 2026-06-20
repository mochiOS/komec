use kome_ast::Span;
use tower_lsp::lsp_types::{Position, Range};

pub fn span_to_range(source: &str, span: Span) -> Range {
    let span = normalize_span(source, span);

    Range {
        start: byte_offset_to_position(source, span.start),
        end: byte_offset_to_position(source, span.end),
    }
}

pub fn byte_offset_to_position(source: &str, byte_offset: usize) -> Position {
    let byte_offset = clamp_to_char_boundary(source, byte_offset);

    let mut line = 0_u32;
    let mut character = 0_u32;

    for (offset, current) in source.char_indices() {
        if offset >= byte_offset {
            break;
        }

        if current == '\n' {
            line += 1;
            character = 0;
        } else {
            character += current.len_utf16() as u32;
        }
    }

    Position { line, character }
}

fn normalize_span(source: &str, span: Span) -> Span {
    let start = clamp_to_char_boundary(source, span.start);
    let end = clamp_to_char_boundary(source, span.end);

    if start < end {
        return Span::new(start, end);
    }

    if start == 0 {
        return Span::new(0, 0);
    }

    let previous = previous_char_boundary(source, start);

    Span::new(previous, start)
}

fn clamp_to_char_boundary(source: &str, byte_offset: usize) -> usize {
    let mut offset = byte_offset.min(source.len());

    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }

    offset
}

fn previous_char_boundary(source: &str, byte_offset: usize) -> usize {
    let offset = clamp_to_char_boundary(source, byte_offset);

    source[..offset]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}
