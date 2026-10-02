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
    let output = build_program(source);
    let invocation = Command::new(&output)
        .output()
        .expect("built executable should run");

    let _ = std::fs::remove_file(&output);

    assert_success(&invocation);
    String::from_utf8_lossy(&invocation.stdout).into_owned()
}

fn build_program(source: &str) -> PathBuf {
    let Some(_runtime_library) = ensure_runtime_library() else {
        panic!("could not locate or build libkome_native_rt.a");
    };

    assert!(cc_available(), "`cc` is required for AOT tests");

    let module = kome_parser::parse(source).unwrap();

    let output = unique_output("program");

    kome_aot::build_executable(&module, &output).unwrap();

    output
}

fn assert_success(invocation: &std::process::Output) {
    assert!(
        invocation.status.success(),
        "executable failed: code={:?} signal={:?} stdout={} stderr={}",
        invocation.status.code(),
        std::os::unix::process::ExitStatusExt::signal(&invocation.status),
        String::from_utf8_lossy(&invocation.stdout),
        String::from_utf8_lossy(&invocation.stderr),
    );
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
fn links_and_calls_a_shared_c_library() {
    let stdout = build_and_run(
        r#"
extern "C" from "libc.so.6" {
    fn abs(value: i32) -> i32
}

@native("core.write_line")
fn println(value: i32)

fn main() {
    println(abs(42))
}
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn links_c_functions_using_incomplete_struct_pointers() {
    let stdout = build_and_run(
        r#"
extern "C" from "libc.so.6" {
    struct FILE
    fn tmpfile() -> *mut FILE
    fn fclose(stream: *mut FILE) -> i32
}

@native("core.write_line")
fn println(value: i32)

fn main() {
    let stream = tmpfile()
    println(fclose(stream))
}
"#,
    );

    assert_eq!(stdout, "0\n");
}

#[test]
fn passes_managed_string_bytes_to_linked_c_functions() {
    let stdout = build_and_run(
        r#"
extern "C" {
    fn __kome_string_data(value: String) -> *const u8
    fn __kome_string_len(value: String) -> usize
}

extern "C" from "libc.so.6" {
    fn write(file: i32, bytes: *const u8, length: usize) -> isize
}

fn main() {
    let text = "ViewKit"
    write(1, __kome_string_data(text), __kome_string_len(text))
}
"#,
    );

    assert_eq!(stdout, "ViewKit");
}

#[test]
fn runs_a_struct_destructor_once_for_the_final_reference() {
    let stdout = build_and_run(
        r#"
trait Drop {
    fn drop(self)
}

@native("core.write_line")
fn printLine(value: String)

struct Resource {
    value: i32,
}

for Resource: Drop {
    fn drop(self) {
        printLine("dropped")
    }
}

fn main() {
    let resource = Resource { value: 42 }
    let copy = resource
    resource.value
    copy.value
}
"#,
    );

    assert_eq!(stdout, "dropped\n");
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
fn builds_and_runs_division_string_addition_and_compound_assignment() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn print_number(value: Number)
@native("core.write_line")
fn print_string(value: String)

fn main() {
    var value = 20
    value += 1
    print_number(value / 2)
    print_string("ko" + "me")
}
"#,
    );

    assert_eq!(stdout, "10.5\nkome\n");
}

#[test]
fn builds_and_runs_lists_and_for_in() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn main() {
    let values = [10, 20, 30]
    var total = 0
    for value in values { total += value }
    var index = 2
    println(total)
    println(values[index])
}
"#,
    );

    assert_eq!(stdout, "60\n30\n");
}

#[test]
fn builds_and_runs_block_object_and_template_expressions() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)

struct User { name: String, score: Number }

fn main() {
    let user: User = { name: "Kome", score: { 20 + 22 } }
    var object = { name: "AOT", "status": "ok", 1: "one" }
    object["status"] += "!"
    println("{user.name}={user.score}")
    println(object.name + object["status"] + object[1])
}
"#,
    );

    assert_eq!(stdout, "Kome=42\nAOTok!one\n");
}

#[test]
fn builds_and_runs_named_arguments() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn combine(first: Number, second: Number) -> Number {
    return first * 10 + second
}

fn main() { println(combine(second: 2, first: 4)) }
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_enum_cases_and_is_patterns() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
@native("core.write_line")
fn printText(value: String)

enum Status { idle, ready = "ready-value", done }

fn main() {
    let status: Status = .ready
    is status .ready => println(42)
    printText("{status}")
}
"#,
    );

    assert_eq!(stdout, "42\nready-value\n");
}

#[test]
fn builds_and_runs_default_arguments() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn add(value: Number, amount: Number = 2) -> Number {
    return value + amount
}

fn main() { println(add(40)) }
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_top_level_constants() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

const BASE: Number = 40
const AMOUNT: Number = 2

