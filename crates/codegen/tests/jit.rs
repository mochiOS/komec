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
fn executes_all_scalar_operators_and_percent_literals() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    var value = 20
    value += 1
    if !(value / 2 == 10.5) { report(0) }
    if "ko" + "me" == "kome" { report(50%) }
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("50").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn executes_fixed_width_numeric_operators() {
    run(r#"
fn integer_math(value: i32) -> i32 {
    var result: i32 = value * 3
    result += 4
    if result >= 10 { return result / 2 }
    return result - 1
}

fn float_math(value: f64) -> f64 {
    return (value + 1.0) / 2.0
}

fn main() {
    integer_math(2)
    float_math(3.0)
}
"#);
}

#[test]
fn executes_list_literals_dynamic_indexing_and_for_in() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let values = [10, 20, 30, 40]
    var total = 0
    var index = 1
    report(values[index])
    for value in values {
        if value == 20 { continue }
        if value == 40 { break }
        total += value
    }
    report(total)
    let empty: Number[] = []
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::Number(Number::parse("20").unwrap()),
            Value::Number(Number::parse("40").unwrap()),
        ]
    );
    clear_thread_registry();
}

#[test]
fn owns_runtime_managed_list_elements() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: String)

fn main() {
    let values = ["a", "b"]
    for value in values { report(value) }
    report(values[0])
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::String(KomeString::new("a")),
            Value::String(KomeString::new("b")),
            Value::String(KomeString::new("a")),
        ]
    );
    clear_thread_registry();
}

#[test]
fn executes_block_object_and_template_expressions() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: String)

struct User { name: String, score: Number }

fn main() {
    let user: User = {
        name: "Kome",
        score: {
            let base = 40
            base + 2
        },
    }
    let signed: i16 = 12
    let unsigned: u32 = 34
    let single: f32 = 1.5
    let double: f64 = 2.25
    report("Hello, {user.name}: {user.score} {true} {null} {signed} {unsigned} {single} {double}")
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::String(KomeString::new(
            "Hello, Kome: 42 true null 12 34 1.5 2.25"
        ))]
    );
    clear_thread_registry();
}

#[test]
fn orders_named_function_and_method_arguments() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

struct Calculator { base: Number }

fn combine(first: Number, second: Number) -> Number {
    return first * 10 + second
}

for Calculator {
    fn combine(self, first: Number, second: Number) -> Number {
        return self.base + first * 10 + second
    }
}

fn main() {
    let calculator = Calculator { base: 100 }
    report(combine(second: 2, first: 4))
    report(calculator.combine(second: 3, first: 5))
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::Number(Number::parse("42").unwrap()),
            Value::Number(Number::parse("153").unwrap()),
        ]
    );
    clear_thread_registry();
}

#[test]
fn executes_enum_cases_and_is_patterns() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

enum Color { red, green, blue }

fn main() {
    let color: Color = .green
    if color == Color.green { report(1) }
    is color .green => report(2)
    is "ready" "ready" => report(3)
    is 4 value => report(value)
}
"#);
    assert_eq!(
        capture.recorded(),
        ["1", "2", "3", "4"].map(|value| Value::Number(Number::parse(value).unwrap()))
    );
    clear_thread_registry();
}

#[test]
fn short_circuits_logical_operators() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

fn touched() -> bool {
    report(99)
    return true
}

fn main() {
    if false && touched() { report(1) }
    if true || touched() { report(2) }
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("2").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn supplies_default_function_and_method_arguments() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

struct Counter { base: Number }

fn add(value: Number, amount: Number = 2) -> Number {
    return value + amount
}

for Counter {
    fn add(self, amount: Number = 3) -> Number { return self.base + amount }
}

fn main() {
    let counter = Counter { base: 10 }
    report(add(40))
    report(add(amount: 4, value: 20))
    report(counter.add())
}
"#);
    assert_eq!(
        capture.recorded(),
        ["42", "24", "13"].map(|value| Value::Number(Number::parse(value).unwrap()))
    );
    clear_thread_registry();
}

#[test]
fn evaluates_top_level_and_nested_bindings() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

const BASE: Number = 40
let AMOUNT: Number = 2

fn main() {
    const LOCAL: Number = BASE + AMOUNT
    report(LOCAL)
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("42").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn executes_tasks_returning_void() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

fn work() { report(1) }

fn main() {
    let pending = task work()
    wait pending
    report(2)
}
"#);
    assert_eq!(
        capture.recorded(),
        ["1", "2"].map(|value| Value::Number(Number::parse(value).unwrap()))
    );
    clear_thread_registry();
}

#[test]
fn passes_fixed_width_numbers_through_native_abi() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report_integer(value: i32)
@native("test.capture")
fn report_float(value: f64)

fn main() {
    report_integer(42)
    report_float(10.5)
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::SignedInteger(42), Value::Float(10.5)]
    );
    clear_thread_registry();
}

#[test]
fn executes_immediately_invoked_capturing_closures() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

fn main() {
    let base = 40
    let result = (|value: Number| {
        let amount = value
        base + amount
    })(2)
    report(result)
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![Value::Number(Number::parse("42").unwrap())]
    );
    clear_thread_registry();
}

