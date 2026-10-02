use kome_ast::declarations::{Declaration, Module, UseImport};
use serde::Deserialize;
use std::{
    collections::{HashSet, VecDeque},
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
}

#[derive(Debug, Deserialize)]
struct KomeupConfig {
    default_toolchain: String,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct LoadedModule {
    /// Canonical module path beginning with `std`.
    pub name: Vec<String>,
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
                return Self::load(installed);
            }
        }

        let home = kome_home()?;
        let config_path = home.join("komeup.toml");

        if !config_path.is_file() {
            let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/stdlib");
            if bundled.is_dir() {
                return Self::load(bundled);
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

        Self::load(root)
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

        Ok(Self { root })
    }

    /// Resolves and loads the standard-library modules imported by `application`.
    pub fn modules_for(&self, application: &Module) -> Result<Vec<LoadedModule>, String> {
        let mut modules = Vec::new();
        let mut pending = VecDeque::new();

        pending.extend(standard_library_imports(application));

        let mut loaded = HashSet::new();

        while let Some(mut path) = pending.pop_front() {
            if !self.module_exists(&path)
                && path.len() > 2
                && self.module_exists(&path[..path.len() - 1])
            {
                path.pop();
            }
            let key = path.join(".");

            if !loaded.insert(key) {
                continue;
            }

            let loaded_module = self.load_module(&path)?;

            pending.extend(standard_library_imports(&loaded_module.module));

            modules.push(loaded_module);
        }

        Ok(modules)
    }

    /// Returns the root directory of this standard library.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Combines the application with modules imported through `use std::...`.
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
        let package_name = segments.first().map(String::as_str).unwrap_or("");

        if !is_known_package(package_name) {
            return Err(format!(
                "unknown package `{}`; expected one of {:?}",
                package_name, KNOWN_PACKAGES,
            ));
        }

        if segments.len() < 2 {
            return Err(format!(
                "`{package_name}` must be followed by a module name"
            ));
        }

        let mut base = self.root.clone();

        for segment in &segments[1..] {
            base.push(segment);
        }

        let file_path = base.with_extension("kome");
        let module_path = base.join("mod.kome");
        let path = if file_path.is_file() {
            file_path
        } else if module_path.is_file() {
            module_path
        } else {
            return Err(format!(
                "package module `{}` was not found; expected `{}` or `{}`",
                segments.join("::"),
                file_path.display(),
                module_path.display(),
            ));
        };

        let source = read_source(&path)?;

        let module =
            kome_parser::parse(&source).map_err(|error| format!("{}: {error}", path.display(),))?;

        Ok(LoadedModule {
            name: segments.to_vec(),
            path,
            source,
            module,
        })
    }

    fn module_exists(&self, segments: &[String]) -> bool {
        if segments.len() < 2 || !is_known_package(&segments[0]) {
            return false;
        }
        let mut base = self.root.clone();
        for segment in &segments[1..] {
            base.push(segment);
        }
        base.with_extension("kome").is_file() || base.join("mod.kome").is_file()
    }
}

fn standard_library_imports(module: &Module) -> Vec<Vec<String>> {
    let mut imports = Vec::new();

    for declaration in &module.declarations {
        let Declaration::Use(use_declaration) = declaration else {
            continue;
        };

        for import in &use_declaration.imports {
            let path = match import {
                UseImport::Module(path)
                | UseImport::AliasedModule { path, .. }
                | UseImport::WildcardFrom { path, .. } => path,
                UseImport::Wildcard { .. } => continue,
            };

            let segments = path
                .segments
                .iter()
                .filter_map(|segment| match &segment.kind {
                    kome_ast::declarations::PathSegmentKind::Ident(name) => Some(name.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();

            if segments.first().is_some_and(|name| is_known_package(name)) && segments.len() > 1 {
                imports.push(segments);
            }
        }
    }

    imports
}

fn installed_prefix() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    installed_prefix_from(&executable)
}

fn installed_prefix_from(executable: &Path) -> Option<PathBuf> {
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
    use super::{StandardLibrary, installed_prefix_from};
    use std::path::PathBuf;

    #[test]
    fn derives_the_installation_prefix_from_the_bin_directory() {
        assert_eq!(
            installed_prefix_from(std::path::Path::new("/home/user/.kome/bin/komec")),
            Some(PathBuf::from("/home/user/.kome")),
        );
        assert_eq!(
            installed_prefix_from(std::path::Path::new("/workspace/target/debug/komec")),
            None,
        );
    }

    #[test]
    fn loads_explicit_and_transitive_standard_modules() {
        let standard_library =
            StandardLibrary::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/stdlib"))
                .unwrap();

        let empty = kome_parser::parse("fn main() {}").unwrap();
        assert!(standard_library.modules_for(&empty).unwrap().is_empty());

        let imported = kome_parser::parse("use std::core\nuse std::net\nfn main() {}").unwrap();
        let modules = standard_library.modules_for(&imported).unwrap();

        assert_eq!(modules.len(), 3);
        assert!(
            modules
                .iter()
                .any(|module| module.path.ends_with("core/mod.kome"))
        );
        assert!(
            modules
                .iter()
                .any(|module| module.path.ends_with("net/mod.kome"))
        );
        assert!(
            modules
                .iter()
                .any(|module| module.path.ends_with("io/mod.kome"))
        );
    }
}
