use kome_ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Fn,
    Component,
    Recipe,
    State,
    Let,
    Const,
    Use,
    If,
    Else,
    While,
    For,
    In,
    Return,
    Break,
    Continue,
    Is,
    True,
    False,
    Null,

    Ident(String),

    String(String),
    Number(String),
    Percent(String),

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Colon,
    Pipe,
    At,
    Arrow,
    RArrow,
    Assign,
    PlusAssign,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
    Not,

    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}
