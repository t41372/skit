#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(skit_cli::entry())
}
