use kome_lsp::definition::definition_at;
use tower_lsp::lsp_types::{Position, Url};

#[test]
fn follows_standard_library_reexports_to_the_original_definition() {
    let source = "use std::net\nfn inspect(error: net::IoError) {}";
    let offset = source.find("IoError").unwrap() + 2;
    let uri = Url::parse("file:///tmp/main.kome").unwrap();

    let location = definition_at(
        &uri,
        source,
        Position::new(1, (offset - "use std::net\n".len()) as u32),
    )
    .unwrap();

    assert!(location.uri.path().ends_with("/io/mod.kome"));
}
