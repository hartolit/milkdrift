//! Stable opt-in boundary for the complete E1 CUDA hardware suite.

#![cfg(feature = "cuda-hardware-tests")]

use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() != Ok("1") {
        eprintln!(
            "E1 CUDA hardware suite failed: set MILKDRIFT_CUDA_TEST=1 to execute the E1 CUDA hardware suite"
        );
        return ExitCode::FAILURE;
    }

    match application_runtime::__run_cuda_hardware_suite() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("E1 CUDA hardware suite failed: {error}");
            ExitCode::FAILURE
        }
    }
}
