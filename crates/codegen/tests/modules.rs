use kome_parser::parse;
use kome_semantics::modules::{SourceModule, link_modules};

#[test]
fn preserves_external_c_symbols_when_modules_are_qualified() {
    let library = SourceModule::new(
        "library",
        Vec::new(),
        parse(
            r#"
extern "C" from "native" {
    fn native_call() -> i32
}
pub fn call() -> i32 { return native_call() }
"#,
        )
        .unwrap(),
        false,
    );
    let application = SourceModule::new(
        "application",
        Vec::new(),
        parse("use library::call\nfn main() { call() }").unwrap(),
        true,
    );
    let module = link_modules(vec![library, application]).unwrap();

    let information = kome_codegen::compile::analyze_module(&module).unwrap();
    let external = information
        .external_functions()
        .find(|(name, ..)| *name == "library::native_call")
        .unwrap();

    assert_eq!(external.1, "native_call");
    assert_eq!(external.2, Some("native"));
}
