//! Process entry point for the bounded normal baseline runner.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match runtime_benchmarks::run(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("runtime baseline failed: {error}");
            ExitCode::FAILURE
        }
    }
}
