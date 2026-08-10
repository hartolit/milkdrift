//! Stable opt-in boundary for the complete E1 CUDA hardware suite.

#![cfg(feature = "cuda-hardware-tests")]

use std::process::ExitCode;

type TestResult = Result<(), String>;

struct HardwareCase {
    name: &'static str,
    run: fn() -> TestResult,
}

macro_rules! hardware_cases {
    ($($case:ident),+ $(,)?) => {
        const HARDWARE_CASES: &[HardwareCase] = &[
            $(HardwareCase {
                name: stringify!($case),
                run: $case,
            }),+
        ];
    };
}

hardware_cases!(application_runtime_cuda_suite);

fn application_runtime_cuda_suite() -> TestResult {
    application_runtime::__run_cuda_hardware_suite()
}

fn run_hardware_suite() -> TestResult {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() != Ok("1") {
        return Err("set MILKDRIFT_CUDA_TEST=1 to execute the E1 CUDA hardware suite".to_owned());
    }
    if HARDWARE_CASES.is_empty() {
        return Err("application CUDA hardware target registered zero cases".to_owned());
    }

    let mut executed = 0_usize;
    for case in HARDWARE_CASES {
        executed = executed.saturating_add(1);
        eprintln!("running application CUDA suite boundary: {}", case.name);
        (case.run)()
            .map_err(|error| format!("application CUDA boundary {} failed: {error}", case.name))?;
    }
    if executed != HARDWARE_CASES.len() {
        return Err(format!(
            "application CUDA target executed {executed} of {} registered boundaries",
            HARDWARE_CASES.len()
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    match run_hardware_suite() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("E1 CUDA hardware suite failed: {error}");
            ExitCode::FAILURE
        }
    }
}
