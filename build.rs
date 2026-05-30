fn main() {
    let mut build = cc::Build::new();

    build.include("std");
    build.include("lib/viewKit");
    println!("cargo:rerun-if-changed=std");
    println!("cargo:rerun-if-changed=lib/viewKit/viewkit_shim.c");
    println!("cargo:rerun-if-changed=lib/viewKit/viewkit.h");
    println!("cargo:rerun-if-changed=lib/viewKit/components/components.c");
    println!("cargo:rerun-if-changed=lib/viewKit/components/components.h");

    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new("std") {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_file() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
            if let Some(ext) = entry.path().extension() {
                if ext == "c" {
                    files.push(entry.path().to_string_lossy().into_owned());
                }
            }
        }
    }

    if files.is_empty() {
        return;
    }

    for f in files.iter() {
        build.file(f);
    }

    // ViewKit の run_loop を別スレッドで回すため
    build.flag_if_supported("-pthread");

    build.file("lib/viewKit/viewkit_shim.c");
    build.file("lib/viewKit/components/components.c");

    build.compile("kome_std");

    println!("cargo:rustc-link-arg=-Wl,-export-dynamic");
}
