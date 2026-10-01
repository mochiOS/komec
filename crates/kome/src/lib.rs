//! Project discovery and manifest resolution for the `kome` command.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The conventional Kome project manifest filename.
pub const MANIFEST_FILE: &str = "Kome.toml";

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
}

/// A dependency declaration supported by the initial local resolver.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Dependency {
    /// A dependency located at a local filesystem path.
    Detailed { path: PathBuf },
    /// A shorthand local filesystem path.
    Path(PathBuf),
}

impl Dependency {
    /// Returns the unresolved path written in the manifest.
    pub fn path(&self) -> &Path {
        match self {
            Self::Detailed { path } | Self::Path(path) => path,
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

    /// Resolves all direct local dependencies in deterministic name order.
    pub fn dependencies(&self) -> Result<Vec<ResolvedDependency>, String> {
        self.manifest
            .dependencies
            .iter()
            .map(|(name, dependency)| self.resolve_dependency(name, dependency))
            .collect()
    }

    fn resolve_dependency(
        &self,
        name: &str,
        dependency: &Dependency,
    ) -> Result<ResolvedDependency, String> {
        let root = self.root.join(dependency.path());
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

        Ok(ResolvedDependency {
            name: name.to_owned(),
            root: project.root,
            source,
        })
    }
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
