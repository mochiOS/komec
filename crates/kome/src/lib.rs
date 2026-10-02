//! Project discovery and manifest resolution for the `kome` command.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus};

/// The conventional Kome project manifest filename.
pub const MANIFEST_FILE: &str = "Kome.toml";

/// Environment variable that overrides the compiler executable used by `kome`.
pub const COMPILER_ENV: &str = "KOMEC";

/// A parsed Kome package manifest.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Package identity and metadata.
    pub package: Package,
    /// Optional application target configuration.
    #[serde(default)]
    pub application: Option<Target>,
    /// Optional library target configuration.
    #[serde(default)]
    pub lib: Option<Target>,
    /// Local package dependencies keyed by their import name.
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
}

/// Package identity stored in `[package]`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Package {
    /// Human-readable package name.
    pub name: String,
    /// Package version, when one has been assigned.
    #[serde(default)]
    pub version: Option<String>,
}

/// A Kome source target.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Target {
    /// Source file relative to the manifest directory.
    pub source: PathBuf,
    /// Additional source files that form the same target.
    #[serde(default)]
    pub sources: Vec<PathBuf>,
}

/// A dependency declaration supported by the initial local resolver.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Dependency {
    /// A dependency located at a local filesystem path.
    Detailed { path: PathBuf },
    /// A package installed beside the Kome toolchain or bundled for development.
    System { system: bool },
    /// A shorthand local filesystem path.
    Path(PathBuf),
}

impl Dependency {
    /// Returns the unresolved local path written in the manifest, if present.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Detailed { path } | Self::Path(path) => Some(path),
            Self::System { .. } => None,
        }
    }
}

/// A project manifest together with its resolved filesystem location.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest: Manifest,
}

impl Project {
    /// Finds `Kome.toml` from `start` or one of its parent directories.
    pub fn discover(start: &Path) -> Result<Self, String> {
        let start = if start.is_file() {
            start.parent().unwrap_or(start)
        } else {
            start
        };

        for directory in start.ancestors() {
            let candidate = directory.join(MANIFEST_FILE);
            if candidate.is_file() {
                return Self::load(&candidate);
            }
        }

        Err(format!(
            "could not find `{MANIFEST_FILE}` in `{}` or a parent directory",
            start.display(),
        ))
    }

