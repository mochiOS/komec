use kome_ast::Span;

/// Kome source code token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Fn,
    Component,
    Enum,
    Recipe,
    State,
    Let,
    Mut,
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

    /// A string containing one or more interpolations.
    Template(Vec<TemplateTokenPart>),

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

    /// `?`
    Question,

    Pipe,
    At,

    /// `->`
    ThinArrow,

    /// `=>`
    FatArrow,

    /// `=`
    Assign,

    /// `+=`
    PlusAssign,

    Plus,
    Minus,
    Star,
    Slash,

    /// `==`
    Eq,

    /// `!=`
    NotEq,

    Lt,
    Lte,
    Gt,
    Gte,

    /// `&&`
    And,

    /// `||`
    Or,

    /// `!`
    Not,

    Eof,
}

/// One lexed part of a template string.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateTokenPart {
    /// Plain decoded string content.
    String { value: String, span: Span },

    /// Tokens contained inside `{ ... }`.
    Expression {
        tokens: Vec<Token>,

        /// Span including the opening and closing braces.
        span: Span,
    },
}

impl TokenKind {
    pub fn from_identifier(identifier: String) -> Self {
        match identifier.as_str() {
            "fn" => Self::Fn,
            "component" => Self::Component,
            "enum" => Self::Enum,
            "recipe" => Self::Recipe,
            "state" => Self::State,
            "let" => Self::Let,
            "mut" => Self::Mut,
            "const" => Self::Const,
            "use" => Self::Use,
            "if" => Self::If,
            "else" => Self::Else,
            "while" => Self::While,
            "for" => Self::For,
            "in" => Self::In,
            "return" => Self::Return,
            "break" => Self::Break,
            "continue" => Self::Continue,
            "is" => Self::Is,
            "true" => Self::True,
            "false" => Self::False,
            "null" => Self::Null,
            _ => Self::Ident(identifier),
        }
    }
}

/// A token and its byte range in the original source code.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub const fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub const fn eof(offset: usize) -> Self {
        Self {
            kind: TokenKind::Eof,
            span: Span::new(offset, offset),
        }
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }
}
