mod stdlib;

use kome_ast::declarations::Module;
use kome_semantics::{
    error::ResolutionError,
    initialization::InitializationChecker,
    modules::{SourceModule, link_modules},
    resolver::ScopeBuilder,
    typecheck::TypeChecker,
};
use std::{env, fs, path::Path, path::PathBuf, process::ExitCode};

const USAGE: &str =
    "usage: komec <check|run|build> <file> [output] [--package-source <package> <source>]...";

#[derive(Debug)]
struct DependencySource {
    package: String,
    path: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);

    let command = arguments.next().ok_or_else(|| USAGE.to_string())?;

    let command = command
        .to_str()
        .ok_or_else(|| "command must be valid UTF-8".to_string())?;

    let path = arguments.next().ok_or_else(|| USAGE.to_string())?;

    let mut output = None;
    let mut dependency_sources = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--package-source" {
            let package = arguments
                .next()
                .ok_or_else(|| "`--package-source` requires a package name".to_string())?
                .into_string()
                .map_err(|_| "package names must be valid UTF-8".to_string())?;
            let path = arguments
                .next()
                .ok_or_else(|| "`--package-source` requires a source path".to_string())?
                .into();
            dependency_sources.push(DependencySource { package, path });
        } else if command == "build" && output.is_none() {
            output = Some(argument);
        } else {
            return Err(USAGE.to_string());
        }
    }

    match command {
        "check" => check(Path::new(&path), &dependency_sources),

        "run" => {
            if output.is_some() {
                return Err(USAGE.to_string());
            }

            run_program(Path::new(&path), &dependency_sources)
        }

        "build" => build_program(Path::new(&path), output.as_deref(), &dependency_sources),

        unknown => Err(format!("unknown command `{unknown}`\n{USAGE}")),
    }
}

fn check(path: &Path, dependency_sources: &[DependencySource]) -> Result<(), String> {
    load_checked_module(path, dependency_sources)?;

    println!("{}: check succeeded", path.display());

    Ok(())
}

fn print_resolution_errors(path: &Path, errors: &[ResolutionError]) {
    for error in errors {
        eprintln!("{}: {}", path.display(), format_resolution_error(error),);
    }
}

fn format_resolution_error(error: &ResolutionError) -> String {
    match error {
        ResolutionError::UndefinedName { name, span } => {
            format!(
                "undefined name `{name}` at byte range {}..{}",
                span.start, span.end,
            )
        }

        ResolutionError::DuplicateDefinition {
            name,
            first,
            second,
        } => {
            format!(
                "duplicate definition of `{name}` at byte range {}..{}; \
                 first defined at byte range {}..{}",
                second.start, second.end, first.start, first.end,
            )
        }

        ResolutionError::AssignmentToImmutable { name, span } => {
            format!(
                "cannot assign to immutable variable `{name}` at byte range {}..{}",
                span.start, span.end,
            )
        }

        ResolutionError::ScopeStackEmpty => "internal error: scope stack is empty".to_string(),

        ResolutionError::InvalidLetLocation { span } => {
            format!(
                "`let` is not allowed here at byte range {}..{}",
                span.start, span.end,
            )
        }
    }
}

fn run_program(path: &Path, dependency_sources: &[DependencySource]) -> Result<(), String> {
    let module = load_checked_module(path, dependency_sources)?;

    kome_jit::execute(&module, "main").map_err(|error| error.to_string())
}

fn build_program(
    path: &Path,
    output: Option<&std::ffi::OsStr>,
    dependency_sources: &[DependencySource],
) -> Result<(), String> {
    let module = load_checked_module(path, dependency_sources)?;

    let output = match output {
        Some(path) => PathBuf::from(path),

        None => PathBuf::from(
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("kome-program"),
        ),
    };

    kome_aot::build_executable(&module, &output).map_err(|error| error.to_string())?;

    println!("built `{}`", output.display());

    Ok(())
}

fn load_checked_module(
    path: &Path,
    dependency_sources: &[DependencySource],
) -> Result<Module, String> {
    let standard_library = stdlib::StandardLibrary::discover()?;

    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display(),))?;

    let application =
        kome_parser::parse(&source).map_err(|error| format!("{}: {error}", path.display()))?;

    let mut dependency_modules = Vec::new();
    let mut import_declarations = Vec::new();

    for dependency in dependency_sources {
        let dependency_source = fs::read_to_string(&dependency.path).map_err(|error| {
            format!(
                "failed to read dependency source `{}`: {error}",
                dependency.path.display(),
            )
        })?;
        let dependency = kome_parser::parse(&dependency_source)
            .map_err(|error| format!("{}: {error}", dependency.path.display()))?;
        import_declarations.extend(dependency.declarations.clone());
        dependency_modules.push(dependency);
    }

    import_declarations.extend(application.declarations.clone());
    let import_module = Module::new(import_declarations, application.span);
    let standard_modules = standard_library.modules_for(&import_module)?;
    let mut sources = standard_modules
        .into_iter()
        .map(|loaded| {
            SourceModule::new(
                "std",
                loaded.name.into_iter().skip(1).collect(),
                loaded.module,
                false,
            )
        })
        .collect::<Vec<_>>();
    sources.extend(dependency_sources.iter().zip(dependency_modules).map(
        |(dependency, module)| SourceModule::new(&dependency.package, Vec::new(), module, false),
    ));
    sources.push(SourceModule::new("__app", Vec::new(), application, true));
    let module = link_modules(sources).map_err(|errors| {
        for error in &errors {
            eprintln!("{}: {error}", path.display());
        }
        format!("check failed with {} module error(s)", errors.len())
    })?;

    let resolution = ScopeBuilder::resolve(&module);

    if !resolution.errors.is_empty() {
        print_resolution_errors(path, &resolution.errors);

        return Err(format!(
            "check failed with {} semantic error(s)",
            resolution.errors.len(),
        ));
    }

    let initialization = InitializationChecker::check(&module);

    if !initialization.errors.is_empty() {
        for error in &initialization.errors {
            eprintln!("{}: {error}", path.display());
        }

        return Err(format!(
            "check failed with {} initialization error(s)",
            initialization.errors.len(),
        ));
    }

    let type_check = TypeChecker::check(&module);

    if !type_check.errors.is_empty() {
        for error in &type_check.errors {
            eprintln!("{}: {error}", path.display());
        }

        return Err(format!(
            "check failed with {} type error(s)",
            type_check.errors.len(),
        ));
    }

    Ok(module)
}
