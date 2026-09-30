use kome_ast::Span;
use tower_lsp::lsp_types::{Position, Range};

/// Converts a byte-based Kome span into an LSP UTF-16 range.
pub fn span_to_range(source: &str, span: Span) -> Range {
    let span = normalize_span(source, span);

    Range {
        start: byte_offset_to_position(source, span.start),
        end: byte_offset_to_position(source, span.end),
    }
}

/// Converts a UTF-8 byte offset into an LSP UTF-16 position.
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

/// Converts an LSP UTF-16 position into a UTF-8 byte offset.
pub fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let mut line_start = 0usize;

    for _ in 0..position.line {
        let newline = source[line_start..].find('\n')?;

        line_start += newline + 1;
    }

    let line_end = source[line_start..]
        .find('\n')
        .map(|offset| line_start + offset)
        .unwrap_or(source.len());

    let line = &source[line_start..line_end];
    let mut utf16_offset = 0u32;

    for (byte_offset, character) in line.char_indices() {
        if utf16_offset >= position.character {
            return Some(line_start + byte_offset);
        }

        let next = utf16_offset + character.len_utf16() as u32;

        if position.character < next {
            return Some(line_start + byte_offset);
        }

        utf16_offset = next;
    }

    if position.character <= utf16_offset {
        Some(line_end)
    } else {
        None
    }
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
