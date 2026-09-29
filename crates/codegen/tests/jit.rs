//! JIT execution tests using a thread-local native registry for capture.

use kome_native_rt::number::Number;
use kome_native_rt::string::KomeString;
use kome_native_rt::{
    NativeRegistry, RuntimeError, Value, clear_thread_registry, set_thread_registry,
};
use std::sync::{Arc, Mutex};

/// A single-argument capture sink installed as a native function.
struct Capture {
    values: Arc<Mutex<Vec<Value>>>,
}

impl Capture {
    /// Installs `symbol` on the current thread's registry.
    fn install(symbol: &'static str) -> Self {
        let values = Arc::new(Mutex::new(Vec::new()));

        let sink = Arc::clone(&values);

        let mut registry = NativeRegistry::new();
        registry.register(symbol, move |arguments: &[Value]| {
            let [value] = arguments else {
                return Err(RuntimeError::native("capture expects exactly one argument"));
            };

            sink.lock().unwrap().push(value.clone());

            Ok(Value::Null)
        });

        set_thread_registry(registry);

        Self { values }
    }

    fn recorded(&self) -> Vec<Value> {
        self.values.lock().unwrap().clone()
    }

    fn number_is_unique(&self, index: usize) -> bool {
        let values = self.values.lock().unwrap();

        let Value::Number(number) = &values[index] else {
            panic!("captured value is not a Number");
        };

        number.is_unique()
    }

    fn string_is_unique(&self, index: usize) -> bool {
        let values = self.values.lock().unwrap();
        let Value::String(string) = &values[index] else {
            panic!("captured value is not a String")
        };
        string.is_unique()
    }
}

fn run(source: &str) {
    let module = kome_parser::parse(source).unwrap();

    kome_jit::execute(&module, "main").unwrap();
}

#[test]
fn executes_kome_wrapper_around_native_function() {
    let capture = Capture::install("core.write_line");

    run(r#"
@native("core.write_line")
fn write_line_native(value: bool)

fn print(value: bool) {
    write_line_native(value)
}

fn main() {
    print(true)
}
"#);

    assert_eq!(capture.recorded(), vec![Value::Boolean(true)]);

    clear_thread_registry();
}

#[test]
fn passes_numbers_through_arithmetic_and_variables() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn add(a: Number, b: Number) -> Number {
    return a + b
}

fn main() {
    let total = add(20, 22)
    let doubled = total * 2
    report(total + doubled - 2)
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("124").unwrap())]
    );

    clear_thread_registry();
}

#[test]
fn recognizes_string_types() {
    let module = kome_parser::parse(
        r#"
fn main(value: String) {
}
"#,
    )
    .unwrap();

    let info = kome_codegen::compile::analyze_module(&module).unwrap();
    assert_eq!(
        info.get("main").unwrap().signature().params,
        vec![kome_codegen::KomeType::String]
    );
}

#[test]
fn compiles_string_literals() {
    let module = kome_parser::parse(
        r#"
fn main() {
    let value = "not supported"
}
"#,
    )
    .unwrap();

    kome_jit::execute(&module, "main").unwrap();
}

#[test]
fn recognizes_named_runtime_backed_types() {
    let module = kome_parser::parse(
        r#"
@runtime("string")
struct RuntimeText

fn identity(value: RuntimeText) -> RuntimeText {
    return value
}
"#,
    )
    .unwrap();
    let info = kome_codegen::compile::analyze_module(&module).unwrap();

    assert_eq!(
        info.runtime_type("RuntimeText"),
        Some(kome_codegen::KomeType::String)
    );
    assert_eq!(
        info.get("identity").unwrap().signature().params,
        vec![kome_codegen::KomeType::String]
    );
}

#[test]
fn executes_struct_construction_fields_and_methods() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

struct Point { x: Number, y: Number }

