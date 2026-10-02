use crate::token::TemplateTokenPart;
use crate::{
    error::{ParseError, ParseErrorKind},
    token::{Token, TokenKind},
};
use kome_ast::declarations::{Path, PathSegment, PathSegmentKind, PathSeparator, UseImport};
use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, Binding, ComponentDeclaration, ComponentMember, Declaration, EnumCase,
        EnumDeclaration, ExternDeclaration, ExternItem, ForDeclaration, FunctionDeclaration,
        GenericParameter, Module, RecipeDeclaration, StructDeclaration, StructField,
        TraitDeclaration, TypeMember, UseDeclaration, Visibility,
    },
    expressions::{
        AssignOp, AssignmentExpression, BinaryOp, BlockExpression, CallArg, CallExpression,
        CancelExpression, ClosureExpression, ComponentExpression, DotIdentifierExpression,
        Expression, GroupExpression, IndexExpression, KeyValueProperty, ListExpression,
        LiteralKind, MemberExpression, NumberLiteral, ObjectExpression, ObjectProperty,
        PropertyKey, StructExpression, TaskExpression, TemplateExpression, TemplatePart,
        UnaryExpression, UnaryOp, UnwrapExpression, WaitExpression,
    },
    patterns::{DotIdentPattern, IdentifierPattern, IsPattern, LiteralPattern, Pattern},
    statements::{
        BlockStatement, BreakStatement, ContinueStatement, ExpressionStatement, ForInStatement,
        IfStatement, IsStatement, ReturnStatement, Statement, WhileStatement,
    },
    types::{
        NamedType, OptionalType, Parameter, PointerMutability, PointerType, PrimitiveType,
        PrimitiveTypeKind, Type,
    },
};

fn implementation_type_parameters(target: &Type) -> Vec<GenericParameter> {
    let mut parameters = Vec::new();
    if let Type::Named(named) = target {
        for argument in &named.type_arguments {
            if let Type::Named(parameter) = argument
                && parameter.type_arguments.is_empty()
                && !parameters
                    .iter()
                    .any(|value: &GenericParameter| value.name == parameter.name)
            {
                parameters.push(GenericParameter {
                    span: parameter.span,
                    name: parameter.name.clone(),
                });
            }
        }
    }
    parameters
}

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
    allow_component_children: bool,
}

impl Parser {
    /// Creates a parser and ensures that its token stream ends with EOF.
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

    /// Parses the complete token stream as a Kome module.
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

    /// Parses exactly one expression followed by EOF.
    pub fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        let expression = self.parse_assignment_expression()?;

        if !self.current().is_eof() {
            return Err(self.expected("the end of the expression"));
        }

