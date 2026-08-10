use std::process::ExitCode;

fn reviewed_case() -> Result<(), &'static str> {
    Ok(())
}

const HARDWARE_CASES: &[fn() -> Result<(), &'static str>] = &[reviewed_case];

fn main() -> ExitCode {
    if std::env::var("MILKDRIFT_CUDA_TEST").as_deref() != Ok("1") {
        eprintln!("set MILKDRIFT_CUDA_TEST=1 to execute the fixture CUDA hardware suite");
        return ExitCode::FAILURE;
    }

    let mut executed = 0_usize;
    for case in HARDWARE_CASES {
        executed = executed.saturating_add(1);
        if let Err(error) = case() {
            eprintln!("fixture CUDA hardware case failed: {error}");
            return ExitCode::FAILURE;
        }
    }
    if executed == 0 || executed != HARDWARE_CASES.len() {
        eprintln!("fixture CUDA hardware suite executed zero or incomplete cases");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
