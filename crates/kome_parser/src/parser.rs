use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, Binding, ComponentDeclaration, ComponentMember, Declaration, Module,
        UseDeclaration, UseSpecifier,
    },
    expressions::{
        AssignOp, AssignmentExpression, BinaryOp, BlockExpression, CallArg, CallExpression,
        ComponentExpression, DotIdentifierExpression, Expression, GroupExpression,
        IndexExpression, ListExpression, LiteralKind, MemberExpression, NumberLiteral,
        UnaryExpression, UnaryOp,
    },
    patterns::{IdentifierPattern, Pattern},
    statements::{
        BlockStatement, BreakStatement, ContinueStatement, ExpressionStatement, IfStatement,
        ReturnStatement, Statement, WhileStatement,
    },
    types::{
        NamedType, Parameter, PrimitiveType, PrimitiveTypeKind, Type,
    },
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
            let offset = tokens
                .last()
                .map_or(0, |token| token.span.end);

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
            Some(self.parse_assignment_expression()?)
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
            Some(self.parse_assignment_expression()?)
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

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match &self.current().kind {
            TokenKind::Let => {
                self.parse_let_binding(Vec::new())
                    .map(Statement::Let)
            }

            TokenKind::If => self.parse_if_statement(),

            TokenKind::While => self.parse_while_statement(),

            TokenKind::Return => self.parse_return_statement(),

            TokenKind::Break => self.parse_break_statement(),

            TokenKind::Continue => self.parse_continue_statement(),

            TokenKind::LBrace => {
                self.parse_statement_block()
                    .map(Statement::Block)
            }

            _ => self.parse_expression_statement(),
        }
    }

    fn parse_statement_block(
        &mut self,
    ) -> Result<BlockStatement, ParseError> {
        let opening = self.expect(
            "`{`",
            |kind| matches!(kind, TokenKind::LBrace),
        )?;

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

        let closing = self.expect(
            "`}`",
            |kind| matches!(kind, TokenKind::RBrace),
        )?;

        Ok(BlockStatement {
            span: Span::new(
                opening.span.start,
                closing.span.end,
            ),
            statements,
        })
    }

    fn parse_if_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let keyword = self.expect(
            "`if`",
            |kind| matches!(kind, TokenKind::If),
        )?;

        let test = self.parse_condition_expression()?;

        let consequent = Statement::Block(
            self.parse_statement_block()?,
        );

        let mut end = consequent.span().end;

        let alternative = if self.at(|kind| {
            matches!(kind, TokenKind::Else)
        }) {
            self.advance();

            let statement = if self.at(|kind| {
                matches!(kind, TokenKind::If)
            }) {
                self.parse_if_statement()?
            } else if self.at(|kind| {
                matches!(kind, TokenKind::LBrace)
            }) {
                Statement::Block(
                    self.parse_statement_block()?,
                )
            } else {
                return Err(self.expected(
                    "`if` or `{` after `else`",
                ));
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

    fn parse_while_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let keyword = self.expect(
            "`while`",
            |kind| matches!(kind, TokenKind::While),
        )?;

        let test = self.parse_condition_expression()?;

        let body = Statement::Block(
            self.parse_statement_block()?,
        );

        let span = Span::new(
            keyword.span.start,
            body.span().end,
        );

        Ok(Statement::While(WhileStatement {
            span,
            test,
            body: Box::new(body),
        }))
    }

    fn parse_return_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let keyword = self.expect(
            "`return`",
            |kind| matches!(kind, TokenKind::Return),
        )?;

        let argument = if self.at(|kind| {
            matches!(
                kind,
                TokenKind::RBrace
                    | TokenKind::Comma
                    | TokenKind::Eof
            )
        }) {
            None
        } else {
            Some(self.parse_assignment_expression()?)
        };

        let end = argument
            .as_ref()
            .map_or(keyword.span.end, |expression| {
                expression.span().end
            });

        Ok(Statement::Return(ReturnStatement {
            span: Span::new(keyword.span.start, end),
            argument,
        }))
    }

    fn parse_break_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let keyword = self.expect(
            "`break`",
            |kind| matches!(kind, TokenKind::Break),
        )?;

        Ok(Statement::Break(BreakStatement {
            span: keyword.span,
            label: None,
        }))
    }

    fn parse_continue_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let keyword = self.expect(
            "`continue`",
            |kind| matches!(kind, TokenKind::Continue),
        )?;

        Ok(Statement::Continue(ContinueStatement {
            span: keyword.span,
            label: None,
        }))
    }

    fn parse_expression_statement(
        &mut self,
    ) -> Result<Statement, ParseError> {
        let expression = self.parse_assignment_expression()?;
        let span = expression.span();

        Ok(Statement::Expression(ExpressionStatement {
            span,
            expression,
        }))
    }

    fn parse_condition_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let previous = self.allow_component_children;
        self.allow_component_children = false;

        let result = self.parse_assignment_expression();

        self.allow_component_children = previous;

        result
    }

    fn parse_assignment_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let left = self.parse_or_expression()?;

        let op = match self.current().kind {
            TokenKind::Assign => AssignOp::Assign,
            TokenKind::PlusAssign => AssignOp::AddAssign,
            _ => return Ok(left),
        };

        self.advance();

        let right = self.parse_assignment_expression()?;
        let span = Span::new(
            left.span().start,
            right.span().end,
        );

        Ok(Expression::Assign(AssignmentExpression {
            span,
            op,
            target: Box::new(left),
            value: Box::new(right),
        }))
    }

    fn parse_or_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression = self.parse_and_expression()?;

        while self.at(|kind| matches!(kind, TokenKind::Or)) {
            self.advance();

            let right = self.parse_and_expression()?;
            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                BinaryOp::Or,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_and_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression = self.parse_equality_expression()?;

        while self.at(|kind| matches!(kind, TokenKind::And)) {
            self.advance();

            let right = self.parse_equality_expression()?;
            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                BinaryOp::And,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_equality_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression = self.parse_comparison_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };

            self.advance();

            let right = self.parse_comparison_expression()?;
            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                op,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_comparison_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
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
            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                op,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_additive_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression =
            self.parse_multiplicative_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };

            self.advance();

            let right =
                self.parse_multiplicative_expression()?;

            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                op,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_multiplicative_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression = self.parse_unary_expression()?;

        loop {
            let op = match self.current().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                _ => break,
            };

            self.advance();

            let right = self.parse_unary_expression()?;
            let span = Span::new(
                expression.span().start,
                right.span().end,
            );

            expression = Expression::binary(
                expression,
                op,
                right,
                span,
            );
        }

        Ok(expression)
    }

    fn parse_unary_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        if self.at(|kind| matches!(kind, TokenKind::Not)) {
            let operator = self.advance();
            let argument = self.parse_unary_expression()?;

            return Ok(Expression::Unary(UnaryExpression {
                span: Span::new(
                    operator.span.start,
                    argument.span().end,
                ),
                op: UnaryOp::Not,
                argument: Box::new(argument),
            }));
        }

        self.parse_postfix_expression()
    }

    fn parse_postfix_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let mut expression = self.parse_primary_expression()?;

        loop {
            if self.at(|kind| matches!(kind, TokenKind::LParen)) {
                expression =
                    self.parse_call_expression(expression)?;

                continue;
            }

            if self.allow_component_children
                && self.at(|kind| {
                matches!(kind, TokenKind::LBrace)
            })
                && Self::is_component_head(&expression)
            {
                expression =
                    self.parse_component_expression(expression)?;

                continue;
            }

            if self.at(|kind| matches!(kind, TokenKind::Dot)) {
                expression =
                    self.parse_member_expression(expression)?;

                continue;
            }

            if self.at(|kind| matches!(kind, TokenKind::LBracket)) {
                expression =
                    self.parse_index_expression(expression)?;

                continue;
            }

            break;
        }

        Ok(expression)
    }

    fn parse_call_expression(
        &mut self,
        callee: Expression,
    ) -> Result<Expression, ParseError> {
        self.expect(
            "`(`",
            |kind| matches!(kind, TokenKind::LParen),
        )?;

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

        let closing = self.expect(
            "`)`",
            |kind| matches!(kind, TokenKind::RParen),
        )?;

        let span = Span::new(
            callee.span().start,
            closing.span.end,
        );

        Ok(Expression::Call(CallExpression {
            span,
            callee: Box::new(callee),
            args,
        }))
    }

    fn parse_call_argument(
        &mut self,
    ) -> Result<CallArg, ParseError> {
        if self.is_named_argument() {
            let (name, name_span) =
                self.expect_identifier("an argument name")?;

            self.expect(
                "`:`",
                |kind| matches!(kind, TokenKind::Colon),
            )?;

            let value = self.parse_assignment_expression()?;
            let span = Span::new(
                name_span.start,
                value.span().end,
            );

            return Ok(CallArg::Named {
                name,
                value: Box::new(value),
                span,
            });
        }

        Ok(CallArg::Positional(
            self.parse_assignment_expression()?,
        ))
    }

    fn parse_component_expression(
        &mut self,
        head: Expression,
    ) -> Result<Expression, ParseError> {
        let start = head.span().start;

        let (name, args) = match head {
            Expression::Ident(identifier) => {
                (identifier.name, Vec::new())
            }

            Expression::Call(call) => {
                let CallExpression {
                    callee,
                    args,
                    ..
                } = call;

                let Expression::Ident(identifier) = *callee else {
                    return Err(self.expected(
                        "a component name before `{`",
                    ));
                };

                (identifier.name, args)
            }

            _ => {
                return Err(self.expected(
                    "a component name before `{`",
                ));
            }
        };

        let (brace_span, children) =
            self.parse_braced_expressions()?;

        Ok(Expression::Component(ComponentExpression {
            span: Span::new(start, brace_span.end),
            name,
            args,
            children,
        }))
    }

    fn parse_member_expression(
        &mut self,
        object: Expression,
    ) -> Result<Expression, ParseError> {
        self.expect(
            "`.`",
            |kind| matches!(kind, TokenKind::Dot),
        )?;

        let (property, property_span) =
            self.expect_identifier(
                "a property name after `.`",
            )?;

        let span = Span::new(
            object.span().start,
            property_span.end,
        );

        Ok(Expression::Member(MemberExpression {
            span,
            object: Box::new(object),
            property,
        }))
    }

    fn parse_index_expression(
        &mut self,
        object: Expression,
    ) -> Result<Expression, ParseError> {
        self.expect(
            "`[`",
            |kind| matches!(kind, TokenKind::LBracket),
        )?;

        let index = self.parse_assignment_expression()?;

        let closing = self.expect(
            "`]`",
            |kind| matches!(kind, TokenKind::RBracket),
        )?;

        let span = Span::new(
            object.span().start,
            closing.span.end,
        );

        Ok(Expression::Index(IndexExpression {
            span,
            object: Box::new(object),
            index: Box::new(index),
        }))
    }

    fn parse_primary_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
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
            return self.parse_block_expression();
        }

        let token = self.advance();
        let span = token.span;

        match token.kind {
            TokenKind::String(value) => {
                Ok(Expression::literal(
                    LiteralKind::String(value),
                    span,
                ))
            }

            TokenKind::Number(value) => {
                Ok(Expression::literal(
                    LiteralKind::Number(NumberLiteral(value)),
                    span,
                ))
            }

            TokenKind::Percent(value) => {
                Ok(Expression::literal(
                    LiteralKind::Percent(NumberLiteral(value)),
                    span,
                ))
            }

            TokenKind::True => {
                Ok(Expression::literal(
                    LiteralKind::Boolean(true),
                    span,
                ))
            }

            TokenKind::False => {
                Ok(Expression::literal(
                    LiteralKind::Boolean(false),
                    span,
                ))
            }

            TokenKind::Null => {
                Ok(Expression::literal(
                    LiteralKind::Null,
                    span,
                ))
            }

            TokenKind::Ident(name) => {
                Ok(Expression::ident(name, span))
            }

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an expression",
                    found,
                },
                span,
            )),
        }
    }

    fn parse_dot_identifier_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let dot = self.expect(
            "`.`",
            |kind| matches!(kind, TokenKind::Dot),
        )?;

        let (name, name_span) =
            self.expect_identifier(
                "an identifier after `.`",
            )?;

        Ok(Expression::DotIdent(
            DotIdentifierExpression {
                span: Span::new(
                    dot.span.start,
                    name_span.end,
                ),
                name,
            },
        ))
    }

    fn parse_group_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let opening = self.expect(
            "`(`",
            |kind| matches!(kind, TokenKind::LParen),
        )?;

        let expression = self.parse_assignment_expression()?;

        let closing = self.expect(
            "`)`",
            |kind| matches!(kind, TokenKind::RParen),
        )?;

        Ok(Expression::Group(GroupExpression {
            span: Span::new(
                opening.span.start,
                closing.span.end,
            ),
            expression: Box::new(expression),
        }))
    }

    fn parse_list_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let opening = self.expect(
            "`[`",
            |kind| matches!(kind, TokenKind::LBracket),
        )?;

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

            elems.push(Some(
                self.parse_assignment_expression()?,
            ));

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            } else {
                break;
            }
        }

        let closing = self.expect(
            "`]`",
            |kind| matches!(kind, TokenKind::RBracket),
        )?;

        Ok(Expression::List(ListExpression {
            span: Span::new(
                opening.span.start,
                closing.span.end,
            ),
            elems,
        }))
    }

    fn parse_block_expression(
        &mut self,
    ) -> Result<Expression, ParseError> {
        let opening = self.expect(
            "`{`",
            |kind| matches!(kind, TokenKind::LBrace),
        )?;

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

            let expression =
                self.parse_assignment_expression()?;

            if self.at(|kind| matches!(kind, TokenKind::RBrace)) {
                tail = Some(Box::new(expression));
                break;
            }

            let span = expression.span();

            statements.push(
                Statement::Expression(ExpressionStatement {
                    span,
                    expression,
                }),
            );

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect(
            "`}`",
            |kind| matches!(kind, TokenKind::RBrace),
        )?;

        Ok(Expression::Block(BlockExpression {
            span: Span::new(
                opening.span.start,
                closing.span.end,
            ),
            statements,
            tail,
        }))
    }

    fn parse_braced_expressions(
        &mut self,
    ) -> Result<(Span, Vec<Expression>), ParseError> {
        let opening = self.expect(
            "`{`",
            |kind| matches!(kind, TokenKind::LBrace),
        )?;

        let mut expressions = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            expressions.push(
                self.parse_assignment_expression()?,
            );

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect(
            "`}`",
            |kind| matches!(kind, TokenKind::RBrace),
        )?;

        Ok((
            Span::new(
                opening.span.start,
                closing.span.end,
            ),
            expressions,
        ))
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

    fn starts_statement(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Let
                | TokenKind::If
                | TokenKind::While
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
        )
    }

    fn is_component_head(expression: &Expression) -> bool {
        match expression {
            Expression::Ident(_) => true,

            Expression::Call(call) => {
                matches!(
                    call.callee.as_ref(),
                    Expression::Ident(_)
                )
            }

            _ => false,
        }
    }

    fn is_named_argument(&self) -> bool {
        matches!(
            (&self.current().kind, &self.next().kind),
            (TokenKind::Ident(_), TokenKind::Colon)
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

    fn next(&self) -> &Token {
        self.tokens
            .get(self.position + 1)
            .unwrap_or_else(|| {
                self.tokens
                    .last()
                    .expect(
                        "parser token stream must contain EOF",
                    )
            })
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