    /// Loads and validates a project from a manifest path.
    pub fn load(manifest_path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(manifest_path)
            .map_err(|error| format!("failed to read `{}`: {error}", manifest_path.display()))?;
        let manifest = toml::from_str::<Manifest>(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", manifest_path.display()))?;
        validate_manifest(&manifest, manifest_path)?;

        let root = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        Ok(Self {
            root,
            manifest_path: manifest_path.to_path_buf(),
            manifest,
        })
    }

    /// Returns the package root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the path of the loaded manifest.
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Returns the parsed manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Resolves the application source, defaulting to `src/main.kome`.
    pub fn application_source(&self) -> Result<PathBuf, String> {
        let relative = self
            .manifest
            .application
            .as_ref()
            .map(|target| target.source.as_path())
            .unwrap_or_else(|| Path::new("src/main.kome"));
        let source = self.root.join(relative);
        require_file(&source, "application source")?;
        Ok(source)
    }

    /// Resolves local and system dependencies recursively in deterministic order.
    pub fn dependencies(&self) -> Result<Vec<ResolvedDependency>, String> {
        self.dependencies_with_system_roots(&system_package_roots())
    }

    fn dependencies_with_system_roots(
        &self,
        system_roots: &[PathBuf],
    ) -> Result<Vec<ResolvedDependency>, String> {
        let mut resolved = Vec::new();
        let mut visited = Vec::new();
        self.resolve_dependencies(system_roots, &mut visited, &mut resolved)?;
        Ok(resolved)
    }

    fn resolve_dependencies(
        &self,
        system_roots: &[PathBuf],
        visited: &mut Vec<PathBuf>,
        resolved: &mut Vec<ResolvedDependency>,
    ) -> Result<(), String> {
        for (name, dependency) in &self.manifest.dependencies {
            let dependency = self.resolve_dependency(name, dependency, system_roots)?;
            let identity = dependency
                .root
                .canonicalize()
                .unwrap_or_else(|_| dependency.root.clone());
            if visited.contains(&identity) {
                continue;
            }
            visited.push(identity);

            let project = Self::load(&dependency.root.join(MANIFEST_FILE))?;
            project.resolve_dependencies(system_roots, visited, resolved)?;
            resolved.push(dependency);
        }
        Ok(())
    }

    fn resolve_dependency(
        &self,
        name: &str,
        dependency: &Dependency,
        system_roots: &[PathBuf],
    ) -> Result<ResolvedDependency, String> {
        let root = match dependency {
            Dependency::Detailed { path } | Dependency::Path(path) => self.root.join(path),
            Dependency::System { system: true } => system_roots
                .iter()
                .flat_map(|root| [root.join(name), root.join(name).join("lib")])
                .find(|candidate| candidate.join(MANIFEST_FILE).is_file())
                .ok_or_else(|| {
                    format!(
                        "system dependency `{name}` was not found in {}",
                        system_roots
                            .iter()
                            .map(|path| format!("`{}`", path.display()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
            Dependency::System { system: false } => {
                return Err(format!(
                    "dependency `{name}` must set `system = true` or specify `path`"
                ));
            }
        };
        let manifest_path = root.join(MANIFEST_FILE);
        let project = Self::load(&manifest_path).map_err(|error| {
            format!(
                "failed to resolve dependency `{name}` from `{}`: {error}",
                root.display()
            )
        })?;
        let target = project.manifest.lib.as_ref().ok_or_else(|| {
            format!(
                "dependency `{name}` has no `[lib]` target in `{}`",
                project.manifest_path.display(),
            )
        })?;
        let source = project.root.join(&target.source);
        require_file(&source, "dependency source")?;
        let mut sources = Vec::with_capacity(target.sources.len());
        for relative in &target.sources {
            let additional = project.root.join(relative);
            require_file(&additional, "dependency source")?;
            if additional == source || sources.contains(&additional) {
                return Err(format!(
                    "dependency `{name}` lists duplicate source `{}`",
                    additional.display()
                ));
            }
            sources.push(additional);
        }

        Ok(ResolvedDependency {
            name: name.to_owned(),
            root: project.root,
            source,
            sources,
        })
    }
}

fn system_package_roots() -> Vec<PathBuf> {
    let Ok(executable) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(binary_directory) = executable.parent() else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    if let Some(installation_root) = binary_directory.parent() {
        roots.push(installation_root.to_path_buf());
    }
    if let Some(repository_root) = binary_directory.parent().and_then(Path::parent) {
        roots.push(repository_root.join("vendor"));
        roots.push(repository_root.join("vendor/devkit/crates"));
    }
    roots
}

/// A user-facing project command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Type-checks the application without generating an executable.
    Check,
    /// Builds the application as an AOT executable.
    Build,
    /// Compiles and runs the application through the JIT.
    Run,
    /// Runs each `tests/*.kome` integration test.
    Test,
}

/// Options accepted by the `kome` command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    /// Requested project operation.
    pub command: Command,
    /// Explicit manifest path, or `None` to discover it from the current directory.
    pub manifest_path: Option<PathBuf>,
    /// Explicit AOT output path for `build`.
    pub output: Option<PathBuf>,
}

impl Cli {
    /// Parses command-line arguments excluding the executable name.
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let command = match arguments
            .next()
            .and_then(|argument| argument.into_string().ok())
            .as_deref()
        {
            Some("check") => Command::Check,
            Some("build") => Command::Build,
            Some("run") => Command::Run,
            Some("test") => Command::Test,
            Some(command) => return Err(format!("unknown command `{command}`\n{USAGE}")),
            None => return Err(USAGE.to_owned()),
        };
        let mut manifest_path = None;
        let mut output = None;

        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--manifest-path") => {
                    manifest_path = Some(
                        arguments
                            .next()
                            .ok_or_else(|| "`--manifest-path` requires a path".to_owned())?
                            .into(),
                    );
                }
                Some("--output") if command == Command::Build => {
                    output = Some(
                        arguments
                            .next()
                            .ok_or_else(|| "`--output` requires a path".to_owned())?
                            .into(),
                    );
                }
                _ => return Err(USAGE.to_owned()),
            }
        }

        Ok(Self {
            command,
            manifest_path,
            output,
        })
    }
}

