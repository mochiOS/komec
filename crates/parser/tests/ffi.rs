use kome_ast::declarations::{Declaration, ExternItem};
use kome_ast::types::{PointerMutability, PrimitiveTypeKind, Type};

#[test]
fn parses_external_c_functions_and_incomplete_structs() {
    let module = kome_parser::parse(
        r#"
extern "C" from "viewkit" {
    struct VkRuntime
    fn vk_runtime_create(id: u64) -> *mut VkRuntime
    fn vk_runtime_destroy(runtime: *mut VkRuntime) -> i32
    fn vk_copy(source: *const u8, length: usize) -> isize
}
"#,
    )
    .unwrap();

    let Declaration::Extern(external) = &module.declarations[0] else {
        panic!("expected an external declaration");
    };
    assert_eq!(external.abi, "C");
    assert_eq!(external.library.as_deref(), Some("viewkit"));
    assert_eq!(external.items.len(), 4);
    let ExternItem::Struct(runtime) = &external.items[0] else {
        panic!("expected an incomplete structure");
    };
    assert_eq!(runtime.name, "VkRuntime");
    assert!(runtime.fields.is_none());
    let ExternItem::Function(create) = &external.items[1] else {
        panic!("expected an external function");
    };
    let Type::Pointer(pointer) = create.return_type.as_ref().unwrap() else {
        panic!("expected a pointer return type");
    };
    assert_eq!(pointer.mutability, PointerMutability::Mut);
    let ExternItem::Function(copy) = &external.items[3] else {
        panic!("expected an external function");
    };
    let Type::Primitive(return_type) = copy.return_type.as_ref().unwrap() else {
        panic!("expected an isize return type");
    };
    assert_eq!(return_type.kind, PrimitiveTypeKind::Isize);
}

#[test]
fn parses_process_external_symbols_without_a_library() {
    let module = kome_parser::parse("extern \"C\" { fn host_version() -> u32 }").unwrap();
    let Declaration::Extern(external) = &module.declarations[0] else {
        panic!("expected an external declaration");
    };
    assert_eq!(external.library, None);
}

#[test]
fn rejects_external_functions_with_kome_bodies() {
    let error = kome_parser::parse("extern \"C\" { fn invalid() { return } }").unwrap_err();
    assert!(error.to_string().contains("without a body"));
}
