mod stdlib;

use kome_ast::declarations::Module;
use kome_semantics::{error::ResolutionError, resolver::ScopeBuilder};
use std::{env, fs, path::Path, path::PathBuf, process::ExitCode};

const USAGE: &str = "usage: komec <check|run|build> <file> [output]";

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

    let output = arguments.next();

    if arguments.next().is_some() {
        return Err(USAGE.to_string());
    }

    match command {
        "check" => check(Path::new(&path)),

        "run" => {
            if output.is_some() {
                return Err(USAGE.to_string());
            }

            run_program(Path::new(&path))
        }

        "build" => build_program(Path::new(&path), output.as_deref()),

        unknown => Err(format!("unknown command `{unknown}`\n{USAGE}")),
    }
}

fn check(path: &Path) -> Result<(), String> {
    load_checked_module(path)?;

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

fn run_program(path: &Path) -> Result<(), String> {
    let module = load_checked_module(path)?;

    kome_jit::execute(&module, "main").map_err(|error| error.to_string())
}

fn build_program(path: &Path, output: Option<&std::ffi::OsStr>) -> Result<(), String> {
    let module = load_checked_module(path)?;

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

fn load_checked_module(path: &Path) -> Result<Module, String> {
    // let standard_library = StandardLibrary::discover()?;

    // let prelude_resolution = ScopeBuilder::resolve(standard_library.prelude());

    // if !prelude_resolution.errors.is_empty() {
    //     print_resolution_errors(standard_library.prelude_path(), &prelude_resolution.errors);

    //     return Err(format!(
    //         "standard library check failed with {} semantic error(s)",
    //         prelude_resolution.errors.len(),
    //     ));
    // }

    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display(),))?;

    let module =
        kome_parser::parse(&source).map_err(|error| format!("{}: {error}", path.display()))?;

    // let module = standard_library.merge_with_imports(application)?;

    let resolution = ScopeBuilder::resolve(&module);

    if !resolution.errors.is_empty() {
        print_resolution_errors(path, &resolution.errors);

        return Err(format!(
            "check failed with {} semantic error(s)",
            resolution.errors.len(),
        ));
    }

    Ok(module)
}
