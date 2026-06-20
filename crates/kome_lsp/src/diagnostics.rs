use kome_ast::Span;
use kome_parser::{FrontendError, LexError, LexErrorKind, ParseError, ParseErrorKind};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

use crate::position::span_to_range;

pub fn syntax_diagnostics(source: &str) -> Vec<Diagnostic> {
    match kome_parser::parse(source) {
        Ok(_) => Vec::new(),
        Err(error) => {
            vec![frontend_error_to_diagnostic(source, error)]
        }
    }
}

fn frontend_error_to_diagnostic(source: &str, error: FrontendError) -> Diagnostic {
    let (span, message) = match error {
        FrontendError::Lex(error) => {
            let message = lex_error_message(&error);

            (error.span, message)
        }

        FrontendError::Parse(error) => {
            let message = parse_error_message(&error);

            (error.span, message)
        }
    };

    Diagnostic {
        range: span_to_range(source, span),
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("kome".to_owned()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

fn lex_error_message(error: &LexError) -> String {
    match &error.kind {
        LexErrorKind::UnexpectedCharacter(character) => {
            format!("unexpected character {character:?}")
        }

        LexErrorKind::UnterminatedString => "unterminated string literal".to_owned(),

        LexErrorKind::UnterminatedInterpolation => "unterminated template interpolation".to_owned(),

        LexErrorKind::InvalidEscape(character) => {
            format!("invalid escape sequence \\{character}")
        }
    }
}

fn parse_error_message(error: &ParseError) -> String {
    match &error.kind {
        ParseErrorKind::Expected { expected, found } => {
            format!("expected {expected}, found {found:?}")
        }
    }
}

pub fn frontend_error_span(error: &FrontendError) -> Span {
    match error {
        FrontendError::Lex(error) => error.span,
        FrontendError::Parse(error) => error.span,
    }
}
