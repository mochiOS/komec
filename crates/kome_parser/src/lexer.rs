use kome_ast::Span;

use crate::{
    error::{LexError, LexErrorKind},
    token::{Token, TokenKind},
};

/// Converts Kome source code into tokens.
pub struct Lexer<'source> {
    source: &'source str,
    offset: usize,
}

impl<'source> Lexer<'source> {
    pub const fn new(source: &'source str) -> Self {
        Self { source, offset: 0 }
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
            return Ok(Token::eof(self.offset));
        };

        if is_identifier_start(character) {
            return Ok(self.lex_identifier(start));
        }

        if character.is_ascii_digit() {
            return Ok(self.lex_number(start));
        }

        let kind = match character {
            '"' => return self.lex_string(start),

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
                    Span::new(start, self.offset),
                ));
            }
        };

        Ok(Token::new(kind, Span::new(start, self.offset)))
    }

    fn lex_identifier(&mut self, start: usize) -> Token {
        while self.peek_char().is_some_and(is_identifier_continue) {
            self.advance_char();
        }

        let identifier = self.source[start..self.offset].to_owned();
        let kind = TokenKind::from_identifier(identifier);

        Token::new(kind, Span::new(start, self.offset))
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

        Token::new(kind, Span::new(start, self.offset))
    }

    fn lex_string(&mut self, start: usize) -> Result<Token, LexError> {
        let mut value = String::new();

        while let Some(character) = self.advance_char() {
            match character {
                '"' => {
                    return Ok(Token::new(
                        TokenKind::String(value),
                        Span::new(start, self.offset),
                    ));
                }

                '\\' => {
                    let escape_start = self.offset - '\\'.len_utf8();

                    let Some(escaped) = self.advance_char() else {
                        return Err(LexError::new(
                            LexErrorKind::UnterminatedString,
                            Span::new(start, self.offset),
                        ));
                    };

                    let escaped = match escaped {
                        '"' => '"',
                        '\\' => '\\',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',

                        invalid => {
                            return Err(LexError::new(
                                LexErrorKind::InvalidEscape(invalid),
                                Span::new(escape_start, self.offset),
                            ));
                        }
                    };

                    value.push(escaped);
                }

                '\n' | '\r' => {
                    return Err(LexError::new(
                        LexErrorKind::UnterminatedString,
                        Span::new(start, self.offset),
                    ));
                }

                other => value.push(other),
            }
        }

        Err(LexError::new(
            LexErrorKind::UnterminatedString,
            Span::new(start, self.offset),
        ))
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
