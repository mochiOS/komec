use std::mem;

use kome_ast::Span;

use crate::{
    error::{LexError, LexErrorKind},
    token::{TemplateTokenPart, Token, TokenKind},
};

/// Converts Kome source code into tokens.
pub struct Lexer<'source> {
    source: &'source str,

    /// Offset relative to `source`.
    offset: usize,

    /// Absolute byte offset of `source` in the original file.
    base_offset: usize,
}

impl<'source> Lexer<'source> {
    pub const fn new(source: &'source str) -> Self {
        Self {
            source,
            offset: 0,
            base_offset: 0,
        }
    }

    const fn with_base_offset(source: &'source str, base_offset: usize) -> Self {
        Self {
            source,
            offset: 0,
            base_offset,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            let token = self.next_token()?;
            let is_eof = token.is_eof();

            tokens.push(token);

            if is_eof {
                return Ok(tokens);
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_ignored();

        let start = self.offset;

        let Some(character) = self.advance_char() else {
            return Ok(Token::eof(self.absolute_offset(self.offset)));
        };

        if is_identifier_start(character) {
            return Ok(self.lex_identifier(start));
        }

        if character.is_ascii_digit() {
            return Ok(self.lex_number(start));
        }

        let kind = match character {
            '"' => {
                return self.lex_string(start);
            }

            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,

            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            '@' => TokenKind::At,

            '|' if self.consume_char('|') => TokenKind::Or,

            '|' => TokenKind::Pipe,

            '-' if self.consume_char('>') => TokenKind::ThinArrow,

            '-' => TokenKind::Minus,

            '=' if self.consume_char('=') => TokenKind::Eq,

            '=' if self.consume_char('>') => TokenKind::FatArrow,

            '=' => TokenKind::Assign,

            '+' if self.consume_char('=') => TokenKind::PlusAssign,

            '+' => TokenKind::Plus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,

            '!' if self.consume_char('=') => TokenKind::NotEq,

            '!' => TokenKind::Not,

            '<' if self.consume_char('=') => TokenKind::Lte,

            '<' => TokenKind::Lt,

            '>' if self.consume_char('=') => TokenKind::Gte,

            '>' => TokenKind::Gt,

            '&' if self.consume_char('&') => TokenKind::And,

            unexpected => {
                return Err(LexError::new(
                    LexErrorKind::UnexpectedCharacter(unexpected),
                    self.span(start, self.offset),
                ));
            }
        };

        Ok(Token::new(kind, self.span(start, self.offset)))
    }

    fn lex_identifier(&mut self, start: usize) -> Token {
        while self.peek_char().is_some_and(is_identifier_continue) {
            self.advance_char();
        }

        let identifier = self.source[start..self.offset].to_owned();

        let kind = TokenKind::from_identifier(identifier);

        Token::new(kind, self.span(start, self.offset))
    }

    fn lex_number(&mut self, start: usize) -> Token {
        while self
            .peek_char()
            .is_some_and(|character| character.is_ascii_digit())
        {
            self.advance_char();
        }

        if self.peek_char() == Some('.')
            && self
                .peek_next_char()
                .is_some_and(|character| character.is_ascii_digit())
        {
            self.advance_char();

            while self
                .peek_char()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.advance_char();
            }
        }

        let number_end = self.offset;

        let value = self.source[start..number_end].to_owned();

        let kind = if self.consume_char('%') {
            TokenKind::Percent(value)
        } else {
            TokenKind::Number(value)
        };

        Token::new(kind, self.span(start, self.offset))
    }

    fn lex_string(&mut self, start: usize) -> Result<Token, LexError> {
        let mut text = String::new();
        let mut parts = Vec::new();

        let mut has_interpolation = false;

        // Opening quote has already been consumed.
        let mut text_start = self.offset;

        while let Some(character) = self.advance_char() {
            match character {
                '"' => {
                    let token_span = self.span(start, self.offset);

                    if !has_interpolation {
                        return Ok(Token::new(TokenKind::String(text), token_span));
                    }

                    let closing_quote = self.offset - '"'.len_utf8();

                    self.push_template_text(&mut parts, &mut text, text_start, closing_quote);

                    return Ok(Token::new(TokenKind::Template(parts), token_span));
                }

                '\\' => {
                    let escape_start = self.offset - '\\'.len_utf8();

                    let Some(escaped) = self.advance_char() else {
                        return Err(LexError::new(
                            LexErrorKind::UnterminatedString,
                            self.span(start, self.offset),
                        ));
                    };

                    let escaped = match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',

                        // Literal template braces.
                        '{' => '{',
                        '}' => '}',

                        invalid => {
                            return Err(LexError::new(
                                LexErrorKind::InvalidEscape(invalid),
                                self.span(escape_start, self.offset),
                            ));
                        }
                    };

                    text.push(escaped);
                }

                '{' => {
                    has_interpolation = true;

                    let opening_brace = self.offset - '{'.len_utf8();

                    self.push_template_text(&mut parts, &mut text, text_start, opening_brace);

                    let expression_start = self.offset;

                    let Some(closing_brace) = self.find_interpolation_end(expression_start) else {
                        return Err(LexError::new(
                            LexErrorKind::UnterminatedInterpolation,
                            self.span(opening_brace, self.source.len()),
                        ));
                    };

                    let expression_source = &self.source[expression_start..closing_brace];

                    let expression_tokens = Lexer::with_base_offset(
                        expression_source,
                        self.absolute_offset(expression_start),
                    )
                    .tokenize()?;

                    parts.push(TemplateTokenPart::Expression {
                        tokens: expression_tokens,
                        span: self.span(opening_brace, closing_brace + 1),
                    });

                    self.offset = closing_brace + 1;
                    text_start = self.offset;
                }

                '\n' | '\r' => {
                    return Err(LexError::new(
                        LexErrorKind::UnterminatedString,
                        self.span(start, self.offset),
                    ));
                }

                other => {
                    text.push(other);
                }
            }
        }

        Err(LexError::new(
            LexErrorKind::UnterminatedString,
            self.span(start, self.offset),
        ))
    }

