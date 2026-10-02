use kome_ast::declarations::Declaration;
use kome_ast::expressions::Expression;
use kome_ast::patterns::Pattern;
use kome_ast::types::{PrimitiveTypeKind, Type};

#[test]
fn parses_generic_declarations_and_specializations() {
    let module = kome_parser::parse(
        r#"
struct Pair<A, B> { first: A, second: B }
trait Getter<T> { fn get(self) -> T }
for Pair<A, B> { fn first(self) -> A { return self.first } }
fn identity<T>(value: T) -> T { return value }
fn main() {
    let pair = Pair<Number, String> { first: 1, second: "two" }
    identity<Number>(pair.first)
}
"#,
    )
    .unwrap();
    let Declaration::Struct(pair) = &module.declarations[0] else {
        panic!()
    };
    assert_eq!(
        pair.type_parameters
            .iter()
            .map(|value| value.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    let Declaration::For(implementation) = &module.declarations[2] else {
        panic!()
    };
    assert_eq!(
        implementation
            .type_parameters
            .iter()
            .map(|value| value.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    let Declaration::Function(main) = &module.declarations[4] else {
        panic!()
    };
    let body = main.body.as_ref().unwrap();
    let kome_ast::statements::Statement::Let(binding) = &body.statements[0] else {
        panic!()
    };
    let Expression::Struct(value) = binding.init.as_ref().unwrap() else {
        panic!()
    };
    assert_eq!(value.type_arguments.len(), 2);
}

#[test]
fn keeps_less_than_as_a_binary_operator() {
    let expression = kome_parser::parse_expression("left < right").unwrap();
    assert!(matches!(expression, Expression::Binary(_)));
}

#[test]
fn parses_function_types_in_parameters() {
    let module = kome_parser::parse(
        "fn apply(value: Number, transform: (item: Number) -> String) -> String { return transform(value) }",
    )
    .unwrap();
    let Declaration::Function(function) = &module.declarations[0] else {
        panic!()
    };
    let Pattern::Ident(transform) = &function.params[1] else {
        panic!()
    };
    let Type::Function(function_type) = transform.type_annotation.as_ref().unwrap() else {
        panic!()
    };
    assert_eq!(function_type.params.len(), 1);
    assert!(matches!(
        function_type.params[0].type_,
        Type::Primitive(ref value) if value.kind == PrimitiveTypeKind::Number
    ));
    assert!(matches!(
        function_type.return_type.as_ref(),
        Type::Primitive(value) if value.kind == PrimitiveTypeKind::String
    ));
}
