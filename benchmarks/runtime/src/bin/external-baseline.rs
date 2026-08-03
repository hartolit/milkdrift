//! Process entry point for the explicit external CPU/CUDA product baseline.

#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match runtime_benchmarks::run_external_baseline(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("external runtime baseline failed: {error}");
            ExitCode::FAILURE
        }
    }
}
