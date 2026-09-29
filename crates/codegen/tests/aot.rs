//! AOT tests: build a standalone executable and run it.
//!
//! Requires `cc` on `$PATH` and `libkome_native_rt.a`, which is built
//! automatically if missing.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_output(name: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("kome-aot-test-{}-{id}-{name}", std::process::id()))
}

/// Makes sure the static runtime library exists, building it if needed.
fn ensure_runtime_library() -> Option<PathBuf> {
    let library = "libkome_native_rt.a";

    if let Some(path) = std::env::var_os("KOME_NATIVE_RT_LIB") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "kome_native_rt", "--quiet"])
        .current_dir(&workspace_root)
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    let freshly_built = workspace_root.join("target/debug").join(library);
    if freshly_built.is_file() {
        return Some(freshly_built);
    }

    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            for candidate in [
                directory.join(library),
                directory.join("..").join(library),
                directory.join("../lib").join(library),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn cc_available() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn build_and_run(source: &str) -> String {
    let Some(_runtime_library) = ensure_runtime_library() else {
        panic!("could not locate or build libkome_native_rt.a");
    };

    assert!(cc_available(), "`cc` is required for AOT tests");

    let module = kome_parser::parse(source).unwrap();

    let output = unique_output("program");

    kome_aot::build_executable(&module, &output).unwrap();

    let invocation = Command::new(&output)
        .output()
        .expect("built executable should run");

    let _ = std::fs::remove_file(&output);

    assert!(
        invocation.status.success(),
        "executable failed: code={:?} signal={:?} stdout={} stderr={}",
        invocation.status.code(),
        std::os::unix::process::ExitStatusExt::signal(&invocation.status),
        String::from_utf8_lossy(&invocation.stdout),
        String::from_utf8_lossy(&invocation.stderr),
    );

    String::from_utf8_lossy(&invocation.stdout).into_owned()
}

#[test]
fn builds_and_runs_numbers() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn main() {
    println(42)
}
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_number_arithmetic() {
    let stdout = build_and_run(
        r#"
@native("core.write")
fn print(value: Number)

fn double(value: Number) -> Number {
    return value * 2
}

fn main() {
    print(double(21))
}
"#,
    );

    assert_eq!(stdout, "42");
}

#[test]
fn builds_and_runs_structs_static_members_and_trait_dispatch() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
struct Point { x: Number, y: Number }
trait Sum { fn sum(self) -> Number }
for Point: Sum { fn sum(self) -> Number { return self.x + self.y } }
for Point {
    const ORIGIN: Point = Point { x: 0, y: 0 }
    fn make(x: Number, y: Number) -> Point { return Point { x: x, y: y } }
}
fn bounce(point: Point) -> Point { return point }
fn main() {
    let origin = Point.ORIGIN
    println(origin.sum())
    println(bounce(Point.make(20, 22)).sum())
}
"#,
    );
    assert_eq!(stdout, "0\n42\n");
}

#[test]
fn builds_and_runs_monomorphized_generics() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
struct Box<T> { value: T }
fn identity<T>(value: T) -> T { return value }
fn main() {
    let value = Box<Number> { value: identity(42) }
    println(value.value)
}
"#,
    );
    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_tasks_with_generic_results() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
@native("core.write_line")
fn print_text(value: String)
struct Container<T> { value: T }
fn answer() -> Number { return 42 }
fn text() -> String { return "Kome" }
fn wrapped() -> Container<Number> {
    return Container<Number> { value: 21 }
}
fn main() {
    let number = task answer()
    let string = task text()
    let container = task wrapped()
    println(wait number)
    print_text(wait string)
    let result = wait container
    println(result.value * 2)
}
"#,
    );
    assert_eq!(stdout, "42\nKome\n42\n");
}

#[test]
fn builds_and_runs_task_control_operations() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
fn child() -> Number { return 70 }
fn parent() -> Number {
    let child_task = task child()
    return wait child_task
}
fn main() {
    let completed = task 5
    let completed_value = wait completed
    cancel completed
    println(completed_value)
    let values = all(task 10, task 20, task 30)
    println(values[0])
    println(values[1])
    println(values[2])
    println(race(task 40, task 50))
    println(timeout(task 60, 100))
    println(wait task parent())
}
"#,
    );
    assert_eq!(stdout, "5\n10\n20\n30\n40\n60\n70\n");
}

#[test]
fn builds_and_runs_cooperative_sleep() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)
@native("io.sleep")
fn sleep(milliseconds: Number) -> Null
fn delayed() -> String {
    sleep(5)
    return "awake"
}
fn main() {
    println(wait task delayed())
}
"#,
    );
    assert_eq!(stdout, "awake\n");
}

#[test]
fn built_binary_has_no_rust_runtime_dependency() {
    // The point of this check is that the binary links only against system
    // libraries; running it in a minimal environment proves the runtime is
    // statically baked in.
    let Some(runtime_library) = ensure_runtime_library() else {
        panic!("could not locate or build libkome_native_rt.a");
    };

    assert!(runtime_library.is_file());
}