#[test]
fn stores_and_calls_local_and_global_closures() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)

const NEXT = |value: Number| value + 1

fn main() {
    let base = 40
    let add = |value: Number| base + value
    report(add(2))
    report(NEXT(41))
}
"#);
    assert_eq!(
        capture.recorded(),
        ["42", "42"].map(|value| Value::Number(Number::parse(value).unwrap()))
    );
    clear_thread_registry();
}

#[test]
fn generates_component_calls_arguments_defaults_and_children() {
    run(r#"
enum Color { primary, secondary }

component Text(content: String, color: Color = .primary)
component VStack()

fn main() {
    VStack {
        Text("Hello")
        Text(color: .secondary, content: "Kome")
    }
}
"#);
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
fn executes_if_else_and_early_returns() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
fn classify(value: Number) -> Number {
    if value < 10 {
        return 1
    } else if value == 10 {
        return 2
    } else {
        return 3
    }
}
fn main() {
    report(classify(5))
    report(classify(10))
    report(classify(20))
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::Number(Number::from_i64(1)),
            Value::Number(Number::from_i64(2)),
            Value::Number(Number::from_i64(3)),
        ]
    );
    clear_thread_registry();
}

#[test]
fn executes_while_break_continue_and_nested_loops() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
fn main() {
    var outer = 0
    var hits = 0
    while outer < 3 {
        outer = outer + 1
        var inner = 0
        while inner < 4 {
            inner = inner + 1
            if inner == 2 {
                continue
            }
            if inner == 4 {
                break
            }
            hits = hits + 1
        }
    }
    report(hits)
}
"#);
    assert_eq!(capture.recorded(), vec![Value::Number(Number::from_i64(6))]);
    clear_thread_registry();
}

#[test]
fn keeps_managed_values_owned_across_control_flow() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
fn choose(flag: bool) {
    let value = 999999999999999999999999999999
    if flag {
        report(value)
    } else {
        report(value)
    }
    while true {
        let temporary = 888888888888888888888888888888
        break
    }
}
fn main() {
    choose(true)
    choose(false)
}
"#);
    assert_eq!(capture.recorded().len(), 2);
    assert!(capture.number_is_unique(0));
    assert!(capture.number_is_unique(1));
    clear_thread_registry();
}

#[test]
fn lowers_generic_calls_and_tasks_inside_control_flow() {
    let capture = Capture::install("test.capture");
    run(r#"
@native("test.capture")
fn report(value: Number)
fn identity<T>(value: T) -> T { return value }
fn answer() -> Number { return 42 }
fn main() {
    var index = 0
    while index < 2 {
        if index == 0 {
            report(identity(10))
        } else {
            report(wait task answer())
        }
        index = index + 1
    }
}
"#);
    assert_eq!(
        capture.recorded(),
        vec![
            Value::Number(Number::from_i64(10)),
            Value::Number(Number::from_i64(42)),
        ]
    );
    clear_thread_registry();
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

#[test]
fn executes_optional_values_across_functions_and_struct_fields() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: bool)

struct Result { value: String? }

fn present() -> String? {
    return "Kome"
}

fn missing() -> String? {
    return null
}

fn main() {
    let first = Result { value: present() }
    let second = Result { value: missing() }
    report(first.value != null)
    report(second.value == null)
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![Value::Boolean(true), Value::Boolean(true)]
    );
    clear_thread_registry();
}

#[test]
fn generates_default_initialized_bindings_and_list_holes() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: String)

struct Defaults { name: String, values: String[] }

fn main() {
    let defaults: Defaults[] = [,]
    let values: String[] = [, "set", ,]
    report(defaults[0].name)
    report(values[0])
    report(values[1])
    report(values[2])
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![
            Value::String(KomeString::new("")),
            Value::String(KomeString::new("")),
            Value::String(KomeString::new("set")),
            Value::String(KomeString::new("")),
        ]
    );
    clear_thread_registry();
}

#[test]
fn assigns_struct_fields_and_list_elements() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: String)

struct User { name: String }

fn main() {
    var user = User { name: "before" }
    var names = ["first", "second"]
    user.name = "after"
    names[0] += " updated"
    names[1] = user.name
    report(user.name)
    report(names[0])
    report(names[1])
}
"#);

    assert_eq!(
        capture.recorded(),
        vec![
            Value::String(KomeString::new("after")),
            Value::String(KomeString::new("first updated")),
            Value::String(KomeString::new("after")),
        ]
    );
    clear_thread_registry();
}

#[test]
fn preserves_context_and_callees_through_groups() {
    let capture = Capture::install("test.capture");

    run(r#"
@native("test.capture")
fn report(value: Number)
struct Value { number: Number }
fn identity(value: Number) -> Number { return value }
for Value {
    fn get(self) -> Number { return self.number }
}
fn main() {
    let value: Value = ({ number: 41 })
    report((identity)(value.number))
    report((value.get)() + 1)
}
"#);

    assert_eq!(
        capture.recorded(),
        ["41", "42"].map(|value| Value::Number(Number::parse(value).unwrap()))
    );
    clear_thread_registry();
}
