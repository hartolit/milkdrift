//! Native workspace runner for architecture validation and repository quality commands.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus};

use llm_app::{validate_repository_hygiene, validate_workspace};

const HELP: &str = "\
llm-app workspace runner

USAGE:
    cargo run --locked --bin llm-app -- <command>

COMMANDS:
    architecture    Validate workspace architecture and repository hygiene
    hygiene         Validate tracked tooling and the selected dependency graph
    benchmark       Measure the sampling hot path
    benchmark-check Compile workspace benchmarks without running them
    check           Compile every workspace target
    test            Run ordinary workspace tests (benchmarks excluded)
    doc             Build workspace API documentation (alias: docs)
    fmt             Format the workspace
    fmt-check       Verify formatting without modifying files
    clippy          Lint every workspace target
    verify          Run the canonical workspace quality gate
    help            Print this message
";

fn main() -> ExitCode {
    match execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("workspace runner failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> io::Result<ExitCode> {
    let command_argument = env::args_os()
        .nth(1)
        .unwrap_or_else(|| OsString::from("help"));
    let Some(command) = command_argument.to_str() else {
        eprintln!("workspace runner commands must be valid UTF-8");
        return Ok(ExitCode::from(2));
    };

    let success = match command {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            return Ok(ExitCode::SUCCESS);
        }
        "architecture" => validate_architecture_and_hygiene(),
        "hygiene" => validate_hygiene(),
        "benchmark" => run_cargo(&[
            "bench",
            "-p",
            "sampling",
            "--bench",
            "sampling_pipeline",
            "--locked",
        ]),
        "benchmark-check" => run_cargo(&["bench", "--workspace", "--no-run", "--locked"]),
        "check" => run_cargo(&["check", "--workspace", "--all-targets", "--locked"]),
        "test" => run_cargo(&["test", "--workspace", "--locked"]),
        "doc" | "docs" => run_cargo(&["doc", "--workspace", "--no-deps", "--locked"]),
        "fmt" => run_cargo(&["fmt", "--all"]),
        "fmt-check" => run_cargo(&["fmt", "--all", "--", "--check"]),
        "clippy" => run_cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        "verify" => {
            if !validate_architecture_and_hygiene()? {
                return Ok(ExitCode::FAILURE);
            }
            run_sequence(&[
                &["fmt", "--all", "--", "--check"],
                &["check", "--workspace", "--all-targets", "--locked"],
                &["test", "--workspace", "--locked"],
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--locked",
                    "--",
                    "-D",
                    "warnings",
                ],
                &["doc", "--workspace", "--no-deps", "--locked"],
                &["bench", "--workspace", "--no-run", "--locked"],
            ])
        }
        _ => {
            eprintln!("unknown command: {command}\n");
            print!("{HELP}");
            return Ok(ExitCode::from(2));
        }
    }?;

    Ok(if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn run_sequence(commands: &[&[&str]]) -> io::Result<bool> {
    for arguments in commands {
        if !run_cargo(arguments)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn run_cargo(arguments: &[&str]) -> io::Result<bool> {
    let Some((subcommand, options)) = arguments.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cargo command requires a subcommand",
        ));
    };
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .arg(subcommand)
        .arg("--manifest-path")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .args(options);
    if *subcommand == "doc" {
        command.env("RUSTDOCFLAGS", "-D warnings");
    }
    let status = command.status()?;
    let success = status.success();

    report_status(arguments, status);
    Ok(success)
}

fn report_status(arguments: &[&str], status: ExitStatus) {
    if status.success() {
        return;
    }

    let rendered = arguments.join(" ");
    match status.code() {
        Some(code) => eprintln!("cargo {rendered} exited with status {code}"),
        None => eprintln!("cargo {rendered} terminated without an exit code"),
    }
}

fn validate_architecture_and_hygiene() -> io::Result<bool> {
    let architecture_valid = validate_architecture()?;
    let hygiene_valid = validate_hygiene()?;
    Ok(architecture_valid && hygiene_valid)
}

fn validate_architecture() -> io::Result<bool> {
    let manifest = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let report = validate_workspace(manifest).map_err(io::Error::other)?;

    for violation in report.violations() {
        eprintln!("{violation}");
    }
    if report.is_valid() {
        println!("workspace architecture and dependency policy are valid");
    }

    Ok(report.is_valid())
}

fn validate_hygiene() -> io::Result<bool> {
    let manifest = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let report = validate_repository_hygiene(manifest).map_err(io::Error::other)?;

    for violation in report.violations() {
        eprintln!("{violation}");
    }
    if report.is_valid() {
        println!("repository hygiene policy is valid");
    }

    Ok(report.is_valid())
}
