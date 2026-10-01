use std::process::ExitCode;

fn main() -> ExitCode {
    match kome::execute_from_env() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
