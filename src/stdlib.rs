use kome_ast::declarations::{Declaration, Module, UseImport};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
};

pub const STDLIB_PATH_ENV: &str = "KOME_STDLIB_PATH";

pub const KNOWN_PACKAGES: &[&str] = &["std"];

/// Returns whether `name` identifies a bundled standard-library package.
pub fn is_known_package(name: &str) -> bool {
    KNOWN_PACKAGES.contains(&name)
}

const KOMEUP_HOME_ENV: &str = "KOMEUP_HOME";

pub struct StandardLibrary {
    root: PathBuf,
    prelude_path: PathBuf,
    prelude_source: String,
    prelude: Module,
    packages: HashMap<String, PackageLibrary>,
}

#[derive(Debug, Deserialize)]
struct KomeupConfig {
    default_toolchain: String,
}

#[derive(Debug, Deserialize)]
struct PackageManifest {
    module: PackageModule,
    lib: PackageLib,
}

#[derive(Debug, Deserialize)]
struct PackageModule {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PackageLib {
    source: PathBuf,
}

#[derive(Debug, Clone)]
struct PackageLibrary {
    root: PathBuf,
    source: PathBuf,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub path: PathBuf,
    pub source: String,
    pub module: Module,
}

#[allow(unused)]
impl StandardLibrary {
    /// Discovers the active standard library from the environment or toolchain configuration.
    pub fn discover() -> Result<Self, String> {
        if let Some(raw_path) = env::var_os(STDLIB_PATH_ENV) {
            if raw_path.is_empty() {
                return Err(format!(
                    "{STDLIB_PATH_ENV} is set, \
                     but its value is empty",
                ));
            }

            return Self::load(PathBuf::from(raw_path));
        }

        if let Some(prefix) = installed_prefix() {
            let installed = prefix.join("stdlib");
            if installed.is_dir() {
                return Self::load_with_package_roots(installed, &[prefix]);
            }
        }

        let home = kome_home()?;
        let config_path = home.join("komeup.toml");

        if !config_path.is_file() {
            let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/stdlib");
            if bundled.is_dir() {
                let vendor = bundled
                    .parent()
                    .expect("bundled standard library has a parent")
                    .to_path_buf();
                return Self::load_with_package_roots(bundled, &[vendor]);
            }
        }

        let source = read_source(&config_path)?;

        let config = toml::from_str::<KomeupConfig>(&source)
            .map_err(|error| format!("failed to parse `{}`: {error}", config_path.display(),))?;

        let root = home
            .join("toolchains")
            .join(config.default_toolchain)
            .join("lib")
            .join("std");

        let package_root = root.parent().map(Path::to_path_buf);
        match package_root {
            Some(package_root) => Self::load_with_package_roots(root, &[package_root]),
            None => Self::load(root),
        }
    }

    /// Loads the standard library from [`STDLIB_PATH_ENV`].
    pub fn load_from_env() -> Result<Self, String> {
        let raw_path = env::var_os(STDLIB_PATH_ENV).ok_or_else(|| {
            format!(
                "{STDLIB_PATH_ENV} is not set; \
                         set it to the kome_std directory",
            )
        })?;

        if raw_path.is_empty() {
            return Err(format!(
                "{STDLIB_PATH_ENV} is set, \
                 but its value is empty",
            ));
        }

        Self::load(PathBuf::from(raw_path))
    }

    /// Loads a standard library rooted at `root`.
    pub fn load(root: PathBuf) -> Result<Self, String> {
        let package_roots = root
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .collect::<Vec<_>>();
        Self::load_with_package_roots(root, &package_roots)
    }

    fn load_with_package_roots(root: PathBuf, package_roots: &[PathBuf]) -> Result<Self, String> {
        let metadata = fs::metadata(&root).map_err(|error| {
            format!(
                "failed to access standard \
                         library directory `{}`: {error}",
                root.display(),
            )
        })?;

        if !metadata.is_dir() {
            return Err(format!(
                "standard library path `{}` \
                 is not a directory",
                root.display(),
            ));
        }

        let prelude_path = root.join("prelude.kome");

        let prelude_source = read_source(&prelude_path)?;

        let prelude = kome_parser::parse(&prelude_source)
            .map_err(|error| format!("{}: {error}", prelude_path.display(),))?;
        let packages = discover_packages(package_roots)?;

        Ok(Self {
            root,
            prelude_path,
            prelude_source,
            prelude,
            packages,
        })
    }

