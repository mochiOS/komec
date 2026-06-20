use kome_parser::parse;
use kome_semantics::resolver::ScopeBuilder;

#[test]
fn resolves_empty_module() {
    let module = parse("").unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());
    assert_eq!(result.scopes.len(), 1);
    assert_eq!(result.scopes[0].kind, kome_semantics::scope::ScopeKind::Module);
    assert!(result.references.is_empty());
    assert!(result.symbols.is_empty());
}

#[test]
fn resolves_function_declaration() {
    let source = "fn greet(name) {}";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let scope = &result.scopes[result.root];
    let names: Vec<_> = scope.symbols.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(names, vec!["greet"]);

    let func_scope_id = scope.children[0];
    let func_scope = &result.scopes[func_scope_id];
    let param_names: Vec<_> = func_scope.symbols.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(param_names, vec!["name"]);
}

#[test]
fn resolves_reference_to_declared_name() {
    let source = "let x = 1\nfn foo() { x }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let x_ref = result.references.iter().find(|r| r.name == "x").unwrap();
    assert!(x_ref.resolved_to.is_some());
}

#[test]
fn reports_undefined_name() {
    let source = "fn foo() { bar }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert_eq!(result.errors.len(), 1);
    let err = &result.errors[0];
    match err {
        kome_semantics::error::ResolutionError::UndefinedName { name, .. } => {
            assert_eq!(name, "bar");
        }
        other => panic!("expected UndefinedName, got {other:?}"),
    }
}

#[test]
fn reports_duplicate_definition() {
    let source = "fn foo() {}\nfn foo() {}";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert_eq!(result.errors.len(), 1);
    let err = &result.errors[0];
    match err {
        kome_semantics::error::ResolutionError::DuplicateDefinition { name, .. } => {
            assert_eq!(name, "foo");
        }
        other => panic!("expected DuplicateDefinition, got {other:?}"),
    }
}

#[test]
fn allows_variable_shadowing() {
    let source = "let x = 1\nlet x = 2";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn allows_same_name_in_different_scopes() {
    let source = "fn foo() { let x = 1 }\nfn bar() { let x = 2 }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn inner_scope_masks_outer() {
    let source = "let x = 1\nfn foo() { let x = 2 }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn resolves_enum_cases() {
    let source = "enum Color { Red, Green, Blue }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let enum_scope_id = result.scopes[result.root].children[0];
    let enum_scope = &result.scopes[enum_scope_id];
    let case_names: Vec<_> = enum_scope.symbols.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(case_names, vec!["Red", "Green", "Blue"]);
}

#[test]
fn resolves_is_pattern_binding() {
    let source = "fn check() { is x => x }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let x_refs: Vec<_> = result.references.iter().filter(|r| r.name == "x").collect();
    assert!(!x_refs.is_empty());
    for r in &x_refs {
        assert!(r.resolved_to.is_some(), "reference to 'x' should resolve");
    }
}

#[test]
fn resolves_dot_ident_as_reference() {
    let source = "fn foo() { .bar }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert_eq!(result.errors.len(), 1);
    let err = &result.errors[0];
    match err {
        kome_semantics::error::ResolutionError::UndefinedName { name, .. } => {
            assert_eq!(name, "bar");
        }
        other => panic!("expected UndefinedName, got {other:?}"),
    }
}

#[test]
fn resolves_use_imports() {
    let source = "use viewKit\nfn foo() { viewKit.Button() }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());
}

#[test]
fn resolves_call_expression() {
    let source = "fn greet() {}\nfn foo() { greet() }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let greet_ref = result.references.iter().find(|r| r.name == "greet").unwrap();
    assert!(greet_ref.resolved_to.is_some());
}

#[test]
fn resolves_component_expression() {
    let source = "component Button(title: String) {}\nfn foo() { Button(title: \"click\") }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let btn_ref = result.references.iter().find(|r| r.name == "Button").unwrap();
    assert!(btn_ref.resolved_to.is_some());
}

#[test]
fn resolves_closure_params() {
    let source = "fn foo() { |x, y| x + y }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let x_ref = result.references.iter().find(|r| r.name == "x").unwrap();
    assert!(x_ref.resolved_to.is_some());
    let y_ref = result.references.iter().find(|r| r.name == "y").unwrap();
    assert!(y_ref.resolved_to.is_some());
}

#[test]
fn for_in_introduces_binding() {
    let source = "fn foo(items) { for item in items {} }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert!(result.errors.is_empty());

    let item_refs: Vec<_> = result.references.iter().filter(|r| r.name == "item").collect();
    assert!(item_refs.is_empty(), "item is a binding, not a reference");
    let items_ref = result.references.iter().find(|r| r.name == "items").unwrap();
    assert!(items_ref.resolved_to.is_some());
}

#[test]
fn reports_undefined_in_block() {
    let source = "fn foo() { { undefined_name } }";
    let module = parse(source).unwrap();
    let result = ScopeBuilder::resolve(&module);

    assert_eq!(result.errors.len(), 1);
    let err = &result.errors[0];
    match err {
        kome_semantics::error::ResolutionError::UndefinedName { name, .. } => {
            assert_eq!(name, "undefined_name");
        }
        other => panic!("expected UndefinedName, got {other:?}"),
    }
}
