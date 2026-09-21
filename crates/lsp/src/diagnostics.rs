use crate::position::span_to_range;
use kome_ast::Span;
use kome_ast::declarations::{Declaration, Module, UseImport};
use kome_parser::{FrontendError, LexError, LexErrorKind, ParseError, ParseErrorKind};
use kome_semantics::error::ResolutionError;
use kome_semantics::resolver::ScopeBuilder;
use komec::stdlib::StandardLibrary;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

/// Runs both syntax and semantic diagnostics on a Kome source file.
///
/// First parses the source via [`kome_parser::parse`]; on success, also runs name
/// resolution via [`ScopeBuilder::resolve`] and reports any [`ResolutionError`]s.
pub fn syntax_diagnostics(source: &str) -> Vec<Diagnostic> {
    match kome_parser::parse(source) {
        Ok(module) => {
            let std_import_span = standard_library_import_span(&module);

            let module = match StandardLibrary::discover() {
                Ok(standard_library) => match standard_library.merge_with_imports(module) {
                    Ok(module) => module,

                    Err(error) => {
                        return vec![standard_library_error_to_diagnostic(
                            source,
                            std_import_span.unwrap_or(Span::new(0, 0)),
                            error,
                        )];
                    }
                },

                Err(error) if std_import_span.is_some() => {
                    return vec![standard_library_error_to_diagnostic(
                        source,
                        std_import_span.unwrap(),
                        error,
                    )];
                }

                Err(_) => module,
            };

            let resolution = ScopeBuilder::resolve(&module);

            resolution
                .errors
                .iter()
                .map(|error| resolution_error_to_diagnostic(source, error))
                .collect()
        }

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

fn resolution_error_to_diagnostic(source: &str, error: &ResolutionError) -> Diagnostic {
    let (span, message) = match error {
        ResolutionError::UndefinedName { name, span } => (span, format!("undefined name `{name}`")),
        ResolutionError::DuplicateDefinition {
            name,
            first: _,
            second,
        } => (second, format!("duplicate definition of `{name}`")),
        ResolutionError::ScopeStackEmpty => {
            return Diagnostic {
                range: span_to_range(source, Span::new(0, 0)),
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("kome".to_owned()),
                message: "internal error: scope stack is empty".to_owned(),
                related_information: None,
                tags: None,
                data: None,
            };
        }
        ResolutionError::InvalidLetLocation { span } => (
            span,
            "`let` is not allowed at module or component level; use `const` or `state` instead"
                .to_owned(),
        ),
        ResolutionError::AssignmentToImmutable { name, span } => (
            span,
            format!("cannot assign to immutable variable `{name}`"),
        ),
    };

    Diagnostic {
        range: span_to_range(source, *span),
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

/// Extracts the [`Span`] from any [`FrontendError`].
pub fn frontend_error_span(error: &FrontendError) -> Span {
    match error {
        FrontendError::Lex(error) => error.span,
        FrontendError::Parse(error) => error.span,
    }
}

fn standard_library_import_span(module: &Module) -> Option<Span> {
    for declaration in &module.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };

        for import in &use_declaration.imports {
            let UseImport::Module(path) = import else {
                continue;
            };

            let is_installed = path.segments.first().is_some_and(|segment| {
                matches!(
                    &segment.kind,
                    kome_ast::declarations::PathSegmentKind::Ident(name)
                        if komec::stdlib::is_known_package(name)
                )
            });

            if is_installed {
                return Some(path.span);
            }
        }
    }

    None
}

fn standard_library_error_to_diagnostic(source: &str, span: Span, error: String) -> Diagnostic {
    Diagnostic {
        range: span_to_range(source, span),

        severity: Some(DiagnosticSeverity::ERROR),

        code: None,
        code_description: None,

        source: Some("kome".to_owned()),

        message: format!("failed to resolve standard library: {error}",),

        related_information: None,
        tags: None,
        data: None,
    }
}