for Point {
    fn sum(self) -> Number { return self.x + self.y }
    fn moved(self, dx: Number, dy: Number) -> Point {
        return Point { x: self.x + dx, y: self.y + dy }
    }
}

fn main() {
    let point = Point { x: 10, y: 20 }
    var moved = point
    moved = point.moved(5, 10)
    report(moved.sum())
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("45").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn executes_static_methods_constants_nested_structs_and_strings() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: String)

struct User { name: String, score: Number }
struct BoxedUser { user: User }

for User {
    const UNKNOWN: User = User { name: "unknown", score: 0 }
    fn make(name: String, score: Number) -> User { return User { name: name, score: score } }
    fn renamed(self, name: String) -> User { return User { name: name, score: self.score } }
}

fn identity(value: BoxedUser) -> BoxedUser { return value }

fn main() {
    let unknown = User.UNKNOWN
    report(unknown.name)
    let original = User.make("alice", 42)
    let nested = identity(BoxedUser { user: original })
    let copy = nested
    report(nested.user.name)
    report(copy.user.renamed("bob").name)
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::String(KomeString::new("unknown")),
            Value::String(KomeString::new("alice")),
            Value::String(KomeString::new("bob"))
        ]
    );
    clear_thread_registry();
}

#[test]
fn executes_statically_dispatched_trait_methods_for_multiple_types() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
trait Value { fn value(self) -> Number }
struct A { n: Number }
struct B { n: Number }
for A: Value { fn value(self) -> Number { return self.n } }
for B: Value { fn value(self) -> Number { return self.n + 1 } }
fn main() {
    report(A { n: 10 }.value())
    report(B { n: 20 }.value())
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::Number(Number::parse("10").unwrap()),
            Value::Number(Number::parse("21").unwrap())
        ]
    );
    clear_thread_registry();
}

#[test]
fn executes_generic_structs_and_functions() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
struct Container<T> { value: T }
struct Pair<A, B> { first: A, second: B }
fn identity<T>(value: T) -> T { return value }
fn pair<A, B>(first: A, second: B) -> Pair<A, B> { return Pair<A, B> { first: first, second: second } }
fn main() {
    let number = Container<Number> { value: 42 }
    let x = identity<Number>(number.value)
    let y = identity(8)
    let paired = pair(7, "Kome")
    report(x + y + paired.first)
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("57").unwrap())]
    );
    assert!(capture.number_is_unique(0));
    clear_thread_registry();
}

#[test]
fn executes_generic_implementation_and_nested_ownership() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: String)
struct Container<T> { value: T }
for Container<T> {
    fn get(self) -> T { return self.value }
}
fn pass<T>(value: Container<T>) -> Container<T> { return value }
fn identity<T>(value: T) -> T { return value }
fn main() {
    let inner = Container<String> { value: "Kome" }
    let outer = Container<Container<String>> { value: inner }
    report(identity<String>(outer.get().get()))
    report(identity("inferred"))
    report(pass(Container<String> { value: "generic" }).get())
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::String(KomeString::new("Kome")),
            Value::String(KomeString::new("inferred")),
            Value::String(KomeString::new("generic"))
        ]
    );
    assert!(capture.string_is_unique(0));
    assert!(capture.string_is_unique(1));
    assert!(capture.string_is_unique(2));
    clear_thread_registry();
}

#[test]
fn executes_generic_trait_static_dispatch() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
trait Getter<T> { fn get(self) -> T }
struct Container<T> { value: T }
struct Pair<A, B> { first: A, second: B }
for Container<T>: Getter<T> { fn get(self) -> T { return self.value } }
for Container<T> {
    fn make(value: T) -> Container<T> { return Container<T> { value: value } }
    fn duplicated(self) -> Pair<T, T> { return Pair<T, T> { first: self.value, second: self.value } }
}
fn main() { report(Container<Number>.make(42).duplicated().first) }
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("42").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn executes_generic_trait_with_concrete_trait_argument() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: String)
trait Converter<T> { fn convert(self) -> T }
struct Foo { value: String }
for Foo: Converter<String> { fn convert(self) -> String { return self.value } }
fn main() { report(Foo { value: "converted" }.convert()) }
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::String(KomeString::new("converted"))]
    );
    clear_thread_registry();
}

