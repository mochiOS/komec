use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, Binding, ComponentDeclaration, ComponentMember, Declaration,
        FunctionDeclaration, Module, RecipeDeclaration, UseDeclaration, UseSpecifier,
    },
    expressions::{
        AssignOp, AssignmentExpression, BinaryOp, BlockExpression, CallArg, CallExpression,
        ClosureExpression, ComponentExpression, DotIdentifierExpression, Expression,
        GroupExpression, IndexExpression, KeyValueProperty, ListExpression, LiteralKind,
        MemberExpression, NumberLiteral, ObjectExpression, ObjectProperty, PropertyKey,
        UnaryExpression, UnaryOp,
    },
    patterns::{DotIdentPattern, IdentifierPattern, IsPattern, LiteralPattern, Pattern},
    statements::{
        BlockStatement, BreakStatement, ContinueStatement, ExpressionStatement, ForInStatement,
        IfStatement, IsStatement, ReturnStatement, Statement, WhileStatement,
    },
    types::{NamedType, Parameter, PrimitiveType, PrimitiveTypeKind, Type},
};

use crate::{
    error::{ParseError, ParseErrorKind},
    token::{Token, TokenKind},
};

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
    allow_component_children: bool,
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
            allow_component_children: true,
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

    pub fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        let expression = self.parse_assignment_expression()?;

        if !self.current().is_eof() {
            return Err(self.expected("the end of the expression"));
        }

        Ok(expression)
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        let attributes = self.parse_attributes()?;

        if self.at(|kind| matches!(kind, TokenKind::Component)) {
            return self
                .parse_component_declaration(attributes)
                .map(Declaration::Component);
        }

        if self.at(|kind| matches!(kind, TokenKind::Fn)) {
            return self
                .parse_function_declaration(attributes)
                .map(Declaration::Function);
        }

        if self.at(|kind| matches!(kind, TokenKind::Let)) {
            return self.parse_let_binding(attributes).map(Declaration::Let);
        }

        if self.at(|kind| matches!(kind, TokenKind::Use)) {
            if attributes.is_empty() {
                return self.parse_use_declaration().map(Declaration::Use);
            }

            return Err(self.expected("a component, function, or let declaration after attributes"));
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
        let at = self.expect("`@`", |kind| matches!(kind, TokenKind::At))?;

        let (name, name_span) = self.expect_identifier("an attribute name")?;

        let (args, end) = if self.at(|kind| matches!(kind, TokenKind::LParen)) {
            self.parse_attribute_arguments()?
        } else {
            (Vec::new(), name_span.end)
        };

        Ok(Attribute {
            span: Span::new(at.span.start, end),
            name,
            args,
        })
    }

    fn parse_attribute_arguments(&mut self) -> Result<(Vec<Expression>, usize), ParseError> {
        self.expect("`(`", |kind| matches!(kind, TokenKind::LParen))?;

        let mut args = Vec::new();

        if !self.at(|kind| matches!(kind, TokenKind::RParen)) {
            loop {
                args.push(self.parse_assignment_expression()?);

                if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                    break;
                }

                self.advance();

                if self.at(|kind| matches!(kind, TokenKind::RParen)) {
                    break;
                }
            }
        }

        let closing = self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        Ok((args, closing.span.end))
    }

    fn parse_component_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<ComponentDeclaration, ParseError> {
        let component = self.expect("`component`", |kind| matches!(kind, TokenKind::Component))?;

        let start = attributes
            .first()
            .map_or(component.span.start, |attribute| attribute.span.start);

        let (name, _) = self.expect_identifier("a component name")?;

        self.expect("`(`", |kind| matches!(kind, TokenKind::LParen))?;

        let params = self.parse_component_parameters()?;

        self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut body = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            body.push(self.parse_component_member()?);
        }

        let closing_brace = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(ComponentDeclaration {
            span: Span::new(start, closing_brace.span.end),
            name,
            params,
            attributes,
            body,
        })
    }

    fn parse_component_parameters(&mut self) -> Result<Vec<Parameter>, ParseError> {
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

    fn parse_component_parameter(&mut self) -> Result<Parameter, ParseError> {
        let (name, name_span) = self.expect_identifier("a parameter name")?;

        self.expect("`:`", |kind| matches!(kind, TokenKind::Colon))?;

        let type_ = self.parse_type()?;
        let type_end = type_.span().end;

        let default = if self.at(|kind| matches!(kind, TokenKind::Assign)) {
            self.advance();
            Some(self.parse_assignment_expression()?)
        } else {
            None
        };

        let end = default
            .as_ref()
            .map_or(type_end, |expression| expression.span().end);

        Ok(Parameter {
            span: Span::new(name_span.start, end),
            name,
            type_,
            default,
        })
    }

    fn parse_component_member(&mut self) -> Result<ComponentMember, ParseError> {
        let attributes = self.parse_attributes()?;

        if self.at(|kind| matches!(kind, TokenKind::State)) {
            return self
                .parse_state_binding(attributes)
                .map(|binding| ComponentMember::State(Box::new(binding)));
        }

        if self.at(|kind| matches!(kind, TokenKind::Let)) {
            return self
                .parse_let_binding(attributes)
                .map(|binding| ComponentMember::Let(Box::new(binding)));
        }

        if self.at(|kind| matches!(kind, TokenKind::Recipe)) {
            return self
                .parse_recipe_declaration(attributes)
                .map(ComponentMember::Recipe);
        }

        if self.at(|kind| matches!(kind, TokenKind::Fn)) {
            return self
                .parse_function_declaration(attributes)
                .map(ComponentMember::Function);
        }

        if attributes.is_empty() {
            Err(self.expected("a component member"))
        } else {
            Err(self.expected("a component member after attributes"))
        }
    }

    fn parse_recipe_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<RecipeDeclaration, ParseError> {
        let keyword = self.expect("`recipe`", |kind| matches!(kind, TokenKind::Recipe))?;

        let start = attributes
            .first()
            .map_or(keyword.span.start, |attribute| attribute.span.start);

        let (name, _) = self.expect_identifier("a recipe name")?;

        let event_source = if self.at(|kind| matches!(kind, TokenKind::Colon)) {
            self.advance();

            let (event_source, _) = self.expect_identifier("an event source after `:`")?;

            Some(event_source)
        } else {
            None
        };

        let body = self.parse_statement_block()?;
        let end = body.span.end;

        Ok(RecipeDeclaration {
            span: Span::new(start, end),
            attributes,
            name,
            event_source,
            body,
        })
    }

    fn parse_function_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<FunctionDeclaration, ParseError> {
        let keyword = self.expect("`fn`", |kind| matches!(kind, TokenKind::Fn))?;

        let start = attributes
            .first()
            .map_or(keyword.span.start, |attribute| attribute.span.start);

        let (name, _) = self.expect_identifier("a function name")?;

        self.expect("`(`", |kind| matches!(kind, TokenKind::LParen))?;

        let params = self.parse_function_parameters()?;

        let closing_parenthesis = self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        let return_type = if self.at(|kind| matches!(kind, TokenKind::ThinArrow)) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = if self.at(|kind| matches!(kind, TokenKind::LBrace)) {
            Some(self.parse_statement_block()?)
        } else {
            None
        };

        let end = body
            .as_ref()
            .map(|body| body.span.end)
            .or_else(|| return_type.as_ref().map(|type_| type_.span().end))
            .unwrap_or(closing_parenthesis.span.end);

        Ok(FunctionDeclaration {
            span: Span::new(start, end),
            attributes,
            name,
            params,
            body,
            return_type,
        })
    }

    fn parse_function_parameters(&mut self) -> Result<Vec<Pattern>, ParseError> {
        let mut parameters = Vec::new();

        if self.at(|kind| matches!(kind, TokenKind::RParen)) {
            return Ok(parameters);
        }

        loop {
            parameters.push(self.parse_function_parameter()?);

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

    fn parse_function_parameter(&mut self) -> Result<Pattern, ParseError> {
        let (name, name_span) = self.expect_identifier("a parameter name")?;

        let type_annotation = if self.at(|kind| matches!(kind, TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let default = if self.at(|kind| matches!(kind, TokenKind::Assign)) {
            self.advance();

            Some(Box::new(self.parse_assignment_expression()?))
        } else {
            None
        };

        let end = default
            .as_ref()
            .map(|expression| expression.span().end)
            .or_else(|| type_annotation.as_ref().map(|type_| type_.span().end))
            .unwrap_or(name_span.end);

        Ok(Pattern::Ident(IdentifierPattern {
            span: Span::new(name_span.start, end),
            name,
            type_annotation,
            default,
        }))
    }

    fn parse_state_binding(&mut self, attributes: Vec<Attribute>) -> Result<Binding, ParseError> {
        let keyword = self.expect("`state`", |kind| matches!(kind, TokenKind::State))?;

        self.parse_binding_after_keyword(attributes, keyword.span.start, false)
    }

    fn parse_let_binding(&mut self, attributes: Vec<Attribute>) -> Result<Binding, ParseError> {
        let keyword = self.expect("`let`", |kind| matches!(kind, TokenKind::Let))?;

        let mutable = if self.at(|kind| matches!(kind, TokenKind::Mut)) {
            self.advance();
            true
        } else {
            false
        };

        self.parse_binding_after_keyword(attributes, keyword.span.start, mutable)
    }

    fn parse_binding_after_keyword(
        &mut self,
        attributes: Vec<Attribute>,
        keyword_start: usize,
        mutable: bool,
    ) -> Result<Binding, ParseError> {
        let (name, name_span) = self.expect_identifier("a binding name")?;

        let type_annotation = if self.at(|kind| matches!(kind, TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let init = if self.at(|kind| matches!(kind, TokenKind::Assign)) {
            self.advance();
            Some(self.parse_assignment_expression()?)
        } else {
            None
        };

        let end = init
            .as_ref()
            .map(|expression| expression.span().end)
            .or_else(|| type_annotation.as_ref().map(|type_| type_.span().end))
            .unwrap_or(name_span.end);

        let start = attributes
            .first()
            .map_or(keyword_start, |attribute| attribute.span.start);

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
        let (name, span) = self.expect_identifier("a type name")?;

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

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match &self.current().kind {
            TokenKind::Let => self.parse_let_binding(Vec::new()).map(Statement::Let),

            TokenKind::If => self.parse_if_statement(),

            TokenKind::While => self.parse_while_statement(),

            TokenKind::For => self.parse_for_in_statement(),

            TokenKind::Is => self.parse_is_statement(),

            TokenKind::Return => self.parse_return_statement(),

            TokenKind::Break => self.parse_break_statement(),

            TokenKind::Continue => self.parse_continue_statement(),

            TokenKind::LBrace => self.parse_statement_block().map(Statement::Block),

            _ => self.parse_expression_statement(),
        }
    }

    fn parse_statement_block(&mut self) -> Result<BlockStatement, ParseError> {
        let opening = self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut statements = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            statements.push(self.parse_statement()?);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(BlockStatement {
            span: Span::new(opening.span.start, closing.span.end),
            statements,
        })
    }

    fn parse_if_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`if`", |kind| matches!(kind, TokenKind::If))?;

        let test = self.parse_condition_expression()?;

        let consequent = Statement::Block(self.parse_statement_block()?);

        let mut end = consequent.span().end;

        let alternative = if self.at(|kind| matches!(kind, TokenKind::Else)) {
            self.advance();

            let statement = if self.at(|kind| matches!(kind, TokenKind::If)) {
                self.parse_if_statement()?
            } else if self.at(|kind| matches!(kind, TokenKind::LBrace)) {
                Statement::Block(self.parse_statement_block()?)
            } else {
                return Err(self.expected("`if` or `{` after `else`"));
            };

            end = statement.span().end;

            Some(Box::new(statement))
        } else {
            None
        };

        Ok(Statement::If(IfStatement {
            span: Span::new(keyword.span.start, end),
            test,
            consequent: Box::new(consequent),
            alternative,
        }))
    }

    fn parse_while_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`while`", |kind| matches!(kind, TokenKind::While))?;

        let test = self.parse_condition_expression()?;

        let body = Statement::Block(self.parse_statement_block()?);

        let span = Span::new(keyword.span.start, body.span().end);

        Ok(Statement::While(WhileStatement {
            span,
            test,
            body: Box::new(body),
        }))
    }

    fn parse_for_in_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`for`", |kind| matches!(kind, TokenKind::For))?;

        let (name, name_span) = self.expect_identifier("a loop binding")?;

        let pattern = Pattern::Ident(IdentifierPattern {
            span: name_span,
            name,
            type_annotation: None,
            default: None,
        });

        self.expect("`in`", |kind| matches!(kind, TokenKind::In))?;

        let right = self.parse_condition_expression()?;

        let body = Statement::Block(self.parse_statement_block()?);

        let span = Span::new(keyword.span.start, body.span().end);

        Ok(Statement::ForIn(ForInStatement {
            span,
            pattern,
            right,
            body: Box::new(body),
        }))
    }

    fn parse_is_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`is`", |kind| matches!(kind, TokenKind::Is))?;

        let arrow_index = self.find_is_arrow()?;
        let pattern_start = self.find_is_pattern_start(arrow_index)?;

        let value = if pattern_start > self.position {
            let value_tokens = self.tokens[self.position..pattern_start].to_vec();

            let mut value_parser = Parser::new(value_tokens);

            value_parser.allow_component_children = false;

            let value = value_parser.parse_expression()?;

            self.position = pattern_start;

            Some(value)
        } else {
            None
        };

        let pattern = self.parse_is_pattern()?;

        self.expect("`=>`", |kind| matches!(kind, TokenKind::FatArrow))?;

        let body = if self.at(|kind| matches!(kind, TokenKind::LBrace)) {
            Statement::Block(self.parse_statement_block()?)
        } else {
            self.parse_statement()?
        };

        let span = Span::new(keyword.span.start, body.span().end);

        Ok(Statement::Is(IsStatement {
            span,
            value,
            pattern,
            body: Box::new(body),
        }))
    }

    fn find_is_arrow(&self) -> Result<usize, ParseError> {
        let mut parentheses = 0usize;
        let mut brackets = 0usize;
        let mut braces = 0usize;
        let mut index = self.position;

        loop {
            let token = &self.tokens[index];

            match token.kind {
                TokenKind::LParen => {
                    parentheses += 1;
                }

                TokenKind::RParen => {
                    parentheses = parentheses.saturating_sub(1);
                }

                TokenKind::LBracket => {
                    brackets += 1;
                }

                TokenKind::RBracket => {
                    brackets = brackets.saturating_sub(1);
                }

                TokenKind::LBrace => {
                    braces += 1;
                }

                TokenKind::RBrace if braces > 0 => {
                    braces -= 1;
                }

                TokenKind::FatArrow if parentheses == 0 && brackets == 0 && braces == 0 => {
                    return Ok(index);
                }

                TokenKind::RBrace | TokenKind::Eof
                    if parentheses == 0 && brackets == 0 && braces == 0 =>
                {
                    return Err(ParseError::new(
                        ParseErrorKind::Expected {
                            expected: "`=>`",
                            found: token.kind.clone(),
                        },
                        token.span,
                    ));
                }

                _ => {}
            }

            index += 1;
        }
    }

    fn find_is_pattern_start(&self, arrow_index: usize) -> Result<usize, ParseError> {
        if arrow_index == self.position {
            let arrow = &self.tokens[arrow_index];

            return Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: an_is_pattern(),
                    found: arrow.kind.clone(),
                },
                arrow.span,
            ));
        }

        let last_index = arrow_index - 1;

        if matches!(self.tokens[last_index].kind, TokenKind::Ident(_))
            && last_index > self.position
            && matches!(self.tokens[last_index - 1].kind, TokenKind::Dot)
        {
            return Ok(last_index - 1);
        }

        Ok(last_index)
    }

    fn parse_is_pattern(&mut self) -> Result<IsPattern, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::Dot)) {
            let dot = self.advance();

            let (name, name_span) = self.expect_identifier("an identifier after `.`")?;

            return Ok(IsPattern::DotIdent(DotIdentPattern {
                span: Span::new(dot.span.start, name_span.end),
                name,
            }));
        }

        let token = self.advance();
        let span = token.span;

        match token.kind {
            TokenKind::Ident(name) => Ok(IsPattern::Ident(IdentifierPattern {
                span,
                name,
                type_annotation: None,
                default: None,
            })),

            TokenKind::String(value) => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::String(value),
            })),

            TokenKind::Number(value) => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::Number(NumberLiteral(value)),
            })),

            TokenKind::Percent(value) => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::Percent(NumberLiteral(value)),
            })),

            TokenKind::True => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::Boolean(true),
            })),

            TokenKind::False => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::Boolean(false),
            })),

            TokenKind::Null => Ok(IsPattern::Literal(LiteralPattern {
                span,
                value: LiteralKind::Null,
            })),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: an_is_pattern(),
                    found,
                },
                span,
            )),
        }
    }

    fn parse_return_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`return`", |kind| matches!(kind, TokenKind::Return))?;

        let argument = if self
            .at(|kind| matches!(kind, TokenKind::RBrace | TokenKind::Comma | TokenKind::Eof))
        {
            None
        } else {
            Some(self.parse_assignment_expression()?)
        };

        let end = argument
            .as_ref()
            .map_or(keyword.span.end, |expression| expression.span().end);

        Ok(Statement::Return(ReturnStatement {
            span: Span::new(keyword.span.start, end),
            argument,
        }))
    }

    fn parse_break_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`break`", |kind| matches!(kind, TokenKind::Break))?;

        Ok(Statement::Break(BreakStatement {
            span: keyword.span,
            label: None,
        }))
    }

    fn parse_continue_statement(&mut self) -> Result<Statement, ParseError> {
        let keyword = self.expect("`continue`", |kind| matches!(kind, TokenKind::Continue))?;

        Ok(Statement::Continue(ContinueStatement {
            span: keyword.span,
            label: None,
        }))
    }

    fn parse_expression_statement(&mut self) -> Result<Statement, ParseError> {
        let expression = self.parse_assignment_expression()?;
        let span = expression.span();

        Ok(Statement::Expression(ExpressionStatement {
            span,
            expression,
        }))
    }

    fn parse_condition_expression(&mut self) -> Result<Expression, ParseError> {
        let previous = self.allow_component_children;

        self.allow_component_children = false;

        let result = self.parse_assignment_expression();

        self.allow_component_children = previous;

        result
    }

    fn parse_assignment_expression(&mut self) -> Result<Expression, ParseError> {
        let left = self.parse_or_expression()?;

        let op = match self.current().kind {
            TokenKind::Assign => AssignOp::Assign,
            TokenKind::PlusAssign => AssignOp::AddAssign,
            _ => return Ok(left),
        };

        self.advance();

        let right = self.parse_assignment_expression()?;

        let span = Span::new(left.span().start, right.span().end);

        Ok(Expression::Assign(AssignmentExpression {
            span,
            op,
            target: Box::new(left),
            value: Box::new(right),
        }))
    }

    fn parse_or_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_and_expression()?;

        while self.at(|kind| matches!(kind, TokenKind::Or)) {
            self.advance();

            let right = self.parse_and_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, BinaryOp::Or, right, span);
        }

        Ok(expression)
    }

    fn parse_and_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_equality_expression()?;

        while self.at(|kind| matches!(kind, TokenKind::And)) {
            self.advance();

            let right = self.parse_equality_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, BinaryOp::And, right, span);
        }

        Ok(expression)
    }

    fn parse_equality_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_comparison_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };

            self.advance();

            let right = self.parse_comparison_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, op, right, span);
        }

        Ok(expression)
    }

    fn parse_comparison_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_additive_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Lte => BinaryOp::Lte,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Gte => BinaryOp::Gte,
                _ => break,
            };

            self.advance();

            let right = self.parse_additive_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, op, right, span);
        }

        Ok(expression)
    }

    fn parse_additive_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_multiplicative_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };

            self.advance();

            let right = self.parse_multiplicative_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, op, right, span);
        }

        Ok(expression)
    }

    fn parse_multiplicative_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_unary_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                _ => break,
            };

            self.advance();

            let right = self.parse_unary_expression()?;

            let span = Span::new(expression.span().start, right.span().end);

            expression = Expression::binary(expression, op, right, span);
        }

        Ok(expression)
    }

    fn parse_unary_expression(&mut self) -> Result<Expression, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::Not)) {
            let operator = self.advance();

            let argument = self.parse_unary_expression()?;

            return Ok(Expression::Unary(UnaryExpression {
                span: Span::new(operator.span.start, argument.span().end),
                op: UnaryOp::Not,
                argument: Box::new(argument),
            }));
        }

        self.parse_postfix_expression()
    }

    fn parse_postfix_expression(&mut self) -> Result<Expression, ParseError> {
        let mut expression = self.parse_primary_expression()?;

        loop {
            if self.at(|kind| matches!(kind, TokenKind::LParen)) {
                expression = self.parse_call_expression(expression)?;

                continue;
            }

            if self.allow_component_children
                && self.at(|kind| matches!(kind, TokenKind::LBrace))
                && Self::is_component_head(&expression)
            {
                expression = self.parse_component_expression(expression)?;

                continue;
            }

            if self.at(|kind| matches!(kind, TokenKind::Dot)) {
                expression = self.parse_member_expression(expression)?;

                continue;
            }

            if self.at(|kind| matches!(kind, TokenKind::LBracket)) {
                expression = self.parse_index_expression(expression)?;

                continue;
            }

            break;
        }

        Ok(expression)
    }

    fn parse_call_expression(&mut self, callee: Expression) -> Result<Expression, ParseError> {
        self.expect("`(`", |kind| matches!(kind, TokenKind::LParen))?;

        let mut args = Vec::new();

        if !self.at(|kind| matches!(kind, TokenKind::RParen)) {
            loop {
                args.push(self.parse_call_argument()?);

                if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                    break;
                }

                self.advance();

                if self.at(|kind| matches!(kind, TokenKind::RParen)) {
                    break;
                }
            }
        }

        let closing = self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        let span = Span::new(callee.span().start, closing.span.end);

        Ok(Expression::Call(CallExpression {
            span,
            callee: Box::new(callee),
            args,
        }))
    }

    fn parse_call_argument(&mut self) -> Result<CallArg, ParseError> {
        if self.is_named_argument() {
            let (name, name_span) = self.expect_identifier("an argument name")?;

            self.expect("`:`", |kind| matches!(kind, TokenKind::Colon))?;

            let value = self.parse_assignment_expression()?;

            let span = Span::new(name_span.start, value.span().end);

            return Ok(CallArg::Named {
                name,
                value: Box::new(value),
                span,
            });
        }

        Ok(CallArg::Positional(self.parse_assignment_expression()?))
    }

    fn parse_component_expression(&mut self, head: Expression) -> Result<Expression, ParseError> {
        let start = head.span().start;

        let (name, args) = match head {
            Expression::Ident(identifier) => (identifier.name, Vec::new()),

            Expression::Call(call) => {
                let CallExpression { callee, args, .. } = call;

                let Expression::Ident(identifier) = *callee else {
                    return Err(self.expected("a component name before `{`"));
                };

                (identifier.name, args)
            }

            _ => {
                return Err(self.expected("a component name before `{`"));
            }
        };

        let (brace_span, children) = self.parse_braced_expressions()?;

        Ok(Expression::Component(ComponentExpression {
            span: Span::new(start, brace_span.end),
            name,
            args,
            children,
        }))
    }

    fn parse_member_expression(&mut self, object: Expression) -> Result<Expression, ParseError> {
        self.expect("`.`", |kind| matches!(kind, TokenKind::Dot))?;

        let (property, property_span) = self.expect_identifier("a property name after `.`")?;

        let span = Span::new(object.span().start, property_span.end);

        Ok(Expression::Member(MemberExpression {
            span,
            object: Box::new(object),
            property,
        }))
    }

    fn parse_index_expression(&mut self, object: Expression) -> Result<Expression, ParseError> {
        self.expect("`[`", |kind| matches!(kind, TokenKind::LBracket))?;

        let index = self.parse_assignment_expression()?;

        let closing = self.expect("`]`", |kind| matches!(kind, TokenKind::RBracket))?;

        let span = Span::new(object.span().start, closing.span.end);

        Ok(Expression::Index(IndexExpression {
            span,
            object: Box::new(object),
            index: Box::new(index),
        }))
    }

    fn parse_primary_expression(&mut self) -> Result<Expression, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::Pipe)) {
            return self.parse_closure_expression();
        }

        if self.at(|kind| matches!(kind, TokenKind::Dot)) {
            return self.parse_dot_identifier_expression();
        }

        if self.at(|kind| matches!(kind, TokenKind::LParen)) {
            return self.parse_group_expression();
        }

        if self.at(|kind| matches!(kind, TokenKind::LBracket)) {
            return self.parse_list_expression();
        }

        if self.at(|kind| matches!(kind, TokenKind::LBrace)) {
            if self.is_object_literal_start() {
                return self.parse_object_expression();
            }

            return self.parse_block_expression();
        }

        let token = self.advance();
        let span = token.span;

        match token.kind {
            TokenKind::String(value) => Ok(Expression::literal(LiteralKind::String(value), span)),

            TokenKind::Number(value) => Ok(Expression::literal(
                LiteralKind::Number(NumberLiteral(value)),
                span,
            )),

            TokenKind::Percent(value) => Ok(Expression::literal(
                LiteralKind::Percent(NumberLiteral(value)),
                span,
            )),

            TokenKind::True => Ok(Expression::literal(LiteralKind::Boolean(true), span)),

            TokenKind::False => Ok(Expression::literal(LiteralKind::Boolean(false), span)),

            TokenKind::Null => Ok(Expression::literal(LiteralKind::Null, span)),

            TokenKind::Ident(name) => Ok(Expression::ident(name, span)),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an expression",
                    found,
                },
                span,
            )),
        }
    }

    fn parse_closure_expression(&mut self) -> Result<Expression, ParseError> {
        let opening = self.expect("`|`", |kind| matches!(kind, TokenKind::Pipe))?;

        if self.at(|kind| matches!(kind, TokenKind::Pipe)) {
            return Err(self.expected("a closure parameter"));
        }

        let mut params = Vec::new();

        loop {
            params.push(self.parse_closure_parameter()?);

            if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                break;
            }

            self.advance();

            if self.at(|kind| matches!(kind, TokenKind::Pipe)) {
                return Err(self.expected("a closure parameter after `,`"));
            }
        }

        self.expect("the closing `|`", |kind| matches!(kind, TokenKind::Pipe))?;

        let body = self.parse_assignment_expression()?;

        let span = Span::new(opening.span.start, body.span().end);

        Ok(Expression::Closure(ClosureExpression {
            span,
            params,
            body: Box::new(body),
        }))
    }

    fn parse_closure_parameter(&mut self) -> Result<Pattern, ParseError> {
        let (name, name_span) = self.expect_identifier("a closure parameter")?;

        let type_annotation = if self.at(|kind| matches!(kind, TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let end = type_annotation
            .as_ref()
            .map_or(name_span.end, |type_| type_.span().end);

        Ok(Pattern::Ident(IdentifierPattern {
            span: Span::new(name_span.start, end),
            name,
            type_annotation,
            default: None,
        }))
    }

    fn parse_object_expression(&mut self) -> Result<Expression, ParseError> {
        let opening = self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut props = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            props.push(self.parse_object_property()?);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();

                if self.at(|kind| matches!(kind, TokenKind::RBrace)) {
                    break;
                }

                continue;
            }

            if !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
                return Err(self.expected("`,` or `}` after an object property"));
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(Expression::Object(ObjectExpression {
            span: Span::new(opening.span.start, closing.span.end),
            props,
        }))
    }

    fn parse_object_property(&mut self) -> Result<ObjectProperty, ParseError> {
        let key = self.parse_property_key()?;
        let start = property_key_span(&key).start;

        self.expect("`:`", |kind| matches!(kind, TokenKind::Colon))?;

        let value = self.parse_assignment_expression()?;

        let span = Span::new(start, value.span().end);

        Ok(ObjectProperty::KeyValue(KeyValueProperty {
            span,
            key,
            value: Box::new(value),
        }))
    }

    fn parse_property_key(&mut self) -> Result<PropertyKey, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::LBracket)) {
            let opening = self.advance();

            let expression = self.parse_assignment_expression()?;

            let closing = self.expect("`]`", |kind| matches!(kind, TokenKind::RBracket))?;

            return Ok(PropertyKey::Computed {
                expression: Box::new(expression),
                span: Span::new(opening.span.start, closing.span.end),
            });
        }

        let token = self.advance();
        let span = token.span;

        match token.kind {
            TokenKind::Ident(name) => Ok(PropertyKey::Ident { name, span }),

            TokenKind::String(value) => Ok(PropertyKey::String { value, span }),

            TokenKind::Number(value) => Ok(PropertyKey::Number { value, span }),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an object property key",
                    found,
                },
                span,
            )),
        }
    }

    fn parse_dot_identifier_expression(&mut self) -> Result<Expression, ParseError> {
        let dot = self.expect("`.`", |kind| matches!(kind, TokenKind::Dot))?;

        let (name, name_span) = self.expect_identifier("an identifier after `.`")?;

        Ok(Expression::DotIdent(DotIdentifierExpression {
            span: Span::new(dot.span.start, name_span.end),
            name,
        }))
    }

    fn parse_group_expression(&mut self) -> Result<Expression, ParseError> {
        let opening = self.expect("`(`", |kind| matches!(kind, TokenKind::LParen))?;

        let expression = self.parse_assignment_expression()?;

        let closing = self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        Ok(Expression::Group(GroupExpression {
            span: Span::new(opening.span.start, closing.span.end),
            expression: Box::new(expression),
        }))
    }

    fn parse_list_expression(&mut self) -> Result<Expression, ParseError> {
        let opening = self.expect("`[`", |kind| matches!(kind, TokenKind::LBracket))?;

        let mut elems = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBracket)) {
            if self.current().is_eof() {
                return Err(self.expected("`]`"));
            }

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                elems.push(None);
                self.advance();
                continue;
            }

            elems.push(Some(self.parse_assignment_expression()?));

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            } else {
                break;
            }
        }

        let closing = self.expect("`]`", |kind| matches!(kind, TokenKind::RBracket))?;

        Ok(Expression::List(ListExpression {
            span: Span::new(opening.span.start, closing.span.end),
            elems,
        }))
    }

    fn parse_block_expression(&mut self) -> Result<Expression, ParseError> {
        let opening = self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut statements = Vec::new();
        let mut tail = None;

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            if self.starts_statement() {
                statements.push(self.parse_statement()?);

                if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                    self.advance();
                }

                continue;
            }

            let expression = self.parse_assignment_expression()?;

            if self.at(|kind| matches!(kind, TokenKind::RBrace)) {
                tail = Some(Box::new(expression));
                break;
            }

            let span = expression.span();

            statements.push(Statement::Expression(ExpressionStatement {
                span,
                expression,
            }));

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(Expression::Block(BlockExpression {
            span: Span::new(opening.span.start, closing.span.end),
            statements,
            tail,
        }))
    }

    fn parse_braced_expressions(&mut self) -> Result<(Span, Vec<Expression>), ParseError> {
        let opening = self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut expressions = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            expressions.push(self.parse_assignment_expression()?);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok((Span::new(opening.span.start, closing.span.end), expressions))
    }

    fn parse_use_declaration(&mut self) -> Result<UseDeclaration, ParseError> {
        let start = self
            .expect("`use`", |kind| matches!(kind, TokenKind::Use))?
            .span
            .start;

        let mut specifiers = vec![self.parse_use_specifier()?];

        while self.at(|kind| matches!(kind, TokenKind::Comma)) {
            self.advance();

            specifiers.push(self.parse_use_specifier()?);
        }

        let end = specifiers.last().map(use_specifier_end).unwrap_or(start);

        Ok(UseDeclaration {
            span: Span::new(start, end),
            specifiers,
            source: None,
        })
    }

    fn parse_use_specifier(&mut self) -> Result<UseSpecifier, ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::Star => Ok(UseSpecifier::Wildcard { span: token.span }),

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

    fn starts_statement(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Let
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Is
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
        )
    }

    fn is_component_head(expression: &Expression) -> bool {
        match expression {
            Expression::Ident(_) => true,

            Expression::Call(call) => {
                matches!(call.callee.as_ref(), Expression::Ident(_))
            }

            _ => false,
        }
    }

    fn is_named_argument(&self) -> bool {
        matches!(
            (&self.current().kind, &self.next().kind,),
            (TokenKind::Ident(_), TokenKind::Colon,)
        )
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

    fn expect_identifier(&mut self, expected: &'static str) -> Result<(String, Span), ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::Ident(name) => Ok((name, token.span)),

            found => Err(ParseError::new(
                ParseErrorKind::Expected { expected, found },
                token.span,
            )),
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn next(&self) -> &Token {
        self.tokens.get(self.position + 1).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("parser token stream must contain EOF")
        })
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();

        if !token.is_eof() {
            self.position += 1;
        }

        token
    }

    fn at(&self, predicate: impl FnOnce(&TokenKind) -> bool) -> bool {
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

    fn is_object_literal_start(&self) -> bool {
        let Some(first) = self.tokens.get(self.position + 1) else {
            return false;
        };

        match &first.kind {
            TokenKind::Ident(_) | TokenKind::String(_) | TokenKind::Number(_) => self
                .tokens
                .get(self.position + 2)
                .is_some_and(|token| matches!(token.kind, TokenKind::Colon)),

            TokenKind::LBracket => self.computed_property_key_has_colon(self.position + 1),

            _ => false,
        }
    }

    fn computed_property_key_has_colon(&self, opening_index: usize) -> bool {
        let mut depth = 0usize;
        let mut index = opening_index;

        while let Some(token) = self.tokens.get(index) {
            match token.kind {
                TokenKind::LBracket => {
                    depth += 1;
                }

                TokenKind::RBracket => {
                    if depth == 1 {
                        return self
                            .tokens
                            .get(index + 1)
                            .is_some_and(|next| matches!(next.kind, TokenKind::Colon));
                    }

                    depth = depth.saturating_sub(1);
                }

                TokenKind::Eof => {
                    return false;
                }

                _ => {}
            }

            index += 1;
        }

        false
    }
}

fn use_specifier_end(specifier: &UseSpecifier) -> usize {
    match specifier {
        UseSpecifier::Wildcard { span } | UseSpecifier::Named { span, .. } => span.end,
    }
}

fn property_key_span(key: &PropertyKey) -> Span {
    match key {
        PropertyKey::Ident { span, .. }
        | PropertyKey::String { span, .. }
        | PropertyKey::Number { span, .. }
        | PropertyKey::Computed { span, .. } => *span,
    }
}

const fn an_is_pattern() -> &'static str {
    "an `is` pattern"
}
