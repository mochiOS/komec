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
fn waits_for_number_and_string_tasks() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report_number(value: Number)
@native("test.capture")
fn report_string(value: String)
fn number() -> Number { return 999999999999999999999999999999 }
fn text() -> String { return "Kome" }
fn main() {
    let number_task = task number()
    let text_task = task text()
    report_number(wait number_task)
    report_string(wait text_task)
}
"#,
    );
    assert_eq!(
        values,
        vec![
            Value::Number(Number::parse("999999999999999999999999999999").unwrap()),
            Value::String(KomeString::new("Kome")),
        ]
    );
    let Value::Number(number) = &values[0] else {
        panic!("expected Number")
    };
    let Value::String(text) = &values[1] else {
        panic!("expected String")
    };
    assert!(number.is_unique());
    assert!(text.is_unique());
}

#[test]
fn waits_for_struct_and_generic_struct_tasks() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: Number)
struct Point { x: Number, y: Number }
struct Container<T> { value: T }
fn point() -> Point { return Point { x: 20, y: 22 } }
fn wrapped() -> Container<Number> {
    return Container<Number> { value: 42 }
}
fn main() {
    let first = wait task point()
    let second = wait task wrapped()
    report(first.x + first.y)
    report(second.value)
}
"#,
    );
    assert_eq!(
        values,
        vec![
            Value::Number(Number::parse("42").unwrap()),
            Value::Number(Number::parse("42").unwrap()),
        ]
    );
}

#[test]
fn supports_multiple_tasks_repeated_waits_and_unwaited_results() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: String)
fn text(value: String) -> String { return value }
fn main() {
    let first = task text("first")
    let unused = task text("unused")
    let second = task text("second")
    report(wait first)
    report(wait first)
    report(wait second)
}
"#,
    );
    assert_eq!(
        values,
        vec![
            Value::String(KomeString::new("first")),
            Value::String(KomeString::new("first")),
            Value::String(KomeString::new("second")),
        ]
    );
}

#[test]
fn passes_and_returns_task_handles() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: Number)
fn answer() -> Number { return 42 }
fn passthrough(value: Task<Number>) -> Task<Number> { return value }
fn make() -> Task<Number> { return task answer() }
fn main() {
    let first = passthrough(make())
    report(wait first)
}
"#,
    );
    assert_eq!(values, vec![Value::Number(Number::parse("42").unwrap())]);
}

#[test]
fn specializes_generic_functions_over_tasks() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: String)
fn resolve<T>(value: Task<T>) -> T { return wait value }
fn main() {
    let result = resolve(task "generic")
    report(result)
}
"#,
    );
    assert_eq!(values, vec![Value::String(KomeString::new("generic"))]);
}

#[test]
fn suspends_and_resumes_a_task_waiting_for_a_nested_task() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: Number)
fn child() -> Number { return 42 }
fn parent() -> Number {
    let pending = task child()
    let value = wait pending
    return value
}
fn main() {
    let pending = task parent()
    report(wait pending)
}
"#,
    );
    assert_eq!(values, vec![Value::Number(Number::parse("42").unwrap())]);
}

#[test]
fn preserves_boolean_and_contextual_integer_results() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: bool)
fn main() {
    let flag = task true
    let integer: Task<i32> = task 42
    let value: i32 = wait integer
    report(wait flag)
}
"#,
    );
    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn cancelling_a_completed_task_does_not_lose_the_result() {
    let values = run_and_capture(
        r#"
@native("test.capture")
fn report(value: String)
fn main() {
    let work = task "kept"
    let kept = wait work
    cancel work
    cancel work
    report(kept)
}
"#,
    );
    assert_eq!(values, vec![Value::String(KomeString::new("kept"))]);
}
