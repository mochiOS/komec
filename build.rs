fn main() {
    let mut build = cc::Build::new();

    build.include("std");

    // Collect .c files under std/
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new("std") {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_type().is_file() {
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

    build.compile("kome_std");
}

