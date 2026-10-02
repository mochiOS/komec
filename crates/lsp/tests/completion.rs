use kome_lsp::completion::completion_at;
use tower_lsp::lsp_types::Position;

#[test]
fn completes_public_standard_library_members() {
    let source = "use std::io\nfn main() { io::pr }";
    let offset = source.find("pr }").unwrap() + 2;
    let items = completion_at(
        source,
        Position::new(1, (offset - "use std::io\n".len()) as u32),
    );
    let labels = items.into_iter().map(|item| item.label).collect::<Vec<_>>();

    assert!(labels.contains(&"print".to_owned()));
    assert!(labels.contains(&"println".to_owned()));
    assert!(!labels.contains(&"__write".to_owned()));
}

#[test]
fn completes_imported_module_names() {
    let source = "use std::io\nfn main() { io }";
    let offset = source.rfind("io }").unwrap() + 2;
    let items = completion_at(
        source,
        Position::new(1, (offset - "use std::io\n".len()) as u32),
    );

    assert!(items.iter().any(|item| item.label == "io"));
}

#[test]
fn completes_public_reexports() {
    let source = "use std::net\nfn main() { net::Io }";
    let offset = source.find("Io }").unwrap() + 2;
    let items = completion_at(
        source,
        Position::new(1, (offset - "use std::net\n".len()) as u32),
    );

    assert!(items.iter().any(|item| item.label == "IoError"));
}
