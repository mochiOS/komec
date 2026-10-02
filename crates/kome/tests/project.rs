use kome::docs::generate_reference;
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
fn resolves_all_library_target_sources() {
    let root = fixture("multiple-sources");
    write(
        &root.join("Kome.toml"),
        "[package]\nname = \"app\"\n[dependencies]\nappcore = { path = \"vendor/appcore\" }\n",
    );
    write(&root.join("src/main.kome"), "fn main() {}\n");
    write(
        &root.join("vendor/appcore/Kome.toml"),
        "[package]\nname = \"appcore\"\n[lib]\nsource = \"src/lib.kome\"\nsources = [\"src/clipboard.kome\", \"src/document.kome\"]\n",
    );
    write(&root.join("vendor/appcore/src/lib.kome"), "");
    write(
        &root.join("vendor/appcore/src/clipboard.kome"),
        "struct Clipboard {}\n",
    );
    write(
        &root.join("vendor/appcore/src/document.kome"),
        "struct Document {}\n",
    );

    let dependencies = Project::load(&root.join("Kome.toml"))
        .unwrap()
        .dependencies()
        .unwrap();

    assert_eq!(dependencies[0].sources.len(), 2);
    assert!(dependencies[0].sources[0].ends_with("src/clipboard.kome"));
    assert!(dependencies[0].sources[1].ends_with("src/document.kome"));
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

#[test]
fn parses_documentation_output_options() {
    let cli = Cli::parse(["doc".into(), "--output".into(), "reference/api.md".into()]).unwrap();

    assert_eq!(cli.command, Command::Doc);
    assert_eq!(
        cli.output,
        Some(std::path::PathBuf::from("reference/api.md"))
    );
}

#[test]
fn generates_reference_for_public_library_apis() {
    let root = fixture("documentation");
    write(
        &root.join("Kome.toml"),
        "[package]\nname = \"sample\"\nversion = \"1.0.0\"\n[lib]\nsource = \"src/lib.kome\"\n",
    );
    write(
        &root.join("src/lib.kome"),
        r#"/// 公開する値です。
pub struct Value {
    /// 数値です。
    pub number: Number,
    hidden: Number,
}

for Value {
    /// 数値を返します。
    pub fn get(self) -> Number { return self.number }
    fn hidden(self) {}
}

fn internal() {}
"#,
    );
    let project = Project::load(&root.join("Kome.toml")).unwrap();
    let output = root.join("target/doc/sample.md");

    generate_reference(&project, &output).unwrap();
    let generated = fs::read_to_string(output).unwrap();

    assert!(generated.contains("# sample"));
    assert!(generated.contains("構造体 `Value`"));
    assert!(generated.contains("公開する値です。"));
    assert!(generated.contains("フィールド `number`"));
    assert!(generated.contains("関数 `Value.get`"));
    assert!(!generated.contains("internal"));
    assert!(!generated.contains("Value.hidden"));
}