    /// Resolves and loads the prelude and standard-library modules imported by `application`.
    pub fn modules_for(&self, application: &Module) -> Result<Vec<LoadedModule>, String> {
        let mut modules = vec![LoadedModule {
            path: self.prelude_path.clone(),
            source: self.prelude_source.clone(),
            module: self.prelude.clone(),
        }];

        let mut pending = VecDeque::new();

        pending.extend(self.library_imports(&self.prelude));

        pending.extend(self.library_imports(application));

        let mut loaded = HashSet::new();

        while let Some(path) = pending.pop_front() {
            let key = path.join(".");

            if !loaded.insert(key) {
                continue;
            }

            let loaded_module = self.load_module(&path)?;

            pending.extend(self.library_imports(&loaded_module.module));

            modules.push(loaded_module);
        }

        Ok(modules)
    }

    /// Returns the root directory of this standard library.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the source path of the loaded prelude.
    pub fn prelude_path(&self) -> &Path {
        &self.prelude_path
    }

    /// Returns the parsed prelude module.
    pub fn prelude(&self) -> &Module {
        &self.prelude
    }

    /// Returns whether `name` identifies an available library package.
    pub fn has_package(&self, name: &str) -> bool {
        is_known_package(name) || self.packages.contains_key(name)
    }

    /// Resolves the source path represented by an imported package path.
    pub fn imported_module_path(&self, segments: &[&str]) -> Option<PathBuf> {
        self.resolve_module_path(segments).ok()
    }

    /// 従来どおりpreludeだけを結合します。
    pub fn merge_with(&self, mut application: Module) -> Module {
        let mut declarations =
            Vec::with_capacity(self.prelude.declarations.len() + application.declarations.len());

        declarations.extend(self.prelude.declarations.iter().cloned());

        declarations.append(&mut application.declarations);

        Module::new(declarations, application.span)
    }

    /// preludeと、アプリが`use std.*`で
    /// importしたモジュールを結合します。
    pub fn merge_with_imports(&self, mut application: Module) -> Result<Module, String> {
        let modules = self.modules_for(&application)?;

        let mut declarations = Vec::new();

        for loaded in modules {
            declarations.extend(
                loaded
                    .module
                    .declarations
                    .into_iter()
                    .filter(|declaration| !matches!(declaration, Declaration::Use(_),)),
            );
        }

        declarations.append(&mut application.declarations);

        Ok(Module::new(declarations, application.span))
    }

    fn load_module(&self, segments: &[String]) -> Result<LoadedModule, String> {
        let borrowed = segments.iter().map(String::as_str).collect::<Vec<_>>();
        let path = self.resolve_module_path(&borrowed)?;

        let source = read_source(&path)?;

        let module =
            kome_parser::parse(&source).map_err(|error| format!("{}: {error}", path.display(),))?;

        Ok(LoadedModule {
            path,
            source,
            module,
        })
    }

    fn resolve_module_path(&self, segments: &[&str]) -> Result<PathBuf, String> {
        let package_name = segments.first().copied().unwrap_or("");
        let (entry, remainder) = if is_known_package(package_name) {
            if segments.len() < 2 {
                return Err(format!(
                    "`{package_name}` must be followed by a module name"
                ));
            }
            (self.root.clone(), &segments[1..])
        } else if let Some(package) = self.packages.get(package_name) {
            if segments.len() == 1 {
                return Ok(package.source.clone());
            }
            let source_root = package
                .source
                .parent()
                .unwrap_or(&package.root)
                .to_path_buf();
            (source_root, &segments[1..])
        } else {
            let mut available = KNOWN_PACKAGES
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>();
            available.extend(self.packages.keys().cloned());
            available.sort();
            return Err(format!(
                "unknown package `{package_name}`; available packages: {}",
                available.join(", ")
            ));
        };

        let mut base = entry;
        for segment in remainder {
            base.push(segment);
        }
        let file_path = base.with_extension("kome");
        let module_path = base.join("mod.kome");
        if file_path.is_file() {
            Ok(file_path)
        } else if module_path.is_file() {
            Ok(module_path)
        } else {
            Err(format!(
                "package module `{}` was not found; expected `{}` or `{}`",
                segments.join("::"),
                file_path.display(),
                module_path.display(),
            ))
        }
    }

