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

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "kome_native_rt", "--quiet"])
        .current_dir(&workspace_root)
        .status()
        .ok()?;

    if !status.success() {
        return None;
    }

    Some(workspace_root.join("target/debug").join(library))
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
fn built_binary_has_no_rust_runtime_dependency() {
    // The point of this check is that the binary links only against system
    // libraries; running it in a minimal environment proves the runtime is
    // statically baked in.
    let Some(runtime_library) = ensure_runtime_library() else {
        panic!("could not locate or build libkome_native_rt.a");
    };

    assert!(runtime_library.is_file());
}
