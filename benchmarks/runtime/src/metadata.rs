//! Reproducibility metadata collected with fixed, non-secret subprocess calls.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::memory::{cpu_information, total_memory_bytes};
use crate::report::{GitMetadata, SystemMetadata, ToolchainMetadata};

const CRITERION_VERSION: &str = "0.8.2";
const THREAD_ENVIRONMENT_VARIABLES: [&str; 10] = [
    "CARGO_BUILD_JOBS",
    "RAYON_NUM_THREADS",
    "OMP_NUM_THREADS",
    "OMP_THREAD_LIMIT",
    "MKL_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "VECLIB_MAXIMUM_THREADS",
    "NUMEXPR_NUM_THREADS",
    "RUST_TEST_THREADS",
    "CANDLE_NUM_THREADS",
];

pub(crate) struct EnvironmentMetadata {
    pub(crate) git: GitMetadata,
    pub(crate) toolchain: ToolchainMetadata,
    pub(crate) system: SystemMetadata,
}

pub(crate) fn collect(repository_root: &Path) -> BenchmarkResult<EnvironmentMetadata> {
    let rustc_verbose = command_stdout("rustc", &["--version", "--verbose"], None)?;
    let rust_version = first_nonempty_line(&rustc_verbose, "rustc version output")?.to_owned();
    let rustc_host = parse_keyed_line(&rustc_verbose, "host")
        .ok_or_else(|| BenchmarkError::new("rustc --version --verbose did not report host"))?;
    let llvm_version = parse_keyed_line(&rustc_verbose, "LLVM version");
    let target_triple = match option_env!("TARGET") {
        Some(target) => target.to_owned(),
        None => rustc_host,
    };
    let cargo_version = command_stdout("cargo", &["--version"], None)?;
    let kernel = command_stdout("uname", &["-sr"], None)?;
    let (cpu_model, physical_cpu_count) = cpu_information()?;
    let logical_cpu_count = std::thread::available_parallelism()
        .ok()
        .map(std::num::NonZeroUsize::get);

    Ok(EnvironmentMetadata {
        git: GitMetadata {
            head: command_stdout("git", &["rev-parse", "HEAD"], Some(repository_root))?,
            head_tree: command_stdout("git", &["rev-parse", "HEAD^{tree}"], Some(repository_root))?,
            dirty: git_is_dirty(repository_root)?,
        },
        toolchain: ToolchainMetadata {
            rust_version,
            cargo_version,
            llvm_version,
            criterion_version: CRITERION_VERSION,
            target_triple,
            build_profile: build_profile(),
            enabled_features: Vec::new(),
        },
        system: SystemMetadata {
            os: std::env::consts::OS,
            kernel,
            cpu_model,
            physical_cpu_count,
            logical_cpu_count,
            total_memory_bytes: total_memory_bytes()?,
            thread_environment: thread_environment(),
        },
    })
}

fn git_is_dirty(repository_root: &Path) -> BenchmarkResult<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repository_root)
        .output()
        .map_err(|error| BenchmarkError::new(format!("could not execute git status: {error}")))?;
    if !output.status.success() {
        return Err(BenchmarkError::new(format!(
            "git status failed with exit status {}",
            output.status
        )));
    }
    Ok(!output.stdout.is_empty())
}

fn command_stdout(
    program: &str,
    arguments: &[&str],
    current_directory: Option<&Path>,
) -> BenchmarkResult<String> {
    let mut command = Command::new(program);
    command.args(arguments);
    if let Some(directory) = current_directory {
        command.current_dir(directory);
    }
    let output = command
        .output()
        .map_err(|error| BenchmarkError::new(format!("could not execute {program}: {error}")))?;
    if !output.status.success() {
        return Err(BenchmarkError::new(format!(
            "{program} failed with exit status {}",
            output.status
        )));
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        BenchmarkError::new(format!("{program} emitted non-UTF-8 stdout: {error}"))
    })?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(BenchmarkError::new(format!(
            "{program} emitted empty stdout"
        )));
    }
    Ok(trimmed.to_owned())
}

fn first_nonempty_line<'a>(input: &'a str, label: &str) -> BenchmarkResult<&'a str> {
    input
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| BenchmarkError::new(format!("{label} was empty")))
}

fn parse_keyed_line(input: &str, key: &str) -> Option<String> {
    input.lines().find_map(|line| {
        let (observed_key, value) = line.split_once(':')?;
        if observed_key.trim() == key {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        } else {
            None
        }
    })
}

fn thread_environment() -> BTreeMap<String, Option<String>> {
    THREAD_ENVIRONMENT_VARIABLES
        .into_iter()
        .map(|name| (name.to_owned(), std::env::var(name).ok()))
        .collect()
}

const fn build_profile() -> &'static str {
    match option_env!("PROFILE") {
        Some(profile) => profile,
        None if cfg!(debug_assertions) => "debug",
        None => "release",
    }
}

#[cfg(test)]
mod tests {
    use super::{first_nonempty_line, parse_keyed_line};

    #[test]
    fn rustc_verbose_fields_are_parsed_without_shells() -> Result<(), String> {
        let input = "rustc 1.96.1 (example)\nbinary: rustc\nhost: x86_64-unknown-linux-gnu\nrelease: 1.96.1\n";
        assert_eq!(
            first_nonempty_line(input, "rustc").map_err(|error| error.to_string())?,
            "rustc 1.96.1 (example)"
        );
        assert_eq!(
            parse_keyed_line(input, "host").as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
        assert!(parse_keyed_line(input, "missing").is_none());
        assert!(parse_keyed_line(input, "LLVM version").is_none());
        Ok(())
    }
}
