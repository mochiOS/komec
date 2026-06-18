//! Demo: constructing and printing a Kome component AST.
//!
//! Builds the equivalent of:
//!
//! ```kome
//! @application
//! component App() {
//!     state counter = 0
//!
//!     @body
//!     let body: View = VStack {
//!         Text("Hello")
//!     }
//!
//!     recipe increment {
//!         counter = counter + 1
//!     }
//!
//!     fn greet(name: String) {
//!         return "Hello, " + name
//!     }
//! }
//! ```

use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, Binding, ComponentDeclaration, ComponentMember, Declaration,
        FunctionDeclaration, Module, RecipeDeclaration,
    },
    expressions::{
        AssignOp, AssignmentExpression, BinaryOp, CallArg, CallExpression,
        ComponentExpression, Expression, LiteralKind, NumberLiteral,
    },
    patterns::{IdentifierPattern, Pattern},
    statements::{
        BlockStatement, ExpressionStatement, ReturnStatement, Statement,
    },
    types::{
        NamedType, PrimitiveType, PrimitiveTypeKind, Type,
    },
};

const SOURCE: &str = r#"@application
component App() {
    state counter = 0

    @body
    let body: View = VStack {
        Text("Hello")
    }

    recipe increment {
        counter = counter + 1
    }

    fn greet(name: String) {
        return "Hello, " + name
    }
}"#;

fn main() {
    // Component attribute: @application

    let application_attribute = Attribute {
        span: Span::new(0, 12),
        name: "application".into(),
        args: Vec::new(),
    };

    // State: state counter = 0

    let counter_pattern = Pattern::Ident(IdentifierPattern {
        span: Span::new(41, 48),
        name: "counter".into(),
        type_annotation: None,
        default: None,
    });

    let counter_initial_value = Expression::literal(
        LiteralKind::Number(NumberLiteral("0".into())),
        Span::new(51, 52),
    );

    let counter_state = ComponentMember::State(Box::new(Binding {
        span: Span::new(35, 52),
        attributes: Vec::new(),
        mutable: false,
        pattern: counter_pattern,
        init: Some(counter_initial_value),
        type_annotation: None,
    }));

    // Body attribute: @body

    let body_attribute = Attribute {
        span: Span::new(58, 63),
        name: "body".into(),
        args: Vec::new(),
    };

    // Text("Hello")

    let text_callee = Expression::ident(
        "Text",
        Span::new(102, 106),
    );

    let text_argument = Expression::literal(
        LiteralKind::String("Hello".into()),
        Span::new(107, 114),
    );

    let text_expression = Expression::Call(CallExpression {
        span: Span::new(102, 115),
        callee: Box::new(text_callee),
        args: vec![
            CallArg::Positional(text_argument),
        ],
    });

    // VStack { Text("Hello") }

    let body_initial_value = Expression::Component(ComponentExpression {
        span: Span::new(85, 121),
        name: "VStack".into(),
        args: Vec::new(),
        children: vec![
            text_expression,
        ],
    });

    // @body let body: View = VStack { ... }

    let body_pattern = Pattern::Ident(IdentifierPattern {
        span: Span::new(72, 76),
        name: "body".into(),
        type_annotation: None,
        default: None,
    });

    let body_type = Type::Named(NamedType {
        span: Span::new(78, 82),
        name: "View".into(),
        type_arguments: Vec::new(),
    });

    let body_binding = ComponentMember::Let(Box::new(Binding {
        span: Span::new(68, 121),
        attributes: vec![
            body_attribute,
        ],
        mutable: false,
        pattern: body_pattern,
        init: Some(body_initial_value),
        type_annotation: Some(body_type),
    }));

    // Recipe: counter = counter + 1

    let assignment_target = Expression::ident(
        "counter",
        Span::new(154, 161),
    );

    let addition_left = Expression::ident(
        "counter",
        Span::new(164, 171),
    );

    let addition_right = Expression::literal(
        LiteralKind::Number(NumberLiteral("1".into())),
        Span::new(174, 175),
    );

    let addition = Expression::binary(
        addition_left,
        BinaryOp::Add,
        addition_right,
        Span::new(164, 175),
    );

    let assignment = Expression::Assign(AssignmentExpression {
        span: Span::new(154, 175),
        op: AssignOp::Assign,
        target: Box::new(assignment_target),
        value: Box::new(addition),
    });

    let assignment_statement = Statement::Expression(
        ExpressionStatement {
            span: Span::new(154, 175),
            expression: assignment,
        },
    );

    let increment_recipe = ComponentMember::Recipe(
        RecipeDeclaration {
            span: Span::new(127, 181),
            attributes: Vec::new(),
            name: "increment".into(),
            event_source: None,
            body: BlockStatement {
                span: Span::new(144, 181),
                statements: vec![
                    assignment_statement,
                ],
            },
        },
    );

    // Function parameter: name: String

    let name_parameter = Pattern::Ident(IdentifierPattern {
        span: Span::new(196, 208),
        name: "name".into(),
        type_annotation: Some(Type::Primitive(PrimitiveType {
            span: Span::new(202, 208),
            kind: PrimitiveTypeKind::String,
        })),
        default: None,
    });

    // Function expression: "Hello, " + name

    let hello_expression = Expression::literal(
        LiteralKind::String("Hello, ".into()),
        Span::new(227, 236),
    );

    let name_expression = Expression::ident(
        "name",
        Span::new(239, 243),
    );

    let greeting_expression = Expression::binary(
        hello_expression,
        BinaryOp::Add,
        name_expression,
        Span::new(227, 243),
    );

    let return_statement = Statement::Return(ReturnStatement {
        span: Span::new(220, 243),
        argument: Some(greeting_expression),
    });

    // Component function: greet

    let greet_function = ComponentMember::Function(
        FunctionDeclaration {
            span: Span::new(187, 249),
            attributes: Vec::new(),
            name: "greet".into(),
            params: vec![
                name_parameter,
            ],
            body: Some(BlockStatement {
                span: Span::new(210, 249),
                statements: vec![
                    return_statement,
                ],
            }),
            return_type: None,
        },
    );

    // Component

    let component = Declaration::Component(ComponentDeclaration {
        span: Span::new(0, SOURCE.len()),
        name: "App".into(),
        params: Vec::new(),
        attributes: vec![
            application_attribute,
        ],
        body: vec![
            counter_state,
            body_binding,
            increment_recipe,
            greet_function,
        ],
    });

    // Module

    let module = Module::new(
        vec![component],
        Span::new(0, SOURCE.len()),
    );

    println!("{module:#?}");
    print_span("module", &module);

    // Span verification

    debug_assert_eq!(
        &SOURCE[0..12],
        "@application",
    );

    debug_assert_eq!(
        &SOURCE[35..52],
        "state counter = 0",
    );

    debug_assert_eq!(
        &SOURCE[68..121],
        "let body: View = VStack {\n        Text(\"Hello\")\n    }",
    );

    debug_assert_eq!(
        &SOURCE[127..181],
        "recipe increment {\n        counter = counter + 1\n    }",
    );

    debug_assert_eq!(
        &SOURCE[187..249],
        "fn greet(name: String) {\n        return \"Hello, \" + name\n    }",
    );
}

/// Prints the source span of any AST node implementing `AstNode`.
fn print_span(label: &str, node: &impl AstNode) {
    let span = node.span();

    println!(
        "{label}: [{:>3}..{:>3})",
        span.start,
        span.end,
    );
}