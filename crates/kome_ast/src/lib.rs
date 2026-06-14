//! AST types for the Kome language.
//!
//! | Module | Contents |
//! |---|---|
//! | [`declarations`] | `component`, `function`, `let`, `const`, `use` |
//! | [`expressions`] | literals, operators, calls, closures |
//! | [`statements`] | blocks, control flow, `is` pattern-match |
//! | [`patterns`] | identifier, literal, dot-ident patterns |
//! | [`types`] | primitives, functions, objects, unions |

pub mod declarations;
pub mod expressions;
pub mod patterns;
pub mod statements;
pub mod types;

/// Start and end byte offset in source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

}

/// Every AST node has a source [`Span`].
pub trait AstNode {
    fn span(&self) -> Span;
}
