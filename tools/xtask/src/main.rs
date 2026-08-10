//! Milkdrift workspace maintenance entry point and canonical composite gate.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

use xtask::{benchmark_command_plan, validate_repository_hygiene, validate_workspace};

const HELP: &str = "\
Milkdrift workspace tooling

USAGE:
    cargo xtask <command>

COMMANDS:
    architecture    Validate workspace roles, layout, dependency DAG, and feature policy
    hygiene         Validate operational surfaces and the selected dependency graph
    verify          Run policy, format, build, test, lint, docs, and exact benchmark gates
    help            Print this message

Ordinary Cargo operations are invoked directly rather than hidden behind forwarding commands.
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
    let command_argument = env::args_os()
        .nth(1)
        .unwrap_or_else(|| OsString::from("help"));
    let Some(command) = command_argument.to_str() else {
        eprintln!("xtask commands must be valid UTF-8");
        return Ok(ExitCode::from(2));
    };

    let success = match command {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            return Ok(ExitCode::SUCCESS);
        }
        "architecture" => validate_architecture(),
        "hygiene" => validate_hygiene(),
        "verify" => verify(),
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
