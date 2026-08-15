//! Opt-in, download-free CUDA hardware suite for hosted E0 lifecycle and accounting.

#![cfg(feature = "cuda-hardware-tests")]

use std::process::ExitCode;

#[allow(
    dead_code,
    reason = "the harness-free CUDA target reuses one case from broader shared integration support"
)]
#[path = "support/native_backend/mod.rs"]
mod native_backend;

use native_backend::TestResult;

struct HardwareCase {
    name: &'static str,
    run: fn() -> TestResult,
}

macro_rules! hardware_cases {
    (
        $(
            fn $case:ident() -> TestResult $body:block
        )+
    ) => {
        $(
            fn $case() -> TestResult $body
        )+

        const HARDWARE_CASES: &[HardwareCase] = &[
            $(HardwareCase {
                name: stringify!($case),
                run: $case,
            }),+
        ];
    };
}

hardware_cases!(
    fn hosted_e0_cuda_lifecycle_and_accounting() -> TestResult {
        native_backend::candle_mixed_cuda_fixture_covers_e0_generation_accounting_and_lifecycle()
    }
);

fn require_cuda_opt_in() -> TestResult {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() == Ok("1") {
        Ok(())
    } else {
        Err("set MILKDRIFT_CUDA_TEST=1 to execute the E0 CUDA hardware suite".to_owned())
    }
}

fn run_hardware_suite() -> TestResult {
    require_cuda_opt_in()?;
    if HARDWARE_CASES.is_empty() {
        return Err("E0 CUDA hardware suite registered zero cases".to_owned());
    }

    let mut executed = 0_usize;
    for case in HARDWARE_CASES {
        executed = executed.saturating_add(1);
        eprintln!("running E0 CUDA case: {}", case.name);
        (case.run)().map_err(|error| format!("E0 CUDA case {} failed: {error}", case.name))?;
    }
    if executed != HARDWARE_CASES.len() {
        return Err(format!(
            "E0 CUDA suite executed {executed} of {} registered cases",
            HARDWARE_CASES.len()
        ));
    }
    eprintln!("E0 CUDA suite passed {executed} cases");
    Ok(())
}

fn main() -> ExitCode {
    match run_hardware_suite() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("E0 CUDA hardware suite failed: {error}");
            ExitCode::FAILURE
        }
    }
}