fn main() { println(BASE + AMOUNT) }
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_fixed_width_native_arguments() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn print_integer(value: i32)
@native("core.write_line")
fn print_float(value: f64)

fn main() {
    print_integer(42)
    print_float(10.5)
}
"#,
    );

    assert_eq!(stdout, "42\n10.5\n");
}

#[test]
fn builds_and_runs_immediate_closures() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn main() {
    let base = 40
    println((|value: Number| base + value)(2))
}
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_stored_closures() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)

fn main() {
    let base = 40
    let add = |value: Number| base + value
    println(add(2))
}
"#,
    );

    assert_eq!(stdout, "42\n");
}

#[test]
fn builds_and_runs_component_expressions() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)

component Text(content: String)
component VStack()
enum Color { primary }

fn main() {
    VStack { Text("Hello") }.padding(24).foreground(.primary)
    println("built")
}
"#,
    );

    assert_eq!(stdout, "built\n");
}

#[test]
fn builds_and_runs_conditionals_and_loops() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: Number)
fn classify(value: Number) -> Number {
    if value < 10 {
        return 1
    } else if value == 10 {
        return 2
    } else {
        return 3
    }
}
fn count() -> Number {
    var value = 0
    var total = 0
    while value < 6 {
        value = value + 1
        if value == 2 { continue }
        if value == 5 { break }
        total = total + value
    }
    return total
}
fn main() {
    println(classify(10))
    println(count())
}
"#,
    );
    assert_eq!(stdout, "2\n8\n");
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
fn builds_and_runs_field_and_index_assignments() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)
struct User { name: String }
fn main() {
    var user = User { name: "before" }
    var names = ["first", "second"]
    user.name = "after"
    names[0] += " updated"
    names[1] = user.name
    println(user.name)
    println(names[0])
    println(names[1])
}

"#,
    );
    assert_eq!(stdout, "after\nfirst updated\nafter\n");
}

#[test]
fn builds_and_runs_persistent_mutable_globals() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)
var LABEL = "Kome"
fn update() { LABEL += "!" }
fn main() {
    update()
    update()
    println(LABEL)
}
"#,
    );
    assert_eq!(stdout, "Kome!!\n");
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
fn builds_and_runs_runtime_socket_io() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let source = format!(
        r#"
@runtime("socket")
struct Socket
@native("io.socket_connect")
fn socketConnect(host: String, port: Number) -> Socket
@native("io.socket_read")
fn socketRead(socket: Socket, maximum: Number) -> String
@native("io.socket_write")
fn socketWrite(socket: Socket, value: String) -> Number
@native("io.socket_close")
fn socketClose(socket: Socket) -> Null
@native("core.write_line")
fn println(value: String)
for Socket {{
    fn connect(host: String, port: Number) -> Socket {{
        return socketConnect(host, port)
    }}
    fn read(self, maximum: Number) -> String {{
        return socketRead(self, maximum)
    }}
    fn write(self, value: String) -> Number {{
        return socketWrite(self, value)
    }}
    fn close(self) {{ socketClose(self) }}
}}
fn main() {{
    let socket = wait task Socket.connect("127.0.0.1", {port})
    let written = wait task socket.write("ping")
    println(wait task socket.read(4))
    socket.close()
}}
"#
    );
    let output = build_program(&source);
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4];
        connection.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"ping");
        connection.write_all(b"pong").unwrap();
    });
    let invocation = Command::new(&output)
        .output()
        .expect("built socket executable should run");
    let _ = std::fs::remove_file(&output);
    server.join().unwrap();
    assert_success(&invocation);
    assert_eq!(String::from_utf8_lossy(&invocation.stdout), "pong\n");
}

#[test]
fn builds_and_runs_component_state_recipes_and_functions() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn println(value: String)
@application
component Counter(start: Number = 40) {
    state count = start
    fn increment(amount: Number = 2) -> Number {
        count += amount
        return count
    }
    @startup
    recipe initialize { increment() }
    recipe view { println("count={increment(0)}") }
}
"#,
    );
    assert_eq!(stdout, "count=42\n");
}

#[test]
fn builds_and_runs_optional_holes_and_scalar_templates() {
    let stdout = build_and_run(
        r#"
@native("core.write_line")
fn printText(value: String)
@native("core.write_line")
fn printBool(value: bool)
struct Result { value: String? }
fn missing() -> String? { return null }
fn main() {
    let values: String[] = [, "set"]
    let result = Result { value: missing() }
    let signed: i16 = 12
    let unsigned: u32 = 34
    let single: f32 = 1.5
    let double: f64 = 2.25
    printText("{signed} {unsigned} {single} {double}")
    printText(values[0])
    printBool(result.value == null)
}
"#,
    );
    assert_eq!(stdout, "12 34 1.5 2.25\n\ntrue\n");
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
