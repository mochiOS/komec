use kome_parser::parse;

#[test]
fn parses_from_source() {
    let source = include_str!(
        "test_source.kome"
    );

    let module = parse(source).unwrap();

    assert!(!module.declarations.is_empty());
}