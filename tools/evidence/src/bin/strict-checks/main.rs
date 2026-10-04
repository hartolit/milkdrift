//! Run the finite static gate and retain every exit status, including blocked compiler coverage.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::Instant,
};

use clap::Parser;
use serde::Serialize;

mod external;

type CheckResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
struct Arguments {
    /// Run only workflow and redacted secret checks (also usable before compiling the product).
    #[arg(long)]
    external_only: bool,
    /// Prove the installed workflow and secret tools reject synthetic violations.
    #[arg(long)]
    probe_tools: bool,
    /// Initial content audit; routine runs scan changed files relative to --secret-base.
    #[arg(long)]
    all_tracked: bool,
    /// Local or CI change base. Must resolve to an existing commit; missing history is a failure.
    #[arg(long, default_value = "HEAD^")]
    secret_base: String,
    /// New evidence directory. Existing reports are not overwritten.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Serialize)]
struct Check {
    name: String,
    program: String,
    arguments: Vec<String>,
    exit_code: Option<i32>,
    seconds: f64,
    classification: &'static str,
    stdout: String,
    stderr: String,
}

struct Gate {
    root: PathBuf,
    output: PathBuf,
    toolchain: String,
    checks: Vec<Check>,
}

impl Gate {
    fn passed(&self) -> bool {
        self.checks
            .iter()
            .all(|check| matches!(check.classification, "passed" | "expected_probe_refusal"))
    }
    #[expect(
        clippy::print_stdout,
        reason = "The static gate reports only checker names and outcomes to its operator"
    )]
    fn run(&mut self, name: &str, program: &str, arguments: &[&str]) -> CheckResult<bool> {
        let stdout = self.output.join(format!("{name}.stdout"));
        let stderr = self.output.join(format!("{name}.stderr"));
        let mut command = Command::new(program);
        command
            .current_dir(&self.root)
            .args(arguments)
            .env("CARGO_TERM_COLOR", "never")
            // Directly launched evidence binaries do not inherit rustup's selection. Without
            // this, dependency working directories can select the user's unrelated default.
            .env("RUSTUP_TOOLCHAIN", &self.toolchain)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("CLIPPY_CONF_DIR")
            .env_remove("GITLEAKS_CONFIG")
            .env_remove("GITLEAKS_CONFIG_TOML")
            .env_remove("GITLEAKS_ENABLE_RULE")
            .stdout(Stdio::from(File::create(&stdout)?))
            .stderr(Stdio::from(File::create(&stderr)?));
        if name.starts_with("doc-") {
            command.env("RUSTDOCFLAGS", "-D warnings");
        }
        let started = Instant::now();
        let result = command.status();
        let (exit_code, classification) = match result {
            Ok(status) if status.success() => (status.code(), "passed"),
            Ok(status) => (status.code(), classify_failure(&stdout, &stderr)?),
            Err(error) => {
                fs::write(
                    &stderr,
                    format!("required checker {program} could not start: {error}\n"),
                )?;
                (None, "tool_failure")
            }
        };
        let passed = classification == "passed";
        self.checks.push(Check {
            name: name.to_owned(),
            program: program.to_owned(),
            arguments: arguments.iter().map(|s| (*s).to_owned()).collect(),
            exit_code,
            seconds: started.elapsed().as_secs_f64(),
            classification,
            stdout: stdout.to_string_lossy().into_owned(),
            stderr: stderr.to_string_lossy().into_owned(),
        });
        self.save()?;
        println!("{name}: {classification}");
        Ok(passed)
    }

    fn save(&self) -> CheckResult {
        fs::write(
            self.output.join("checks.json"),
            serde_json::to_vec_pretty(&self.checks)?,
        )?;
        Ok(())
    }

    fn cargo(&mut self, name: &str, arguments: &[&str]) -> CheckResult {
        self.run(name, "cargo", arguments)?;
        Ok(())
    }
}

fn classify_failure(stdout: &Path, stderr: &Path) -> CheckResult<&'static str> {
    let mut lint = false;
    let mut compiler = false;
    for line in fs::read_to_string(stdout)?.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
            continue;
        }
        let message = value
            .get("message")
            .ok_or("compiler diagnostic has no message")?;
        if message.get("level").and_then(|v| v.as_str()) != Some("error") {
            continue;
        }
        match message.pointer("/code/code").and_then(|v| v.as_str()) {
            Some(code) if code.starts_with('E') => compiler = true,
            Some("unknown_lints" | "renamed_and_removed_lints") => {
                return Ok("configuration_failure");
            }
            Some(_) => lint = true,
            None => compiler = true,
        }
    }
    let error = fs::read_to_string(stderr)?;
    if error.contains("error reading Clippy's configuration") || error.contains("unknown field") {
        return Ok("configuration_failure");
    }
    if compiler {
        Ok("compilation_failure")
    } else if lint {
        Ok("policy_violation_incomplete_coverage")
    } else {
        Ok("failed_check_requires_review")
    }
}

