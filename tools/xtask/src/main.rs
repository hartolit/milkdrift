//! Milkdrift workspace maintenance entry point and canonical composite gate.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

use xtask::{
    CargoCommand, benchmark_command_plan, cuda_clippy_command_plan, cuda_compile_command_plan,
    cuda_hardware_command_plan, portable_command_plan, validate_repository_hygiene,
    validate_workspace,
};

const HELP: &str = "\
Milkdrift workspace tooling

USAGE:
    cargo xtask <command>
    cargo xtask portable <wasm32-unknown-unknown|thumbv7em-none-eabihf>

COMMANDS:
    architecture    Validate workspace roles, layout, dependency DAG, and feature policy
    hygiene         Validate operational surfaces and the selected dependency graph
    verify          Run policy, format, build, test, lint, docs, and exact benchmark gates
    portable        Check every metadata-owned domain library for one portable target
    cuda-compile    Check CUDA owners and compile CUDA tests and exact hardware suites
    cuda-clippy     Lint CUDA owners and exact hardware suites with warnings denied
    cuda-hardware   Run every exact harness-free CUDA hardware suite in release mode
    help            Print this message

Canonical matrices are derived from locked Cargo metadata; ordinary Cargo operations remain direct.
";

fn main() -> ExitCode {
    match execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> io::Result<ExitCode> {
    let mut arguments = env::args_os().skip(1);
    let command_argument = arguments.next().unwrap_or_else(|| OsString::from("help"));
    let remaining = arguments.collect::<Vec<_>>();
    let Some(command) = command_argument.to_str() else {
        eprintln!("xtask commands must be valid UTF-8");
        return Ok(ExitCode::from(2));
    };

    let success = match command {
        "help" | "--help" | "-h" => {
            if !remaining.is_empty() {
                return Ok(argument_error("help does not accept arguments"));
            }
            print!("{HELP}");
            return Ok(ExitCode::SUCCESS);
        }
        "architecture" => {
            if !remaining.is_empty() {
                return Ok(argument_error("architecture does not accept arguments"));
            }
            validate_architecture()
        }
        "hygiene" => {
            if !remaining.is_empty() {
                return Ok(argument_error("hygiene does not accept arguments"));
            }
            validate_hygiene()
        }
        "verify" => {
            if !remaining.is_empty() {
                return Ok(argument_error("verify does not accept arguments"));
            }
            verify()
        }
        "portable" => {
            let [target] = remaining.as_slice() else {
                return Ok(argument_error(
                    "portable requires exactly one maintained target",
                ));
            };
            let Some(target) = target.to_str() else {
                return Ok(argument_error("portable target must be valid UTF-8"));
            };
            let commands =
                portable_command_plan(&workspace_manifest(), target).map_err(io::Error::other)?;
            run_cargo_plan(&commands)
        }
        "cuda-compile" => {
            if !remaining.is_empty() {
                return Ok(argument_error("cuda-compile does not accept arguments"));
            }
            let commands =
                cuda_compile_command_plan(&workspace_manifest()).map_err(io::Error::other)?;
            run_cargo_plan(&commands)
        }
        "cuda-clippy" => {
            if !remaining.is_empty() {
                return Ok(argument_error("cuda-clippy does not accept arguments"));
            }
            let commands =
                cuda_clippy_command_plan(&workspace_manifest()).map_err(io::Error::other)?;
            run_cargo_plan(&commands)
        }
        "cuda-hardware" => {
            if !remaining.is_empty() {
                return Ok(argument_error("cuda-hardware does not accept arguments"));
            }
            let commands =
                cuda_hardware_command_plan(&workspace_manifest()).map_err(io::Error::other)?;
            run_cargo_plan(&commands)
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

fn argument_error(message: &str) -> ExitCode {
    eprintln!("{message}\n");
    print!("{HELP}");
    ExitCode::from(2)
}

fn verify() -> io::Result<bool> {
    if !validate_architecture()? || !validate_hygiene()? {
        return Ok(false);
    }

    let commands = [
        command(&["fmt", "--all", "--", "--check"]),
        command(&["check", "--workspace", "--all-targets", "--locked"]),
        command(&["test", "--workspace", "--locked"]),
        command(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        command(&["doc", "--workspace", "--no-deps", "--locked"]),
    ];
    if !run_sequence(&commands)? {
        return Ok(false);
    }

    let benchmark_commands =
        benchmark_command_plan(&workspace_manifest()).map_err(io::Error::other)?;
    for benchmark in benchmark_commands {
        if !run_cargo(benchmark.arguments())? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn command(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect()
}

fn run_sequence(commands: &[Vec<String>]) -> io::Result<bool> {
    for arguments in commands {
        if !run_cargo(arguments)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn run_cargo_plan(commands: &[CargoCommand]) -> io::Result<bool> {
    for command in commands {
        if !run_cargo(command.arguments())? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn run_cargo(arguments: &[String]) -> io::Result<bool> {
    let Some((subcommand, options)) = arguments.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cargo command requires a subcommand",
        ));
    };
    let manifest = workspace_manifest();
    let Some(workspace_root) = manifest.parent() else {
        return Err(io::Error::other(
            "workspace manifest has no parent directory",
        ));
    };
    println!("+ cargo {}", arguments.join(" "));

    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut process = Command::new(cargo);
    process
        .current_dir(workspace_root)
        .arg(subcommand)
        .args(options);
    if subcommand == "doc" {
        process.env("RUSTDOCFLAGS", "-D warnings");
    }
    let status = process.status()?;
    let success = status.success();

    report_status(arguments, status);
    Ok(success)
}

fn report_status(arguments: &[String], status: ExitStatus) {
    if status.success() {
        return;
    }

    let rendered = arguments.join(" ");
    match status.code() {
        Some(code) => eprintln!("cargo {rendered} exited with status {code}"),
        None => eprintln!("cargo {rendered} terminated without an exit code"),
    }
}

fn validate_architecture() -> io::Result<bool> {
    let manifest = workspace_manifest();
    let report = validate_workspace(&manifest).map_err(io::Error::other)?;

    for violation in report.violations() {
        eprintln!("{violation}");
    }
    if report.is_valid() {
        println!("workspace architecture and dependency policy are valid");
    }

    Ok(report.is_valid())
}

fn validate_hygiene() -> io::Result<bool> {
    let manifest = workspace_manifest();
    let report = validate_repository_hygiene(&manifest).map_err(io::Error::other)?;

    for violation in report.violations() {
        eprintln!("{violation}");
    }
    if report.is_valid() {
        println!("repository hygiene policy is valid");
    }

    Ok(report.is_valid())
}

fn workspace_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Cargo.toml")
}
