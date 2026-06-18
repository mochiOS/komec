use kome_ast::{
    Span,
    declarations::{
        Attribute, ComponentDeclaration, Declaration, Module, UseDeclaration, UseSpecifier,
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
            let offset = tokens.last().map_or(0, |token| token.span.end);
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

        Ok(Module::new(
            declarations,
            Span::new(0, self.current().span.end),
        ))
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        let attributes = self.parse_attributes()?;

        match &self.current().kind {
            TokenKind::Component => self
                .parse_component_declaration(attributes)
                .map(Declaration::Component),

            TokenKind::Use if attributes.is_empty() => {
                self.parse_use_declaration().map(Declaration::Use)
            }

            TokenKind::Use => {
                Err(self.expected("a component or function declaration after attributes"))
            }

            _ if attributes.is_empty() => Err(self.expected("a top-level declaration")),

            _ => Err(self.expected("a declaration after attributes")),
        }
    }

    fn parse_attributes(&mut self) -> Result<Vec<Attribute>, ParseError> {
        let mut attributes = Vec::new();

        while self.at(|kind| matches!(kind, TokenKind::At)) {
            attributes.push(self.parse_attribute()?);
        }

        Ok(attributes)
    }

    fn parse_attribute(&mut self) -> Result<Attribute, ParseError> {
        let at = self.expect(
            "`@`",
            |kind| matches!(kind, TokenKind::At),
        )?;

        let (name, name_span) = self.expect_identifier("an attribute name")?;

        Ok(Attribute {
            span: Span::new(at.span.start, name_span.end),
            name,
            args: Vec::new(),
        })
    }

    fn parse_component_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<ComponentDeclaration, ParseError> {
        let component = self.expect(
            "`component`",
            |kind| matches!(kind, TokenKind::Component),
        )?;

        let start = attributes
            .first()
            .map_or(component.span.start, |attribute| attribute.span.start);

        let (name, _) = self.expect_identifier("a component name")?;

        self.expect(
            "`(`",
            |kind| matches!(kind, TokenKind::LParen),
        )?;

        self.expect(
            "`)`",
            |kind| matches!(kind, TokenKind::RParen),
        )?;

        self.expect(
            "`{`",
            |kind| matches!(kind, TokenKind::LBrace),
        )?;

        let closing_brace = self.expect(
            "`}`",
            |kind| matches!(kind, TokenKind::RBrace),
        )?;

        Ok(ComponentDeclaration {
            span: Span::new(start, closing_brace.span.end),
            name,
            params: Vec::new(),
            attributes,
            body: Vec::new(),
        })
    }

    fn parse_use_declaration(&mut self) -> Result<UseDeclaration, ParseError> {
        let start = self.expect(
            "`use`",
            |kind| matches!(kind, TokenKind::Use),
        )?
            .span
            .start;

        let mut specifiers = vec![self.parse_use_specifier()?];

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

    fn parse_use_specifier(&mut self) -> Result<UseSpecifier, ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::Star => Ok(UseSpecifier::Wildcard {
                span: token.span,
            }),

            TokenKind::Ident(name) => Ok(UseSpecifier::Named {
                name,
                span: token.span,
            }),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an import name or `*`",
                    found,
                },
                token.span,
            )),
        }
    }

    fn expect(
        &mut self,
        expected: &'static str,
        predicate: impl FnOnce(&TokenKind) -> bool,
    ) -> Result<Token, ParseError> {
        if predicate(&self.current().kind) {
            Ok(self.advance())
        } else {
            Err(self.expected(expected))
        }
    }

    fn expect_identifier(
        &mut self,
        expected: &'static str,
    ) -> Result<(String, Span), ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::Ident(name) => Ok((name, token.span)),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected,
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