fn static_checks(gate: &mut Gate) -> CheckResult {
    gate.cargo("fmt", &["fmt", "--all", "--", "--check"])?;
    for (name, command) in [("check", "check"), ("clippy", "clippy")] {
        for (scope, flags) in [
            (
                "product-default",
                vec!["-p", "milkdrift-daemon", "-p", "milkdrift-cli", "--bins"],
            ),
            ("workspace-default", vec!["--workspace", "--all-targets"]),
            (
                "workspace-all",
                vec!["--workspace", "--all-targets", "--all-features"],
            ),
        ] {
            let mut arguments = vec![command, "--locked", "--message-format=json"];
            arguments.extend(flags);
            if command == "clippy" {
                arguments.extend(["--", "-D", "warnings"]);
            }
            gate.cargo(&format!("{name}-{scope}"), &arguments)?;
        }
    }
    // These defaults must remain visible outside the all-feature evidence graph.
    for package in [
        "milkdrift-runtime",
        "milkdrift-capability-host",
        "milkdrift-redb-store",
        "milkdrift-model-provider",
    ] {
        gate.cargo(
            &format!("clippy-default-{package}"),
            &[
                "clippy",
                "--locked",
                "-p",
                package,
                "--lib",
                "--message-format=json",
                "--",
                "-D",
                "warnings",
            ],
        )?;
        gate.cargo(
            &format!("doc-default-{package}"),
            &[
                "doc",
                "--locked",
                "-p",
                package,
                "--no-deps",
                "--message-format=json",
            ],
        )?;
    }
    gate.cargo(
        "doc-workspace-all",
        &[
            "doc",
            "--locked",
            "--workspace",
            "--all-features",
            "--no-deps",
            "--message-format=json",
        ],
    )?;
    gate.cargo(
        "repository-contracts",
        &[
            "test",
            "--locked",
            "-p",
            "milkdrift-evidence",
            "--test",
            "repository_contracts",
            "--all-features",
            "--message-format=json",
        ],
    )?;
    gate.cargo("deny", &["deny", "check"])?;
    gate.cargo("machete", &["machete"])?;
    gate.cargo(
        "duplicates",
        &["tree", "--locked", "--workspace", "--duplicates"],
    )?;
    Ok(())
}

fn summarize(gate: &Gate) -> CheckResult {
    let mut findings: BTreeMap<(String, String, u64, String), std::collections::BTreeSet<String>> =
        BTreeMap::new();
    for check in &gate.checks {
        for line in fs::read_to_string(&check.stdout)?.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if value.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
                continue;
            }
            let message = value
                .get("message")
                .ok_or("compiler diagnostic has no message")?;
            if !matches!(
                message.get("level").and_then(|v| v.as_str()),
                Some("error" | "warning")
            ) {
                continue;
            }
            let span = message
                .get("spans")
                .and_then(|v| v.as_array())
                .and_then(|spans| {
                    spans
                        .iter()
                        .find(|s| s.get("is_primary").and_then(|v| v.as_bool()) == Some(true))
                });
            let Some(span) = span else {
                continue;
            };
            let key = (
                message
                    .pointer("/code/code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unclassified")
                    .to_owned(),
                span.get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned(),
                span.get("line_start").and_then(|v| v.as_u64()).unwrap_or(0),
                message
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned(),
            );
            findings.entry(key).or_default().insert(check.name.clone());
        }
    }
    let findings: Vec<_> = findings.into_iter().map(|((rule, file, line, message), coverage)| {
        let owner = file.split('/').take(2).collect::<Vec<_>>().join("/");
        serde_json::json!({"rule":rule,"owner":owner,"file":file,"line":line,"message":message,"coverage":coverage})
    }).collect();
    fs::write(
        gate.output.join("findings.json"),
        serde_json::to_vec_pretty(&findings)?,
    )?;
    Ok(())
}

fn fingerprint_diff(root: &Path, output: &Path) -> CheckResult {
    // Keep exact-byte provenance without copying a potentially secret-bearing diff into
    // uploaded diagnostics. The commit and changed paths are captured by ordinary commands.
    let diff = Command::new("git")
        .current_dir(root)
        .args(["diff", "--binary", "HEAD", "--", "."])
        .output()?;
    if !diff.status.success() {
        return Err("source diff fingerprint failed".into());
    }
    fs::write(
        output.join("source-diff.blake3"),
        format!("{}\n", blake3::hash(&diff.stdout)),
    )?;
    Ok(())
}