/// Runs a parsed `kome` command and waits for the compiler or test programs.
pub fn execute(cli: &Cli, current_directory: &Path) -> Result<(), String> {
    let project = match &cli.manifest_path {
        Some(path) => Project::load(path),
        None => Project::discover(current_directory),
    }?;
    let dependencies = project.dependencies()?;
    let compiler = compiler_path();

    match cli.command {
        Command::Check => {
            run_compiler(
                &compiler,
                "check",
                &project.application_source()?,
                None,
                &dependencies,
            )?;
        }
        Command::Run => {
            run_compiler(
                &compiler,
                "run",
                &project.application_source()?,
                None,
                &dependencies,
            )?;
        }
        Command::Build => {
            let output = match &cli.output {
                Some(path) if path.is_absolute() => path.clone(),
                Some(path) => current_directory.join(path),
                None => project
                    .root()
                    .join("target/debug")
                    .join(&project.manifest().package.name),
            };
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
            }
            run_compiler(
                &compiler,
                "build",
                &project.application_source()?,
                Some(&output),
                &dependencies,
            )?;
            println!("built `{}`", output.display());
        }
        Command::Test => run_tests(&compiler, &project, &dependencies)?,
    }

    Ok(())
}

/// Parses process arguments and runs the requested project command.
pub fn execute_from_env() -> Result<(), String> {
    let cli = Cli::parse(std::env::args_os().skip(1))?;
    let current_directory = std::env::current_dir()
        .map_err(|error| format!("failed to determine the current directory: {error}"))?;
    execute(&cli, &current_directory)
}

const USAGE: &str =
    "usage: kome <check|build|run|test> [--manifest-path <Kome.toml>] [--output <path>]";

fn run_tests(
    compiler: &Path,
    project: &Project,
    dependencies: &[ResolvedDependency],
) -> Result<(), String> {
    let tests_directory = project.root().join("tests");
    let mut tests = if tests_directory.is_dir() {
        fs::read_dir(&tests_directory)
            .map_err(|error| format!("failed to read `{}`: {error}", tests_directory.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "kome")
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    tests.sort();

    if tests.is_empty() {
        println!(
            "no integration tests found in `{}`",
            tests_directory.display()
        );
        return Ok(());
    }

    for test in &tests {
        println!("running `{}`", test.display());
        run_compiler(compiler, "run", test, None, dependencies)?;
    }
    println!("{} test(s) passed", tests.len());
    Ok(())
}

fn run_compiler(
    compiler: &Path,
    command: &str,
    source: &Path,
    output: Option<&Path>,
    dependencies: &[ResolvedDependency],
) -> Result<(), String> {
    let mut invocation = ProcessCommand::new(compiler);
    invocation.arg(command).arg(source);
    if let Some(output) = output {
        invocation.arg(output);
    }
    for dependency in dependencies {
        invocation
            .arg("--package-source")
            .arg(&dependency.name)
            .arg(&dependency.source);
        for source in &dependency.sources {
            invocation
                .arg("--package-source")
                .arg(&dependency.name)
                .arg(source);
        }
    }

    let mut library_paths = std::env::var_os("KOME_LIBRARY_PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    for path in native_library_paths(dependencies) {
        if !library_paths.contains(&path) {
            library_paths.push(path);
        }
    }
    if !library_paths.is_empty() {
        let value = std::env::join_paths(library_paths.iter())
            .map_err(|error| format!("failed to construct native library search path: {error}"))?;
        invocation.env("KOME_LIBRARY_PATH", value);
    }

    let status = invocation.status().map_err(|error| {
        format!(
            "failed to execute compiler `{}`: {error}; set {COMPILER_ENV} to the komec path",
            compiler.display(),
        )
    })?;
    require_success(status, compiler)
}

fn require_success(status: ExitStatus, compiler: &Path) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "compiler `{}` exited with {status}",
            compiler.display(),
        ))
    }
}

