use kome_semantics::resolver::ScopeBuilder;
use kome_semantics::typecheck::TypeChecker;

#[test]
fn resolves_and_types_external_c_declarations() {
    let module = kome_parser::parse(
        r#"
extern "C" from "viewkit" {
    struct VkRuntime
    fn vk_runtime_create(id: u64) -> *mut VkRuntime
    fn vk_runtime_destroy(runtime: *mut VkRuntime) -> i32
}

fn close(id: u64) -> i32 {
    let runtime = vk_runtime_create(id)
    return vk_runtime_destroy(runtime)
}
"#,
    )
    .unwrap();

    let resolution = ScopeBuilder::resolve(&module);
    assert!(resolution.errors.is_empty(), "{:?}", resolution.errors);
    let checked = TypeChecker::check(&module);
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}