fn selected_toolchain(root: &Path) -> CheckResult<String> {
    let configuration: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("rust-toolchain.toml"))?)?;
    Ok(configuration
        .get("toolchain")
        .and_then(|value| value.get("channel"))
        .and_then(toml::Value::as_str)
        .ok_or("pinned toolchain channel is missing")?
        .to_owned())
}

fn execute(arguments: Arguments) -> CheckResult<bool> {
    let root = std::env::current_dir()?;
    if !root.join("rust-toolchain.toml").is_file() {
        return Err("run strict-checks from the repository root".into());
    }
    if arguments.output.exists() {
        return Err("choose a new output directory to retain prior evidence".into());
    }
    fs::create_dir_all(&arguments.output)?;
    let output = fs::canonicalize(&arguments.output)?;
    let toolchain = selected_toolchain(&root)?;
    let mut gate = Gate {
        root,
        output,
        toolchain,
        checks: Vec::new(),
    };
    gate.run("source-head", "git", &["rev-parse", "HEAD"])?;
    gate.run("source-status", "git", &["status", "--short"])?;
    gate.run("source-diff", "git", &["diff", "--stat", "HEAD", "--", "."])?;
    fingerprint_diff(&gate.root, &gate.output)?;
    gate.run("rust-version", "rustc", &["-Vv"])?;
    gate.cargo("clippy-version", &["clippy", "--version"])?;
    gate.cargo("deny-version", &["deny", "--version"])?;
    gate.cargo("machete-version", &["machete", "--version"])?;
    external::versions(&mut gate)?;
    if arguments.probe_tools {
        external::probes(&mut gate)?;
    } else {
        if !arguments.external_only {
            static_checks(&mut gate)?;
        }
        external::checks(&mut gate, &arguments.secret_base, arguments.all_tracked)?;
    }
    summarize(&gate)?;
    Ok(gate.passed())
}

#[expect(
    clippy::print_stderr,
    reason = "The static command reports its startup or verification failure to the operator"
)]
fn main() -> ExitCode {
    match execute(Arguments::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("strict checks failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckResult, Gate};

    #[test]
    fn runner_never_accepts_missing_tools_or_nonzero_status() -> CheckResult {
        let output = tempfile::tempdir()?;
        let mut gate = Gate {
            root: std::env::current_dir()?,
            output: output.path().to_owned(),
            toolchain: super::selected_toolchain(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
            )?,
            checks: Vec::new(),
        };
        assert!(!gate.run("missing", "milkdrift-deliberately-missing-checker", &[])?);
        assert_eq!(
            gate.checks.last().ok_or("missing result")?.classification,
            "tool_failure"
        );
        assert!(!gate.run("bad-command", "rustc", &["--milkdrift-invalid-option"])?);
        assert!(gate.run("valid-command", "rustc", &["--version"])?);
        assert!(
            !gate.passed(),
            "a later success must not erase failed checks"
        );
        assert!(output.path().join("checks.json").is_file());
        Ok(())
    }

    #[test]
    fn source_provenance_keeps_a_digest_without_copying_diff_secrets() -> CheckResult {
        let repository = tempfile::tempdir()?;
        let evidence = tempfile::tempdir()?;
        for arguments in [
            vec!["init", "--quiet"],
            vec![
                "-c",
                "user.name=Checker fixture",
                "-c",
                "user.email=checker@example.invalid",
                "commit",
                "--allow-empty",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .current_dir(repository.path())
                    .args(arguments)
                    .status()?
                    .success()
            );
        }
        std::fs::write(
            repository.path().join("input.txt"),
            "private-sentinel-value",
        )?;
        assert!(
            std::process::Command::new("git")
                .current_dir(repository.path())
                .args(["add", "input.txt"])
                .status()?
                .success()
        );
        super::fingerprint_diff(repository.path(), evidence.path())?;
        let first = std::fs::read_to_string(evidence.path().join("source-diff.blake3"))?;
        assert!(!first.contains("private-sentinel-value"));
        assert_eq!(first.trim().len(), 64);
        std::fs::write(
            repository.path().join("input.txt"),
            "changed-private-sentinel",
        )?;
        super::fingerprint_diff(repository.path(), evidence.path())?;
        assert_ne!(
            first,
            std::fs::read_to_string(evidence.path().join("source-diff.blake3"))?
        );
        Ok(())
    }
}
