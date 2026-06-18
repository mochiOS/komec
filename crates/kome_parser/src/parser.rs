use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, Binding, ComponentDeclaration, ComponentMember, Declaration, Module,
        UseDeclaration, UseSpecifier,
    },
    expressions::{
        DotIdentifierExpression, Expression, LiteralKind, NumberLiteral,
    },
    patterns::{IdentifierPattern, Pattern},
    types::{
        NamedType, Parameter, PrimitiveType, PrimitiveTypeKind, Type,
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

        Ok(Module::new(
            declarations,
            Span::new(0, self.current().span.end),
        ))
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        let attributes = self.parse_attributes()?;

        if self.at(|kind| matches!(kind, TokenKind::Component)) {
            return self
                .parse_component_declaration(attributes)
                .map(Declaration::Component);
        }

        if self.at(|kind| matches!(kind, TokenKind::Use)) {
            if attributes.is_empty() {
                return self
                    .parse_use_declaration()
                    .map(Declaration::Use);
            }

            return Err(self.expected(
                "a component or function declaration after attributes",
            ));
        }

        if attributes.is_empty() {
            Err(self.expected("a top-level declaration"))
        } else {
            Err(self.expected("a declaration after attributes"))
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

        let (name, name_span) =
            self.expect_identifier("an attribute name")?;

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
            .map_or(component.span.start, |attribute| {
                attribute.span.start
            });

        let (name, _) =
            self.expect_identifier("a component name")?;

        self.expect(
            "`(`",
            |kind| matches!(kind, TokenKind::LParen),
        )?;

        let params = self.parse_component_parameters()?;

        self.expect(
            "`)`",
            |kind| matches!(kind, TokenKind::RParen),
        )?;

        self.expect(
            "`{`",
            |kind| matches!(kind, TokenKind::LBrace),
        )?;

        let mut body = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            body.push(self.parse_component_member()?);
        }

        let closing_brace = self.expect(
            "`}`",
            |kind| matches!(kind, TokenKind::RBrace),
        )?;

        Ok(ComponentDeclaration {
            span: Span::new(start, closing_brace.span.end),
            name,
            params,
            attributes,
            body,
        })
    }

    fn parse_component_parameters(
        &mut self,
    ) -> Result<Vec<Parameter>, ParseError> {
        let mut parameters = Vec::new();

        if self.at(|kind| matches!(kind, TokenKind::RParen)) {
            return Ok(parameters);
        }

        loop {
            parameters.push(self.parse_component_parameter()?);

            if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                break;
            }

            self.advance();

            if self.at(|kind| matches!(kind, TokenKind::RParen)) {
                break;
            }
        }

        Ok(parameters)
    }

    fn parse_component_parameter(
        &mut self,
    ) -> Result<Parameter, ParseError> {
        let (name, name_span) =
            self.expect_identifier("a parameter name")?;

        self.expect(
            "`:`",
            |kind| matches!(kind, TokenKind::Colon),
        )?;

        let type_ = self.parse_type()?;
        let type_end = type_.span().end;

        let default = if self.at(|kind| {
            matches!(kind, TokenKind::Assign)
        }) {
            self.advance();

            Some(self.parse_value_expression(
                "a parameter default value",
            )?)
        } else {
            None
        };

        let end = default
            .as_ref()
            .map_or(type_end, |expression| {
                expression.span().end
            });

        Ok(Parameter {
            span: Span::new(name_span.start, end),
            name,
            type_,
            default,
        })
    }

    fn parse_component_member(
        &mut self,
    ) -> Result<ComponentMember, ParseError> {
        let attributes = self.parse_attributes()?;

        if self.at(|kind| matches!(kind, TokenKind::State)) {
            return self
                .parse_state_binding(attributes)
                .map(|binding| {
                    ComponentMember::State(Box::new(binding))
                });
        }

        if self.at(|kind| matches!(kind, TokenKind::Let)) {
            return self
                .parse_let_binding(attributes)
                .map(|binding| {
                    ComponentMember::Let(Box::new(binding))
                });
        }

        if attributes.is_empty() {
            Err(self.expected("a component member"))
        } else {
            Err(self.expected(
                "a component member after attributes",
            ))
        }
    }

    fn parse_state_binding(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<Binding, ParseError> {
        let keyword = self.expect(
            "`state`",
            |kind| matches!(kind, TokenKind::State),
        )?;

        self.parse_binding_after_keyword(
            attributes,
            keyword.span.start,
            false,
        )
    }

    fn parse_let_binding(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<Binding, ParseError> {
        let keyword = self.expect(
            "`let`",
            |kind| matches!(kind, TokenKind::Let),
        )?;

        let mutable = if self.at(|kind| {
            matches!(kind, TokenKind::Mut)
        }) {
            self.advance();
            true
        } else {
            false
        };

        self.parse_binding_after_keyword(
            attributes,
            keyword.span.start,
            mutable,
        )
    }

    fn parse_binding_after_keyword(
        &mut self,
        attributes: Vec<Attribute>,
        keyword_start: usize,
        mutable: bool,
    ) -> Result<Binding, ParseError> {
        let (name, name_span) =
            self.expect_identifier("a binding name")?;

        let type_annotation = if self.at(|kind| {
            matches!(kind, TokenKind::Colon)
        }) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let init = if self.at(|kind| {
            matches!(kind, TokenKind::Assign)
        }) {
            self.advance();

            Some(self.parse_value_expression(
                "an initializer expression",
            )?)
        } else {
            None
        };

        let end = init
            .as_ref()
            .map(|expression| expression.span().end)
            .or_else(|| {
                type_annotation
                    .as_ref()
                    .map(|type_| type_.span().end)
            })
            .unwrap_or(name_span.end);

        let start = attributes
            .first()
            .map_or(keyword_start, |attribute| {
                attribute.span.start
            });

        Ok(Binding {
            span: Span::new(start, end),
            attributes,
            mutable,
            pattern: Pattern::Ident(IdentifierPattern {
                span: name_span,
                name,
                type_annotation: None,
                default: None,
            }),
            init,
            type_annotation,
        })
    }

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        let (name, span) =
            self.expect_identifier("a type name")?;

        let type_ = match name.as_str() {
            "String" => Type::Primitive(PrimitiveType {
                span,
                kind: PrimitiveTypeKind::String,
            }),

            "Number" => Type::Primitive(PrimitiveType {
                span,
                kind: PrimitiveTypeKind::Number,
            }),

            "Boolean" => Type::Primitive(PrimitiveType {
                span,
                kind: PrimitiveTypeKind::Boolean,
            }),

            "Null" => Type::Primitive(PrimitiveType {
                span,
                kind: PrimitiveTypeKind::Null,
            }),

            _ => Type::Named(NamedType {
                span,
                name,
                type_arguments: Vec::new(),
            }),
        };

        Ok(type_)
    }

    fn parse_value_expression(
        &mut self,
        expected: &'static str,
    ) -> Result<Expression, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::Dot)) {
            let dot = self.advance();

            let (name, name_span) =
                self.expect_identifier(
                    "an identifier after `.`",
                )?;

            return Ok(Expression::DotIdent(
                DotIdentifierExpression {
                    span: Span::new(
                        dot.span.start,
                        name_span.end,
                    ),
                    name,
                },
            ));
        }

        let token = self.advance();
        let span = token.span;

        let expression = match token.kind {
            TokenKind::String(value) => {
                Expression::literal(
                    LiteralKind::String(value),
                    span,
                )
            }

            TokenKind::Number(value) => {
                Expression::literal(
                    LiteralKind::Number(
                        NumberLiteral(value),
                    ),
                    span,
                )
            }

            TokenKind::Percent(value) => {
                Expression::literal(
                    LiteralKind::Percent(
                        NumberLiteral(value),
                    ),
                    span,
                )
            }

            TokenKind::True => {
                Expression::literal(
                    LiteralKind::Boolean(true),
                    span,
                )
            }

            TokenKind::False => {
                Expression::literal(
                    LiteralKind::Boolean(false),
                    span,
                )
            }

            TokenKind::Null => {
                Expression::literal(
                    LiteralKind::Null,
                    span,
                )
            }

            TokenKind::Ident(name) => {
                Expression::ident(name, span)
            }

            found => {
                return Err(ParseError::new(
                    ParseErrorKind::Expected {
                        expected,
                        found,
                    },
                    span,
                ));
            }
        };

        Ok(expression)
    }

    fn parse_use_declaration(
        &mut self,
    ) -> Result<UseDeclaration, ParseError> {
        let start = self
            .expect(
                "`use`",
                |kind| matches!(kind, TokenKind::Use),
            )?
            .span
            .start;

        let mut specifiers =
            vec![self.parse_use_specifier()?];

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
            TokenKind::Ident(name) => {
                Ok((name, token.span))
            }

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

    fn expected(
        &self,
        expected: &'static str,
    ) -> ParseError {
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