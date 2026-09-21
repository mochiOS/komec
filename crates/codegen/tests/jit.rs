//! JIT execution tests using a thread-local native registry for capture.

use kome_native_rt::number::Number;
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
fn rejects_string_types() {
    let module = kome_parser::parse(
        r#"
fn main(value: String) {
}
"#,
    )
    .unwrap();

    let error = kome_jit::execute(&module, "main").unwrap_err();
    assert_eq!(error.message(), "String is not supported yet");
}

#[test]
fn rejects_string_literals() {
    let module = kome_parser::parse(
        r#"
fn main() {
    let value = "not supported"
}
"#,
    )
    .unwrap();

    let error = kome_jit::execute(&module, "main").unwrap_err();
    assert_eq!(error.message(), "string literals are not supported yet");
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
