use kome_ast::Span;

/// Kome source code token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Fn,
    Component,
    Struct,
    Trait,
    Enum,
    Recipe,
    State,
    Let,
    Var,
    Const,
    Pub,
    Use,
    Extern,
    From,
    As,
    Mut,
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
    Self_,
    Super,
    Task,
    Wait,
    Cancel,

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

    /// `::`
    ColonColon,

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
    /// Converts identifier text into a keyword token or a regular identifier.
    pub fn from_identifier(identifier: String) -> Self {
        match identifier.as_str() {
            "fn" => Self::Fn,
            "component" => Self::Component,
            "struct" => Self::Struct,
            "trait" => Self::Trait,
            "enum" => Self::Enum,
            "recipe" => Self::Recipe,
            "state" => Self::State,
            "let" => Self::Let,
            "var" => Self::Var,
            "const" => Self::Const,
            "pub" => Self::Pub,
            "use" => Self::Use,
            "extern" => Self::Extern,
            "from" => Self::From,
            "as" => Self::As,
            "mut" => Self::Mut,
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
            "self" => Self::Self_,
            "super" => Self::Super,
            "task" => Self::Task,
            "wait" => Self::Wait,
            "cancel" => Self::Cancel,
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

    /// Returns whether this token marks the end of the source.
    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }
}
