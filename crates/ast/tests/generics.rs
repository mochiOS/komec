use kome_ast::Span;
use kome_ast::generics::TypeSubstitution;
use kome_ast::types::{ListType, NamedType, OptionalType, PrimitiveType, PrimitiveTypeKind, Type};

fn named(name: &str, arguments: Vec<Type>) -> Type {
    Type::Named(NamedType {
        span: Span::new(0, 0),
        name: name.into(),
        type_arguments: arguments,
    })
}

#[test]
fn recursively_substitutes_nested_generic_types() {
    let number = Type::Primitive(PrimitiveType {
        span: Span::new(0, 0),
        kind: PrimitiveTypeKind::Number,
    });
    let string = Type::Primitive(PrimitiveType {
        span: Span::new(0, 0),
        kind: PrimitiveTypeKind::String,
    });
    let substitution = TypeSubstitution::new(["T", "U"], &[number.clone(), string.clone()]);
    let input = Type::Optional(OptionalType {
        span: Span::new(0, 0),
        inner: Box::new(named(
            "Pair",
            vec![
                named("T", vec![]),
                Type::List(ListType {
                    span: Span::new(0, 0),
                    element: Box::new(named("U", vec![])),
                }),
            ],
        )),
    });
    let Type::Optional(output) = substitution.apply(&input) else {
        panic!()
    };
    let Type::Named(pair) = output.inner.as_ref() else {
        panic!()
    };
    assert_eq!(pair.type_arguments[0], number);
    let Type::List(list) = &pair.type_arguments[1] else {
        panic!()
    };
    assert_eq!(list.element.as_ref(), &string);
}
