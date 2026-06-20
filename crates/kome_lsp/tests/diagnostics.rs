use kome_lsp::diagnostics::syntax_diagnostics;
use tower_lsp::lsp_types::DiagnosticSeverity;

#[test]
fn returns_no_diagnostics_for_valid_source() {
    let source = r#"
enum Color {
    red,
    green,
    blue,
}
"#;

    let diagnostics = syntax_diagnostics(source);

    assert!(diagnostics.is_empty());
}

#[test]
fn reports_parse_error() {
    let source = r#"
enum Color {
    red
    green,
}
"#;

    let diagnostics = syntax_diagnostics(source);

    assert_eq!(diagnostics.len(), 1);

    let diagnostic = &diagnostics[0];

    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR),);

    assert_eq!(diagnostic.source.as_deref(), Some("kome"),);

    assert!(diagnostic.message.contains("expected"),);
}

#[test]
fn reports_lex_error() {
    let source = "let value = $";

    let diagnostics = syntax_diagnostics(source);

    assert_eq!(diagnostics.len(), 1);

    assert!(diagnostics[0].message.contains("unexpected character"),);
}

#[test]
fn reports_missing_closing_brace() {
    let source = r#"
enum Color {
    red,
"#;

    let diagnostics = syntax_diagnostics(source);

    assert_eq!(diagnostics.len(), 1);

    let diagnostic = &diagnostics[0];

    assert!(diagnostic.message.contains("`}`"),);

    assert!(diagnostic.range.start <= diagnostic.range.end,);
}

#[test]
fn clears_diagnostics_after_source_is_fixed() {
    let invalid_source = r#"
enum Color {
    red
    green,
}
"#;

    let valid_source = r#"
enum Color {
    red,
    green,
}
"#;

    assert_eq!(syntax_diagnostics(invalid_source).len(), 1,);

    assert!(syntax_diagnostics(valid_source).is_empty(),);
}