    fn push_template_text(
        &self,
        parts: &mut Vec<TemplateTokenPart>,
        text: &mut String,
        start: usize,
        end: usize,
    ) {
        if text.is_empty() {
            return;
        }

        parts.push(TemplateTokenPart::String {
            value: mem::take(text),
            span: self.span(start, end),
        });
    }

    fn find_interpolation_end(&self, start: usize) -> Option<usize> {
        let mut index = start;
        let mut brace_depth = 1usize;

        let mut in_string = false;
        let mut escaped = false;
        let mut in_line_comment = false;

        while index < self.source.len() {
            if in_line_comment {
                let character = self.source[index..].chars().next()?;

                if matches!(character, '\n' | '\r') {
                    return None;
                }

                index += character.len_utf8();
                continue;
            }

            if in_string {
                let character = self.source[index..].chars().next()?;

                index += character.len_utf8();

                if escaped {
                    escaped = false;
                    continue;
                }

                match character {
                    '\\' => {
                        escaped = true;
                    }

                    '"' => {
                        in_string = false;
                    }

                    '\n' | '\r' => {
                        return None;
                    }

                    _ => {}
                }

                continue;
            }

            if self.source[index..].starts_with("//") {
                in_line_comment = true;
                index += 2;
                continue;
            }

            let character = self.source[index..].chars().next()?;

            match character {
                '"' => {
                    in_string = true;
                }

                '{' => {
                    brace_depth += 1;
                }

                '}' => {
                    brace_depth -= 1;

                    if brace_depth == 0 {
                        return Some(index);
                    }
                }

                '\n' | '\r' => {
                    return None;
                }

                _ => {}
            }

            index += character.len_utf8();
        }

        None
    }

    fn skip_ignored(&mut self) {
        loop {
            while self.peek_char().is_some_and(char::is_whitespace) {
                self.advance_char();
            }

            if self.starts_with("//") {
                while self.peek_char().is_some_and(|character| character != '\n') {
                    self.advance_char();
                }

                continue;
            }

            return;
        }
    }

    fn absolute_offset(&self, local_offset: usize) -> usize {
        self.base_offset + local_offset
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.absolute_offset(start), self.absolute_offset(end))
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn peek_next_char(&self) -> Option<char> {
        let mut characters = self.source[self.offset..].chars();

        characters.next()?;
        characters.next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let character = self.peek_char()?;

        self.offset += character.len_utf8();

        Some(character)
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.peek_char() != Some(expected) {
            return false;
        }

        self.advance_char();
        true
    }

    fn starts_with(&self, expected: &str) -> bool {
        self.source[self.offset..].starts_with(expected)
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}