        Ok(expression)
    }

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        let attributes = self.parse_attributes()?;
        let visibility = self.parse_visibility()?;

        if self.at(|kind| matches!(kind, TokenKind::Component)) {
            let mut declaration = self.parse_component_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(Declaration::Component(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Struct)) {
            let mut declaration = self.parse_struct_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(Declaration::Struct(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Trait)) && attributes.is_empty() {
            let mut declaration = self.parse_trait_declaration()?;
            declaration.visibility = visibility;
            return Ok(Declaration::Trait(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::For))
            && attributes.is_empty()
            && visibility == Visibility::Private
        {
            return self.parse_for_declaration().map(Declaration::For);
        }

        if self.at(|kind| matches!(kind, TokenKind::Enum)) {
            let mut declaration = self.parse_enum_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(Declaration::Enum(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Fn)) {
            let mut declaration = self.parse_function_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(Declaration::Function(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Let)) && visibility == Visibility::Private {
            return self.parse_let_binding(attributes).map(Declaration::Let);
        }

        if self.at(|kind| matches!(kind, TokenKind::Var)) && visibility == Visibility::Private {
            return self.parse_var_binding(attributes).map(Declaration::Let);
        }

        if self.at(|kind| matches!(kind, TokenKind::Const)) {
            let mut declaration = self.parse_const_binding(attributes)?;
            declaration.visibility = visibility;
            return Ok(Declaration::Constant(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Use)) {
            if attributes.is_empty() {
                let mut declaration = self.parse_use_declaration()?;
                declaration.visibility = visibility;
                return Ok(Declaration::Use(declaration));
            }

            return Err(self.expected(
                "a component, enum, function, let, or var declaration after attributes",
            ));
        }

        if self.at(|kind| matches!(kind, TokenKind::Extern)) {
            if attributes.is_empty() && visibility == Visibility::Private {
                return self.parse_extern_declaration().map(Declaration::Extern);
            }
            return Err(self.expected("an external declaration without attributes"));
        }

        if attributes.is_empty() {
            Err(self.expected("a top-level declaration"))
        } else {
            Err(self.expected("a declaration after attributes"))
        }
    }

    fn parse_visibility(&mut self) -> Result<Visibility, ParseError> {
        if !self.at(|kind| matches!(kind, TokenKind::Pub)) {
            return Ok(Visibility::Private);
        }

        self.advance();
        if !self.at(|kind| matches!(kind, TokenKind::LParen)) {
            return Ok(Visibility::Public);
        }

        self.advance();
        let (scope, _) = self.expect_identifier("`package` in a visibility modifier")?;
        if scope != "package" {
            return Err(self.expected("`package` in a visibility modifier"));
        }
        self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;
        Ok(Visibility::Package)
    }

    fn parse_extern_declaration(&mut self) -> Result<ExternDeclaration, ParseError> {
        let keyword = self.expect("`extern`", |kind| matches!(kind, TokenKind::Extern))?;
        let abi_token =
            self.expect("an ABI string", |kind| matches!(kind, TokenKind::String(_)))?;
        let TokenKind::String(abi) = abi_token.kind else {
            unreachable!("the ABI token kind was checked");
        };
        let library = if self.at(|kind| matches!(kind, TokenKind::From)) {
            self.advance();
            let token = self.expect("a library string after `from`", |kind| {
                matches!(kind, TokenKind::String(_))
            })?;
            let TokenKind::String(library) = token.kind else {
                unreachable!("the library token kind was checked");
            };
            Some(library)
        } else {
            None
        };
        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;
        let mut items = Vec::new();
        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }
            let visibility = self.parse_visibility()?;
            if self.at(|kind| matches!(kind, TokenKind::Struct)) {
                let mut declaration = self.parse_struct_declaration(Vec::new())?;
                declaration.visibility = visibility;
                items.push(ExternItem::Struct(declaration));
            } else if self.at(|kind| matches!(kind, TokenKind::Fn)) {
                let mut function = self.parse_function_declaration(Vec::new())?;
                function.visibility = visibility;
                if function.body.is_some() {
                    return Err(ParseError::new(
                        ParseErrorKind::Expected {
                            expected: "an external function declaration without a body",
                            found: TokenKind::LBrace,
                        },
                        function.span,
                    ));
                }
                items.push(ExternItem::Function(function));
            } else {
                return Err(self.expected("an external `struct` or `fn` declaration"));
            }
            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }
        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;
        Ok(ExternDeclaration {
            span: Span::new(keyword.span.start, closing.span.end),
            abi,
            library,
            items,
        })
    }

    fn parse_struct_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<StructDeclaration, ParseError> {
        let keyword = self.expect("`struct`", |kind| matches!(kind, TokenKind::Struct))?;
        let start = attributes
            .first()
            .map_or(keyword.span.start, |attribute| attribute.span.start);
        let (name, name_span) = self.expect_identifier("a struct name")?;
        let type_parameters = self.parse_generic_parameters()?;

        if !self.at(|kind| matches!(kind, TokenKind::LBrace)) {
            return Ok(StructDeclaration {
                span: Span::new(start, name_span.end),
                visibility: Visibility::Private,
                attributes,
                name,
                type_parameters,
                fields: None,
            });
        }

        self.advance();
        let mut fields = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            let visibility = self.parse_visibility()?;
            let (field_name, field_span) = self.expect_identifier("a struct field")?;
            self.expect("`:`", |kind| matches!(kind, TokenKind::Colon))?;
            let type_ = self.parse_type()?;
            let end = type_.span().end;
            fields.push(StructField {
                span: Span::new(field_span.start, end),
                visibility,
                name: field_name,
                type_,
            });

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            if !self.at(|kind| matches!(kind, TokenKind::RBrace | TokenKind::Ident(_))) {
                return Err(self.expected("`,` or `}` after a struct field"));
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(StructDeclaration {
            span: Span::new(start, closing.span.end),
            visibility: Visibility::Private,
            attributes,
            name,
            type_parameters,
            fields: Some(fields),
        })
    }

    fn parse_for_declaration(&mut self) -> Result<ForDeclaration, ParseError> {
        let keyword = self.expect("`for`", |kind| matches!(kind, TokenKind::For))?;
        let target = self.parse_type()?;
        let type_parameters = implementation_type_parameters(&target);
        let trait_ = if self.at(|kind| matches!(kind, TokenKind::Colon)) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let (members, end) = self.parse_type_members()?;

        Ok(ForDeclaration {
            span: Span::new(keyword.span.start, end),
            type_parameters,
            target,
            trait_,
            members,
        })
    }

    fn parse_trait_declaration(&mut self) -> Result<TraitDeclaration, ParseError> {
        let keyword = self.expect("`trait`", |kind| matches!(kind, TokenKind::Trait))?;
        let (name, _) = self.expect_identifier("a trait name")?;
        let type_parameters = self.parse_generic_parameters()?;
        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;
        let mut functions = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            let attributes = self.parse_attributes()?;
            let visibility = self.parse_visibility()?;
            if !self.at(|kind| matches!(kind, TokenKind::Fn)) {
                return Err(self.expected("a `fn` trait member"));
            }
            let mut function = self.parse_function_declaration(attributes)?;
            function.visibility = match visibility {
                Visibility::Private => Visibility::Public,
                visibility => visibility,
            };
            functions.push(function);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;
        Ok(TraitDeclaration {
            span: Span::new(keyword.span.start, closing.span.end),
            visibility: Visibility::Private,
            name,
            type_parameters,
            functions,
        })
    }

    fn parse_type_members(&mut self) -> Result<(Vec<TypeMember>, usize), ParseError> {
        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;
        let mut members = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            members.push(self.parse_type_member()?);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;
        Ok((members, closing.span.end))
    }

    fn parse_type_member(&mut self) -> Result<TypeMember, ParseError> {
        let attributes = self.parse_attributes()?;
        let visibility = self.parse_visibility()?;

        if self.at(|kind| matches!(kind, TokenKind::Const)) {
            let mut declaration = self.parse_const_binding(attributes)?;
            declaration.visibility = visibility;
            return Ok(TypeMember::Constant(declaration));
        }

        if self.at(|kind| matches!(kind, TokenKind::Fn)) {
            let mut declaration = self.parse_function_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(TypeMember::Function(declaration));
        }

        Err(self.expected("a `const` or `fn` type member"))
    }

    fn parse_enum_declaration(
        &mut self,
        attributes: Vec<Attribute>,
    ) -> Result<EnumDeclaration, ParseError> {
        let enum_token = self.expect("`enum`", |kind| matches!(kind, TokenKind::Enum))?;

        let start = attributes
            .first()
            .map_or(enum_token.span.start, |attribute| attribute.span.start);

        let (name, _) = self.expect_identifier("an enum name")?;

        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;

        let mut cases = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            let (case_name, case_name_span) = self.expect_identifier("an enum case name")?;

            let value = if self.at(|kind| matches!(kind, TokenKind::Assign)) {
                self.advance();

                Some(self.parse_assignment_expression()?)
            } else {
                None
            };

            let case_end = value
                .as_ref()
                .map_or(case_name_span.end, |expression| expression.span().end);

            cases.push(EnumCase {
                span: Span::new(case_name_span.start, case_end),
                name: case_name,
                value,
            });

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
                continue;
            }

            if !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
                return Err(self.expected("`,` or `}` after an enum case"));
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(EnumDeclaration {
            span: Span::new(start, closing.span.end),
            visibility: Visibility::Private,
            attributes,
            name,
            cases,
        })
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

        let closing_parenthesis = self.expect("`)`", |kind| matches!(kind, TokenKind::RParen))?;

        if !self.at(|kind| matches!(kind, TokenKind::LBrace)) {
            return Ok(ComponentDeclaration {
                span: Span::new(start, closing_parenthesis.span.end),
                visibility: Visibility::Private,
                name,
                params,
                attributes,
                body: None,
            });
        }

        self.advance();

        let mut members = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            members.push(self.parse_component_member()?);
        }

        let closing_brace = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(ComponentDeclaration {
            span: Span::new(start, closing_brace.span.end),
            visibility: Visibility::Private,
            name,
            params,
            attributes,
            body: Some(members),
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
        let visibility = self.parse_visibility()?;

        if self.at(|kind| matches!(kind, TokenKind::State)) {
            let mut binding = self.parse_state_binding(attributes)?;
            binding.visibility = visibility;
            return Ok(ComponentMember::State(Box::new(binding)));
        }

        if self.at(|kind| matches!(kind, TokenKind::Let)) {
            let mut binding = self.parse_let_binding(attributes)?;
            binding.visibility = visibility;
            return Ok(ComponentMember::Let(Box::new(binding)));
        }

        if self.at(|kind| matches!(kind, TokenKind::Var)) {
            let mut binding = self.parse_var_binding(attributes)?;
            binding.visibility = visibility;
            return Ok(ComponentMember::Let(Box::new(binding)));
        }

        if self.at(|kind| matches!(kind, TokenKind::Recipe)) && visibility == Visibility::Private {
            return self
                .parse_recipe_declaration(attributes)
                .map(ComponentMember::Recipe);
        }

        if self.at(|kind| matches!(kind, TokenKind::Fn)) {
            let mut declaration = self.parse_function_declaration(attributes)?;
            declaration.visibility = visibility;
            return Ok(ComponentMember::Function(declaration));
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
        let type_parameters = self.parse_generic_parameters()?;

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
            visibility: Visibility::Private,
            attributes,
            name,
            type_parameters,
            params,
            body,
            return_type,
        })
    }

    fn parse_generic_parameters(&mut self) -> Result<Vec<GenericParameter>, ParseError> {
        if !self.at(|kind| matches!(kind, TokenKind::Lt)) {
            return Ok(Vec::new());
        }
        self.advance();
        let mut parameters = Vec::new();
        if self.at(|kind| matches!(kind, TokenKind::Gt)) {
            return Err(self.expected("a generic parameter"));
        }
        loop {
            let (name, span) = self.expect_identifier("a generic parameter")?;
            parameters.push(GenericParameter { span, name });
            if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                break;
            }
            self.advance();
        }
        self.expect("`>`", |kind| matches!(kind, TokenKind::Gt))?;
        Ok(parameters)
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
        let token = self.advance();
        let name_span = token.span;
        let name = match token.kind {
            TokenKind::Ident(name) => name,
            TokenKind::Self_ => "self".to_owned(),
            found => {
                return Err(ParseError::new(
                    ParseErrorKind::Expected {
                        expected: "a parameter name",
                        found,
                    },
                    name_span,
                ));
            }
        };

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

        self.parse_binding_after_keyword(attributes, keyword.span.start, true)
    }

    fn parse_let_binding(&mut self, attributes: Vec<Attribute>) -> Result<Binding, ParseError> {
        let keyword = self.expect("`let`", |kind| matches!(kind, TokenKind::Let))?;

        let binding = self.parse_binding_after_keyword(attributes, keyword.span.start, false)?;

        if binding.init.is_none() {
            return Err(self.expected("an initializer for a `let` binding"));
        }

        Ok(binding)
    }

    fn parse_var_binding(&mut self, attributes: Vec<Attribute>) -> Result<Binding, ParseError> {
        let keyword = self.expect("`var`", |kind| matches!(kind, TokenKind::Var))?;

        let binding = self.parse_binding_after_keyword(attributes, keyword.span.start, true)?;

        if binding.init.is_none() && binding.type_annotation.is_none() {
            return Err(self.expected("a type annotation or initializer for a `var` binding"));
        }

        Ok(binding)
    }

    fn parse_const_binding(&mut self, attributes: Vec<Attribute>) -> Result<Binding, ParseError> {
        let keyword = self.expect("`const`", |kind| matches!(kind, TokenKind::Const))?;
        self.parse_binding_after_keyword(attributes, keyword.span.start, false)
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
            visibility: Visibility::Private,
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
        let mut type_ = if self.at(|kind| matches!(kind, TokenKind::Star)) {
            let star = self.advance();
            let mutability = if self.at(|kind| matches!(kind, TokenKind::Const)) {
                self.advance();
                PointerMutability::Const
            } else if self.at(|kind| matches!(kind, TokenKind::Mut)) {
                self.advance();
                PointerMutability::Mut
            } else {
                return Err(self.expected("`const` or `mut` after `*`"));
            };
            let pointee = self.parse_type()?;
            Type::Pointer(PointerType {
                span: Span::new(star.span.start, pointee.span().end),
                mutability,
                pointee: Box::new(pointee),
            })
        } else {
            self.parse_primary_type()?
        };

        while self.at(|kind| matches!(kind, TokenKind::LBracket))
            && matches!(self.next().kind, TokenKind::RBracket)
        {
            self.advance();
            let close = self.advance();
            let start = type_.span().start;
            type_ = Type::List(kome_ast::types::ListType {
                span: Span::new(start, close.span.end),
                element: Box::new(type_),
            });
        }

        if self.at(|kind| matches!(kind, TokenKind::Question)) {
            let question = self.advance();
            let start = type_.span().start;

            type_ = Type::Optional(OptionalType {
                span: Span::new(start, question.span.end),
                inner: Box::new(type_),
            });
        }

        Ok(type_)
    }

    fn parse_primary_type(&mut self) -> Result<Type, ParseError> {
        let (mut name, name_span) = self.expect_identifier("a type name")?;
        let mut end = name_span.end;

        while self.at(|kind| matches!(kind, TokenKind::ColonColon)) {
            self.advance();
            let (segment, segment_span) = self.expect_identifier("a type path segment")?;
            name.push_str("::");
            name.push_str(&segment);
            end = segment_span.end;
        }

        let primitive_kind = match name.as_str() {
            "String" => Some(PrimitiveTypeKind::String),

            "Number" => Some(PrimitiveTypeKind::Number),

            "bool" => Some(PrimitiveTypeKind::Bool),

            "i8" => Some(PrimitiveTypeKind::I8),

            "i16" => Some(PrimitiveTypeKind::I16),

            "i32" => Some(PrimitiveTypeKind::I32),

            "i64" => Some(PrimitiveTypeKind::I64),

            "u8" => Some(PrimitiveTypeKind::U8),

            "u16" => Some(PrimitiveTypeKind::U16),

            "u32" => Some(PrimitiveTypeKind::U32),

            "u64" => Some(PrimitiveTypeKind::U64),

            "isize" => Some(PrimitiveTypeKind::Isize),

            "usize" => Some(PrimitiveTypeKind::Usize),

            "f32" => Some(PrimitiveTypeKind::F32),

            "f64" => Some(PrimitiveTypeKind::F64),

            "Null" => Some(PrimitiveTypeKind::Null),

            _ => None,
        };

        if let Some(kind) = primitive_kind {
            if self.at(|kind| matches!(kind, TokenKind::Lt)) {
                return Err(self.expected("the end of a primitive type"));
            }

            return Ok(Type::Primitive(PrimitiveType {
                span: Span::new(name_span.start, end),
                kind,
            }));
        }

        let mut type_arguments = Vec::new();

        if self.at(|kind| matches!(kind, TokenKind::Lt)) {
            self.advance();

            if self.at(|kind| matches!(kind, TokenKind::Gt)) {
                return Err(self.expected("a type argument"));
            }

            loop {
                type_arguments.push(self.parse_type()?);

                if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                    break;
                }

                self.advance();

                if self.at(|kind| matches!(kind, TokenKind::Gt)) {
                    break;
                }
            }

            let closing = self.expect("`>`", |kind| matches!(kind, TokenKind::Gt))?;

            end = closing.span.end;
        }

        Ok(Type::Named(NamedType {
            span: Span::new(name_span.start, end),
            name,
            type_arguments,
        }))
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match &self.current().kind {
            TokenKind::Let => self.parse_let_binding(Vec::new()).map(Statement::Let),

            TokenKind::Var => self.parse_var_binding(Vec::new()).map(Statement::Let),

            TokenKind::Const => self.parse_const_binding(Vec::new()).map(Statement::Let),

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
        if self.at(|kind| matches!(kind, TokenKind::Task | TokenKind::Wait | TokenKind::Cancel)) {
            let operator = self.advance();
            let argument = self.parse_unary_expression()?;
            let span = Span::new(operator.span.start, argument.span().end);

            return Ok(match operator.kind {
                TokenKind::Task => Expression::Task(TaskExpression {
                    span,
                    argument: Box::new(argument),
                }),
                TokenKind::Wait => Expression::Wait(WaitExpression {
                    span,
                    argument: Box::new(argument),
                }),
                TokenKind::Cancel => Expression::Cancel(CancelExpression {
                    span,
                    argument: Box::new(argument),
                }),
                _ => unreachable!("the task/wait branch checked the token kind"),
            });
        }

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
        let mut type_arguments = Vec::new();

        loop {
            if self.at(|kind| matches!(kind, TokenKind::Lt))
                && matches!(expression, Expression::Ident(_) | Expression::Member(_))
            {
                let checkpoint = self.position;
                if let Ok(arguments) = self.parse_type_argument_list()
                    && self.at(|kind| {
                        matches!(kind, TokenKind::LParen | TokenKind::LBrace | TokenKind::Dot)
                    })
                {
                    type_arguments = arguments;
                    continue;
                }
                self.position = checkpoint;
            }
            if self.at(|kind| matches!(kind, TokenKind::LParen)) {
                expression =
                    self.parse_call_expression(expression, std::mem::take(&mut type_arguments))?;

                continue;
            }

            if self.at(|kind| matches!(kind, TokenKind::LBrace))
                && matches!(expression, Expression::Ident(_))
                && self.is_struct_expression_start()
            {
                expression =
                    self.parse_struct_expression(expression, std::mem::take(&mut type_arguments))?;

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

            if self.at(|kind| matches!(kind, TokenKind::Not)) {
                let operator = self.advance();
                expression = Expression::Unwrap(UnwrapExpression {
                    span: Span::new(expression.span().start, operator.span.end),
                    argument: Box::new(expression),
                });
                continue;
            }

            break;
        }

        Ok(expression)
    }

    fn parse_type_argument_list(&mut self) -> Result<Vec<Type>, ParseError> {
        self.expect("`<`", |kind| matches!(kind, TokenKind::Lt))?;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.parse_type()?);
            if !self.at(|kind| matches!(kind, TokenKind::Comma)) {
                break;
            }
            self.advance();
        }
        self.expect("`>`", |kind| matches!(kind, TokenKind::Gt))?;
        Ok(arguments)
    }

    fn parse_call_expression(
        &mut self,
        callee: Expression,
        type_arguments: Vec<Type>,
    ) -> Result<Expression, ParseError> {
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
            type_arguments,
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

    fn parse_struct_expression(
        &mut self,
        head: Expression,
        type_arguments: Vec<Type>,
    ) -> Result<Expression, ParseError> {
        let Expression::Ident(identifier) = head else {
            return Err(self.expected("a struct name before `{`"));
        };
        self.expect("`{`", |kind| matches!(kind, TokenKind::LBrace))?;
        let mut fields = Vec::new();

        while !self.at(|kind| matches!(kind, TokenKind::RBrace)) {
            if self.current().is_eof() {
                return Err(self.expected("`}`"));
            }

            let ObjectProperty::KeyValue(field) = self.parse_object_property()?;
            fields.push(field);

            if self.at(|kind| matches!(kind, TokenKind::Comma)) {
                self.advance();
            } else if !self.at(|kind| matches!(kind, TokenKind::RBrace | TokenKind::Ident(_))) {
                return Err(self.expected("`,` or `}` after a struct field value"));
            }
        }

        let closing = self.expect("`}`", |kind| matches!(kind, TokenKind::RBrace))?;

        Ok(Expression::Struct(StructExpression {
            span: Span::new(identifier.span.start, closing.span.end),
            name: identifier.name,
            type_arguments,
            fields,
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

            TokenKind::Template(parts) => self.parse_template_expression(parts, span),

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

            TokenKind::Ident(mut name) => {
                let mut end = span.end;
                while self.at(|kind| matches!(kind, TokenKind::ColonColon)) {
                    self.advance();
                    let (segment, segment_span) =
                        self.expect_identifier("a path segment after `::`")?;
                    name.push_str("::");
                    name.push_str(&segment);
                    end = segment_span.end;
                }
                Ok(Expression::ident(name, Span::new(span.start, end)))
            }

            TokenKind::Self_ => Ok(Expression::ident("self", span)),

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an expression",
                    found,
                },
                span,
            )),
        }
    }

    fn parse_template_expression(
        &mut self,
        token_parts: Vec<TemplateTokenPart>,
        span: Span,
    ) -> Result<Expression, ParseError> {
        let mut parts = Vec::new();

        for token_part in token_parts {
            match token_part {
                TemplateTokenPart::String { value, span } => {
                    parts.push(TemplatePart::String { value, span });
                }

                TemplateTokenPart::Expression { tokens, span } => {
                    let mut parser = Parser::new(tokens);

                    let expression = parser.parse_expression()?;

                    parts.push(TemplatePart::Expression {
                        expression: Box::new(expression),
                        span,
                    });
                }
            }
        }

        Ok(Expression::Template(TemplateExpression { span, parts }))
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

        let mut imports = vec![self.parse_use_import()?];

        while self.at(|kind| matches!(kind, TokenKind::Comma)) {
            self.advance();

            imports.push(self.parse_use_import()?);
        }

        let end = imports
            .last()
            .map(|import| import.span().end)
            .unwrap_or(start);

        Ok(UseDeclaration {
            span: Span::new(start, end),
            visibility: Visibility::Private,
            imports,
        })
    }

    fn parse_use_import(&mut self) -> Result<UseImport, ParseError> {
        let token = self.advance();
        let token_span = token.span;

        match token.kind {
            TokenKind::Star => Ok(UseImport::Wildcard { span: token_span }),

            TokenKind::Ident(_) | TokenKind::Self_ | TokenKind::Super | TokenKind::Task => {
                let kind = match &token.kind {
                    TokenKind::Ident(name) => PathSegmentKind::Ident(name.clone()),
                    TokenKind::Self_ => PathSegmentKind::Self_,
                    TokenKind::Super => PathSegmentKind::Super,
                    TokenKind::Task => PathSegmentKind::Ident("task".to_owned()),
                    _ => unreachable!(),
                };

                let mut segments = vec![PathSegment {
                    span: token_span,
                    kind,
                }];
                let mut separators = Vec::new();

                let start = token_span.start;
                let mut end = token_span.end;

                let mut wildcard_span = None;
                loop {
                    if self.at(|kind| matches!(kind, TokenKind::ColonColon)) {
                        self.advance();
                        if self.at(|kind| matches!(kind, TokenKind::Star)) {
                            let star = self.advance();
                            wildcard_span = Some(Span::new(start, star.span.end));
                            break;
                        }
                        separators.push(PathSeparator::ColonColon);

                        let segment_token = self.advance();
                        let segment_span = segment_token.span;

                        let kind = match &segment_token.kind {
                            TokenKind::Ident(name) => PathSegmentKind::Ident(name.clone()),
                            TokenKind::Self_ => PathSegmentKind::Self_,
                            TokenKind::Super => PathSegmentKind::Super,
                            TokenKind::Task => PathSegmentKind::Ident("task".to_owned()),
                            found => {
                                return Err(ParseError::new(
                                    ParseErrorKind::Expected {
                                        expected: "a path segment after `::`",
                                        found: found.clone(),
                                    },
                                    segment_span,
                                ));
                            }
                        };

                        end = segment_span.end;

                        segments.push(PathSegment {
                            span: segment_span,
                            kind,
                        });
                    } else {
                        break;
                    }
                }

                let path = Path {
                    span: Span::new(start, end),
                    segments,
                    separators,
                };

                if let Some(span) = wildcard_span {
                    return Ok(UseImport::WildcardFrom { path, span });
                }

                let alias = if self.at(|kind| matches!(kind, TokenKind::As)) {
                    self.advance();
                    let (name, span) = self.expect_identifier("an import alias after `as`")?;
                    Some(PathSegment {
                        span,
                        kind: PathSegmentKind::Ident(name),
                    })
                } else {
                    None
                };

                match alias {
                    Some(alias) => Ok(UseImport::AliasedModule { path, alias }),
                    None => Ok(UseImport::Module(path)),
                }
            }

            found => Err(ParseError::new(
                ParseErrorKind::Expected {
                    expected: "an import name or `*`",

                    found,
                },
                token_span,
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

    fn is_struct_expression_start(&self) -> bool {
        matches!(
            (
                self.tokens.get(self.position + 1).map(|token| &token.kind),
                self.tokens.get(self.position + 2).map(|token| &token.kind),
            ),
            (Some(TokenKind::Ident(_)), Some(TokenKind::Colon))
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
