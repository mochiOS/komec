use kome_native_rt::number::Number;
use kome_native_rt::string::KomeString;
use kome_native_rt::{
    NativeRegistry, RuntimeError, Value, clear_thread_registry, set_thread_registry,
};
use std::sync::{Arc, Mutex};

fn run_and_capture(source: &str) -> Vec<Value> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&values);
    let mut registry = NativeRegistry::new();
    registry.register("test.capture", move |arguments: &[Value]| {
        let [value] = arguments else {
            return Err(RuntimeError::native("capture expects one argument"));
        };
        output.lock().unwrap().push(value.clone());
        Ok(Value::Null)
    });
    set_thread_registry(registry);
    let module = kome_parser::parse(source).unwrap();
    kome_jit::execute(&module, "main").unwrap();
    clear_thread_registry();
    Arc::try_unwrap(values).unwrap().into_inner().unwrap()
}

#[test]
fn all_preserves_number_order_for_two_and_three_tasks() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: Number)
fn main() {
    let two = all(task 10, task 20)
    report(two[0])
    report(two[1])
    let three = all(task 30, task 40, task 50)
    report(three[0])
    report(three[1])
    report(three[2])
}
"#,
    );
    assert_eq!(
        values,
        ["10", "20", "30", "40", "50"]
            .into_iter()
            .map(|value| Value::Number(Number::parse(value).unwrap()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn all_owns_strings_and_generic_struct_results() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: String)
struct Container<T> { value: T }
fn main() {
    let strings = all(task "a", task "b")
    report(strings[0])
    report(strings[1])
    let boxes = all(
        task Container<String> { value: "c" },
        task Container<String> { value: "d" },
    )
    report(boxes[0].value)
    report(boxes[1].value)
}
"#,
    );
    assert_eq!(
        values,
        ["a", "b", "c", "d"]
            .into_iter()
            .map(|value| Value::String(KomeString::new(value)))
            .collect::<Vec<_>>()
    );
    assert!(values.iter().all(|value| match value {
        Value::String(value) => value.is_unique(),
        _ => false,
    }));
}

#[test]
fn race_returns_the_first_success_for_supported_result_types() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report_number(value: Number)
@native("test.capture")
fn report_string(value: String)
struct Container<T> { value: T }
fn main() {
    report_number(race(task 1, task 2, task 3))
    report_string(race(task "first", task "second"))
    let winner = race(
        task Container<Number> { value: 42 },
        task Container<Number> { value: 99 },
    )
    report_number(winner.value)
}
"#,
    );
    assert_eq!(
        values,
        vec![
            Value::Number(Number::parse("1").unwrap()),
            Value::String(KomeString::new("first")),
            Value::Number(Number::parse("42").unwrap()),
        ]
    );
}

#[test]
fn timeout_returns_values_completed_before_the_deadline() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: String)
struct Container<T> { value: T }
fn main() {
    report(timeout(task "ready", 100))
    let result = timeout(task Container<String> { value: "nested" }, 100)
    report(result.value)
}
"#,
    );
    assert_eq!(
        values,
        vec![
            Value::String(KomeString::new("ready")),
            Value::String(KomeString::new("nested")),
        ]
    );
}
