use kome::{Cli, Command, Project};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> std::path::PathBuf {
    let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kome-project-test-{}-{name}-{serial}",
        std::process::id(),
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn discovers_a_project_and_uses_the_default_application_source() {
    let root = fixture("discovery");
    write(
        &root.join("Kome.toml"),
        "[package]\nname = \"hello\"\nversion = \"0.1.0\"\n",
    );
    write(&root.join("src/main.kome"), "fn main() {}\n");
    let nested = root.join("src/nested");
    fs::create_dir_all(&nested).unwrap();

    let project = Project::discover(&nested).unwrap();

    assert_eq!(project.root(), root);
    assert_eq!(
        project.application_source().unwrap(),
        root.join("src/main.kome")
    );
}

#[test]
fn resolves_a_local_library_dependency() {
    let root = fixture("dependency");
    write(
        &root.join("Kome.toml"),
        "[package]\nname = \"app\"\n\n[dependencies]\nui = { path = \"vendor/ui\" }\n",
    );
    write(&root.join("src/main.kome"), "fn main() {}\n");
    write(
        &root.join("vendor/ui/Kome.toml"),
        "[package]\nname = \"ui\"\n\n[lib]\nsource = \"src/lib.kome\"\n",
    );
    write(&root.join("vendor/ui/src/lib.kome"), "fn button() {}\n");

    let dependencies = Project::load(&root.join("Kome.toml"))
        .unwrap()
        .dependencies()
        .unwrap();

    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].name, "ui");
    assert_eq!(dependencies[0].source, root.join("vendor/ui/src/lib.kome"));
}

#[test]
fn parses_project_commands_and_build_options() {
    let cli = Cli::parse([
        "build".into(),
        "--manifest-path".into(),
        "examples/Kome.toml".into(),
        "--output".into(),
        "build/app".into(),
    ])
    .unwrap();

    assert_eq!(cli.command, Command::Build);
    assert_eq!(
        cli.manifest_path,
        Some(std::path::PathBuf::from("examples/Kome.toml"))
    );
    assert_eq!(cli.output, Some(std::path::PathBuf::from("build/app")));
}