    fn library_imports(&self, module: &Module) -> Vec<Vec<String>> {
        module_imports(module)
            .into_iter()
            .filter(|segments| segments.first().is_some_and(|name| self.has_package(name)))
            .collect()
    }
}

fn module_imports(module: &Module) -> Vec<Vec<String>> {
    let mut imports = Vec::new();

    for declaration in &module.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };

        for import in &use_declaration.imports {
            let UseImport::Module(path) = import else {
                continue;
            };

            let segments = path
                .segments
                .iter()
                .filter_map(|segment| match &segment.kind {
                    kome_ast::declarations::PathSegmentKind::Ident(name) => Some(name.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();

            if !segments.is_empty() {
                imports.push(segments);
            }
        }
    }

    imports
}

fn discover_packages(roots: &[PathBuf]) -> Result<HashMap<String, PackageLibrary>, String> {
    let mut packages = HashMap::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries {
            let path = entry
                .map_err(|error| format!("failed to read `{}`: {error}", root.display()))?
                .path();
            if !path.is_dir() {
                continue;
            }
            for package_root in [path.clone(), path.join("lib")] {
                let manifest_path = package_root.join("Kome.toml");
                if !manifest_path.is_file() {
                    continue;
                }
                let manifest_source = read_source(&manifest_path)?;
                let manifest =
                    toml::from_str::<PackageManifest>(&manifest_source).map_err(|error| {
                        format!("failed to parse `{}`: {error}", manifest_path.display())
                    })?;
                let source = package_root.join(&manifest.lib.source);
                if !source.is_file() {
                    return Err(format!(
                        "package `{}` source `{}` was not found",
                        manifest.module.name,
                        source.display()
                    ));
                }
                packages.insert(
                    manifest.module.name,
                    PackageLibrary {
                        root: package_root,
                        source,
                    },
                );
                break;
            }
        }
    }
    Ok(packages)
}

fn installed_prefix() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    bin.parent().map(Path::to_path_buf)
}

fn kome_home() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(KOMEUP_HOME_ENV) {
        if path.is_empty() {
            return Err(format!(
                "{KOMEUP_HOME_ENV} is set, \
                 but its value is empty",
            ));
        }

        return Ok(PathBuf::from(path));
    }

    let home = env::var_os("HOME").ok_or_else(|| {
        "HOME is not set and \
                 KOMEUP_HOME was not provided"
            .to_owned()
    })?;

    Ok(PathBuf::from(home).join(".kome"))
}

fn read_source(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display(),))
}

#[cfg(test)]
mod tests {
    use super::StandardLibrary;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temporary_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "kome-library-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[test]
    fn discovers_sibling_packages_from_their_manifest() {
        let root = temporary_root();
        let stdlib = root.join("stdlib");
        let package = root.join("viewkit/lib");
        fs::create_dir_all(&stdlib).unwrap();
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(stdlib.join("prelude.kome"), "").unwrap();
        fs::write(
            package.join("Kome.toml"),
            "[module]\nname = \"viewKit\"\n[lib]\nsource = \"src/lib.kome\"\n",
        )
        .unwrap();
        fs::write(
            package.join("src/lib.kome"),
            "fn viewKitVersion() -> u32 { return 1 }\n",
        )
        .unwrap();

        let library = StandardLibrary::load(stdlib).unwrap();
        let application = kome_parser::parse("use viewKit\n").unwrap();
        let modules = library.modules_for(&application).unwrap();

        assert!(library.has_package("viewKit"));
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[1].path, package.join("src/lib.kome"));

        fs::remove_dir_all(root).unwrap();
    }
}
