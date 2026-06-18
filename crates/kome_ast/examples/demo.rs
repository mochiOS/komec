//! Demo: constructing and printing a Kome component AST.
//!
//! Builds the equivalent of:
//!
//! ```kome
//! @application
//! component App() {
//!     fn greet(name: String) {
//!         return "Hello, " + name
//!     }
//! }
//! ```

use kome_ast::{
    AstNode, Span,
    declarations::{
        Attribute, ComponentDeclaration, ComponentMember, Declaration, FunctionDeclaration, Module,
    },
    expressions::{BinaryOp, Expression, LiteralKind},
    patterns::{IdentifierPattern, Pattern},
    statements::{BlockStatement, ReturnStatement, Statement},
    types::{PrimitiveType, PrimitiveTypeKind, Type},
};

const SOURCE: &str = r#"@application
component App() {
    fn greet(name: String) {
        return "Hello, " + name
    }
}"#;

fn main() {
    // Expression: name
    let name_expression = Expression::ident("name", Span::new(87, 91));

    // Expression: "Hello, "
    let hello_expression = Expression::literal(
        LiteralKind::String("Hello, ".into()),
        Span::new(75, 84),
    );

    // Expression: "Hello, " + name
    let greeting_expression = Expression::binary(
        hello_expression,
        BinaryOp::Add,
        name_expression,
        Span::new(75, 91),
    );

    // Statement: return "Hello, " + name
    let return_statement = Statement::Return(ReturnStatement {
        span: Span::new(68, 91),
        argument: Some(greeting_expression),
    });

    // Parameter: name: String
    let name_parameter = Pattern::Ident(IdentifierPattern {
        span: Span::new(44, 56),
        name: "name".into(),
        type_annotation: Some(Type::Primitive(PrimitiveType {
            span: Span::new(50, 56),
            kind: PrimitiveTypeKind::String,
        })),
        default: None,
    });

    // Function body
    let function_body = BlockStatement {
        span: Span::new(58, 97),
        statements: vec![return_statement],
    };

    // Function declared inside the component
    let greet_function = FunctionDeclaration {
        span: Span::new(35, 97),
        attributes: vec![],
        name: "greet".into(),
        params: vec![name_parameter],
        body: Some(function_body),
        return_type: None,
    };

    // Attribute: @application
    let application_attribute = Attribute {
        span: Span::new(0, 12),
        name: "application".into(),
        args: Vec::new(),
    };

    // Component: App
    let component = Declaration::Component(ComponentDeclaration {
        span: Span::new(0, 99),
        name: "App".into(),
        params: Vec::new(),
        attributes: vec![application_attribute],
        body: vec![ComponentMember::Function(greet_function)],
    });

    print_span("component declaration", &component);

    // Top-level module
    let module = Module::new(vec![component], Span::new(0, SOURCE.len()));

    println!("{module:#?}");
    print_span("module", &module);

    // Confirm that the important spans point to the expected source text.
    debug_assert_eq!(&SOURCE[0..12], "@application");
    debug_assert_eq!(&SOURCE[35..97], "fn greet(name: String) {\n        return \"Hello, \" + name\n    }");
    debug_assert_eq!(&SOURCE[75..84], "\"Hello, \"");
    debug_assert_eq!(&SOURCE[87..91], "name");
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