use kome_ast::{
    Span,
    declarations::{
        Declaration, Module, UseDeclaration, UseSpecifier,
    },
};

use crate::{
    error::{ParseError, ParseErrorKind},
    token::{Token, TokenKind},
};

/// Converts Kome tokens into an AST.
pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(mut tokens: Vec<Token>) -> Self {
        if !tokens.last().is_some_and(Token::is_eof) {
            let offset = tokens
                .last()
                .map_or(0, |token| token.span.end);

            tokens.push(Token::eof(offset));
        }

        Self {
            tokens,
            position: 0,
        }
    }

    pub fn parse_module(&mut self) -> Result<Module, ParseError> {
        let mut declarations = Vec::new();

        while !self.current().is_eof() {
            declarations.push(self.parse_declaration()?);
        }

        let end = self.current().span.end;

        Ok(Module::new(
            declarations,
            Span::new(0, end),
        ))
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        match &self.current().kind {
            TokenKind::Use => {
                self.parse_use_declaration()
                    .map(Declaration::Use)
            }

            _ => Err(self.expected("a top-level declaration")),
        }
    }

    fn parse_use_declaration(
        &mut self,
    ) -> Result<UseDeclaration, ParseError> {
        let start = self.advance().span.start;
        let mut specifiers = Vec::new();

        specifiers.push(self.parse_use_specifier()?);

        while self.at(|kind| matches!(kind, TokenKind::Comma)) {
            self.advance();
            specifiers.push(self.parse_use_specifier()?);
        }

        let end = specifiers
            .last()
            .map(use_specifier_end)
            .unwrap_or(start);

        Ok(UseDeclaration {
            span: Span::new(start, end),
            specifiers,
            source: None,
        })
    }

    fn parse_use_specifier(
        &mut self,
    ) -> Result<UseSpecifier, ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::Star => {
                Ok(UseSpecifier::Wildcard {
                    span: token.span,
                })
            }

            TokenKind::Ident(name) => {
                Ok(UseSpecifier::Named {
                    name,
                    span: token.span,
                })
            }

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an import name or `*`",
                    found,
                },
                token.span,
            )),
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();

        if !token.is_eof() {
            self.position += 1;
        }

        token
    }

    fn at(
        &self,
        predicate: impl FnOnce(&TokenKind) -> bool,
    ) -> bool {
        predicate(&self.current().kind)
    }

    fn expected(&self, expected: &'static str) -> ParseError {
        ParseError::new(
            ParseErrorKind::Expected {
                expected,
                found: self.current().kind.clone(),
            },
            self.current().span,
        )
    }
}

fn use_specifier_end(specifier: &UseSpecifier) -> usize {
    match specifier {
        UseSpecifier::Wildcard { span }
        | UseSpecifier::Named { span, .. } => span.end,
    }
}