fn compiler_path() -> PathBuf {
    if let Some(path) = std::env::var_os(COMPILER_ENV).filter(|path| !path.is_empty()) {
        return path.into();
    }

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let sibling = directory.join("komec");
        if sibling.is_file() {
            return sibling;
        }
    }

    PathBuf::from("komec")
}

fn native_library_paths(dependencies: &[ResolvedDependency]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dependency in dependencies {
        let mut candidates = vec![
            dependency.root.clone(),
            dependency.root.join("target/debug"),
            dependency.root.join("target/release"),
        ];
        if dependency
            .root
            .file_name()
            .is_some_and(|name| name == "lib")
            && let Some(package_root) = dependency.root.parent()
        {
            candidates.extend([
                package_root.to_path_buf(),
                package_root.join("target/debug"),
                package_root.join("target/release"),
            ]);
        }
        for candidate in candidates {
            if candidate.is_dir() && !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

/// A local dependency resolved to concrete compiler inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDependency {
    /// Import name used by the application.
    pub name: String,
    /// Root directory containing the dependency manifest.
    pub root: PathBuf,
    /// Concrete Kome library source file.
    pub source: PathBuf,
    /// Additional Kome source files in this library target.
    pub sources: Vec<PathBuf>,
}

fn validate_manifest(manifest: &Manifest, path: &Path) -> Result<(), String> {
    if manifest.package.name.trim().is_empty() {
        return Err(format!(
            "`package.name` must not be empty in `{}`",
            path.display(),
        ));
    }

    Ok(())
}

fn require_file(path: &Path, description: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("{description} `{}` was not found", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn fixture(name: &str) -> PathBuf {
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kome-system-package-test-{}-{name}-{serial}",
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
    fn resolves_system_packages_and_their_dependencies() {
        let root = fixture("recursive");
        let project_root = root.join("project");
        let packages = root.join("packages");
        write(
            &project_root.join(MANIFEST_FILE),
            "[package]\nname = \"app\"\n[dependencies]\nappcore = { system = true }\n",
        );
        write(
            &packages.join("appcore/Kome.toml"),
            "[package]\nname = \"appcore\"\n[lib]\nsource = \"src/lib.kome\"\n[dependencies]\nviewkit = { system = true }\n",
        );
        write(
            &packages.join("appcore/src/lib.kome"),
            "struct Clipboard {}\n",
        );
        write(
            &packages.join("viewkit/lib/Kome.toml"),
            "[package]\nname = \"viewkit\"\n[lib]\nsource = \"src/lib.kome\"\n",
        );
        write(
            &packages.join("viewkit/lib/src/lib.kome"),
            "struct View {}\n",
        );

        let dependencies = Project::load(&project_root.join(MANIFEST_FILE))
            .unwrap()
            .dependencies_with_system_roots(std::slice::from_ref(&packages))
            .unwrap();

        assert_eq!(
            dependencies
                .iter()
                .map(|dependency| dependency.name.as_str())
                .collect::<Vec<_>>(),
            ["viewkit", "appcore"]
        );
    }

    #[test]
    fn rejects_disabled_system_dependencies() {
        let root = fixture("disabled");
        write(
            &root.join(MANIFEST_FILE),
            "[package]\nname = \"app\"\n[dependencies]\nappcore = { system = false }\n",
        );

        let error = Project::load(&root.join(MANIFEST_FILE))
            .unwrap()
            .dependencies_with_system_roots(&[])
            .unwrap_err();

        assert!(error.contains("must set `system = true`"));
    }

    #[test]
    fn finds_native_builds_beside_a_development_kome_library() {
        let root = fixture("native-layout");
        let library_root = root.join("viewkit/lib");
        fs::create_dir_all(root.join("viewkit/target/debug")).unwrap();
        fs::create_dir_all(&library_root).unwrap();
        let dependency = ResolvedDependency {
            name: "viewkit".to_owned(),
            root: library_root,
            source: root.join("viewkit/lib/src/lib.kome"),
            sources: Vec::new(),
        };

        let paths = native_library_paths(&[dependency]);

        assert!(paths.contains(&root.join("viewkit/target/debug")));
    }
}
