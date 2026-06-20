use kome_ast::Span;

#[derive(Debug, Clone)]
pub enum ResolutionError {
    UndefinedName {
        name: String,
        span: Span,
    },
    DuplicateDefinition {
        name: String,
        first: Span,
        second: Span,
    },
    ScopeStackEmpty,
    InvalidLetLocation {
        span: Span,
    },
}