#[test]
fn marshals_booleans_and_nulls_into_natives() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report_flag(value: bool)

@native("test.capture")
fn report_nothing(value: Null)

fn main() {
    report_flag(1 < 2)
    report_flag(false == true)
    report_nothing(null)
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Boolean(true), Value::Boolean(false), Value::Null,]
    );

    clear_thread_registry();
}

#[test]
fn resolves_forward_references_between_functions() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    report(outer())
}

fn outer() -> Number {
    return inner() + 1
}

fn inner() -> Number {
    return 41
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("42").unwrap())]
    );

    clear_thread_registry();
}

#[test]
fn returns_error_when_entry_function_does_not_exist() {
    let module = kome_parser::parse(
        r#"
fn other() {
}
"#,
    )
    .unwrap();

    let error = kome_jit::execute(&module, "main").unwrap_err();

    assert_eq!(error.message(), "entry function `main` was not found");
}

#[test]
fn rejects_unsupported_statements_at_compile_time() {
    let module = kome_parser::parse(
        r#"
fn main() {
    while true {
    }
}
"#,
    )
    .unwrap();

    let error = kome_jit::execute(&module, "main").unwrap_err();

    assert!(
        error.message().contains("`while` is not supported"),
        "unexpected error: {error}"
    );
}

#[test]
fn compiles_fixed_width_numeric_bindings() {
    run(r#"
fn main() {
    let a: i8 = 10
    let b: i16 = 20
    let c: i32 = 30
    let d: i64 = 40
    let e: u8 = 50
    let f: u16 = 60
    let g: u32 = 70
    let h: u64 = 80
    let i: f32 = 1.5
    let j: f64 = 2.5
}
"#);
}

#[test]
fn releases_copied_number_bindings() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let a = 999999999999999999999999999999
    let b = a
    report(b)
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn handles_number_self_assignment() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    var value = 999999999999999999999999999999
    value = value
    report(value)
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn transfers_returned_number_ownership() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn make() -> Number {
    let value = 999999999999999999999999999999
    return value
}

fn main() {
    let value = make()
    report(value)
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn releases_non_returned_numbers_on_return() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn make() -> Number {
    let returned = 999999999999999999999999999999
    let unused = 888888888888888888888888888888
    return returned
}

fn main() {
    let value = make()
    report(value)
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn moves_number_on_final_read() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let a = 999999999999999999999999999999
    let b = a
    report(b)
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn returns_parameter_number_safely() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn identity(value: Number) -> Number {
    return value
}

fn main() {
    let value = 999999999999999999999999999999
    report(identity(value))
}
"#);

    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn handles_shadowed_number_bindings() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let value = 999999999999999999999999999999

    {
        let value = 888888888888888888888888888888
        report(value)
    }

    report(value)
}
"#);

    assert_eq!(capture.recorded().len(), 2);

    clear_thread_registry();
}

#[test]
fn handles_owned_number_temporary_in_arithmetic() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let a = 100000000000000000000000000000
    let b = 200000000000000000000000000000
    let c = 3
    let result = (a + b) * c
    report(result)
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Number(
            Number::parse("900000000000000000000000000000").unwrap()
        )]
    );
    assert!(capture.number_is_unique(0));

    clear_thread_registry();
}

#[test]
fn handles_owned_number_temporary_in_comparison() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: bool)

fn main() {
    let a = 100000000000000000000000000000
    let b = 200000000000000000000000000000
    let c = 400000000000000000000000000000
    report((a + b) < c)
}
"#);

    assert_eq!(capture.recorded(), vec![Value::Boolean(true)]);

    clear_thread_registry();
}
