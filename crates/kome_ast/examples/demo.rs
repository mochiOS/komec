//! Demo: constructing and printing a simple Kome AST.
//!
//! Builds the equivalent of:
//!
//! ```kome
//! fn greet(name: String) {
//!     return "Hello, " + name;
//! }
//! ```

use kome_ast::{
    AstNode, Span,
    declarations::{Declaration, FunctionDeclaration, Module},
    expressions::{BinaryOp, Expression, LiteralKind},
    patterns::{IdentifierPattern, Pattern},
    statements::{BlockStatement, ReturnStatement, Statement},
    types::{PrimitiveType, PrimitiveTypeKind, Type},
};

fn main() {
    // --- leaf nodes ---
    let name_ident = Expression::ident("name", Span::new(28, 32));
    let hello = Expression::literal(LiteralKind::String("Hello, ".into()), Span::new(40, 49));

    // --- expression: "Hello, " + name (Span::new(40, 53)) ---
    let concat = Expression::binary(hello, BinaryOp::Add, name_ident, Span::new(40, 53));

    // --- statement: return "Hello, " + name; ---
    let ret = Statement::Return(ReturnStatement {
        span: Span::new(33, 54),
        argument: Some(concat),
    });

    // --- block ---
    let block = Statement::Block(BlockStatement {
        span: Span::new(31, 56),
        statements: vec![ret],
    });

    // --- parameter: name: String ---
    let param = Pattern::Ident(IdentifierPattern {
        span: Span::new(13, 27),
        name: "name".into(),
        type_annotation: Some(Type::Primitive(PrimitiveType {
            span: Span::new(20, 26),
            kind: PrimitiveTypeKind::String,
        })),
        default: None,
    });

    // --- function: fn greet(name: String) { ... } ---
    let func = Declaration::Function(FunctionDeclaration {
        span: Span::new(0, 56),
        name: "greet".into(),
        params: vec![param],
        body: match block {
            Statement::Block(b) => Some(b),
            _ => unreachable!(),
        },
        return_type: None,
    });

    // --- top-level module ---
    let module = Module::new(vec![func], Span::new(0, 56));

    // --- display ---
    println!("{:#?}", module);

    // --- verify AstNode trait works via generics ---
    print_span("module", &module);
}

/// Generic function: works with any `AstNode`.
fn print_span(label: &str, node: &impl AstNode) {
    let s = node.span();
    println!("{}: [{:>3}..{:>3})", label, s.start, s.end);
}
