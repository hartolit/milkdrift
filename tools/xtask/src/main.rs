//! Milkdrift workspace maintenance entry point and canonical composite gate.

#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus, Stdio};

use xtask::{
    CargoCommand, VerificationComponent, VerificationOperation, VerificationPlan,
    cuda_clippy_command_plan, cuda_compile_command_plan, hardware_profile_command_plan,
    native_verification_plan, portable_command_plan, validate_repository_hygiene,
    validate_workspace, verification_component_plan,
};

const HELP: &str = "\
Milkdrift workspace tooling

USAGE:
    cargo xtask <command>
    cargo xtask verify-component <structure|check|test|clippy|docs|benches|nursery>
    cargo xtask portable <wasm32-unknown-unknown|thumbv7em-none-eabihf>
    cargo xtask hardware <PROFILE>

COMMANDS:
    architecture    Validate workspace roles, layout, dependency DAG, and feature policy
    hygiene         Validate operational surfaces and the selected dependency graph
    verify          Run policy, format, build, test, lint, docs, and exact benchmark gates
    verify-component
                    Run one metadata-owned native component; nursery is exploratory
    portable        Check every metadata-owned domain library for one portable target
    cuda-compile    Check CUDA owners and compile CUDA tests and exact hardware suites
    cuda-clippy     Lint CUDA owners and exact hardware suites with warnings denied
    hardware        Run every Cargo-metadata suite in a declared hardware profile
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
        "help" | "--help" | "-h" => return Ok(help(&remaining)),
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
        "verify-component" => {
            let [component] = remaining.as_slice() else {
                return Ok(argument_error(
                    "verify-component requires exactly one component name",
                ));
            };
            let Some(component) = component.to_str() else {
                return Ok(argument_error("verification component must be valid UTF-8"));
            };
            let Some(component) = VerificationComponent::parse(component) else {
                return Ok(argument_error("unknown verification component"));
            };
            let plan = verification_component_plan(&workspace_manifest(), component)
                .map_err(io::Error::other)?;
            run_verification_plan(&plan)
        }
        "portable" => {
            let target = match exact_utf8_argument(&remaining, "portable", "maintained target") {
                Ok(target) => target,
                Err(message) => return Ok(argument_error(&message)),
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
        "hardware" => {
            let profile = match exact_utf8_argument(&remaining, "hardware", "declared profile") {
                Ok(profile) => profile,
                Err(message) => return Ok(argument_error(&message)),
            };
            let commands = hardware_profile_command_plan(&workspace_manifest(), profile)
                .map_err(io::Error::other)?;
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

fn help(arguments: &[OsString]) -> ExitCode {
    if !arguments.is_empty() {
        return argument_error("help does not accept arguments");
    }
    print!("{HELP}");
    ExitCode::SUCCESS
}

fn exact_utf8_argument<'a>(
    arguments: &'a [OsString],
    command: &str,
    argument: &str,
) -> Result<&'a str, String> {
    let [value] = arguments else {
        return Err(format!("{command} requires exactly one {argument} name"));
    };
    value
        .to_str()
        .ok_or_else(|| format!("{command} {argument} must be valid UTF-8"))
}

fn argument_error(message: &str) -> ExitCode {
    eprintln!("{message}\n");
    print!("{HELP}");
    ExitCode::from(2)
}

fn verify() -> io::Result<bool> {
    let plans = native_verification_plan(&workspace_manifest()).map_err(io::Error::other)?;
    for plan in plans {
        if !run_verification_plan(&plan)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn run_verification_plan(plan: &VerificationPlan) -> io::Result<bool> {
    println!("== verify component: {} ==", plan.component().as_str());
    for operation in plan.operations() {
        let success = match operation {
            VerificationOperation::Architecture => validate_architecture()?,
            VerificationOperation::Hygiene => validate_hygiene()?,
            VerificationOperation::Cargo(command) => run_cargo(command.arguments())?,
        };
        if !success {
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
    let suppress_stdout = subcommand == "metadata";
    if suppress_stdout {
        println!("+ cargo {} > /dev/null", arguments.join(" "));
    } else {
        println!("+ cargo {}", arguments.join(" "));
    }

    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut process = Command::new(cargo);
    process
        .current_dir(workspace_root)
        .arg(subcommand)
        .args(options);
    if suppress_stdout {
        process.stdout(Stdio::null());
    }
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
