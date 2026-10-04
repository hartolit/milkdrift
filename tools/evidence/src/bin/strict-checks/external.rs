//! Local scanners see bounded repository content; their reports always redact suspected secrets.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
    process::Command,
};

use super::{CheckResult, Gate};

pub(super) fn versions(gate: &mut Gate) -> CheckResult {
    for (name, arguments, required) in [
        ("actionlint", vec!["-version"], "1.7.12"),
        ("zizmor", vec!["--version"], "1.30.1"),
        ("gitleaks", vec!["version"], "8.30.1"),
    ] {
        if gate.run(&format!("{name}-version"), name, &arguments)? {
            let result = gate.checks.last_mut().ok_or("version result missing")?;
            if !fs::read_to_string(&result.stdout)?
                .split_whitespace()
                .any(|word| word == required)
            {
                result.classification = "tool_version_mismatch";
                gate.save()?;
            }
        }
    }
    Ok(())
}

pub(super) fn checks(gate: &mut Gate, base: &str, all_tracked: bool) -> CheckResult {
    // Python shell snippets are not part of the backend. ShellCheck is deliberately not a hidden
    // optional dependency: this command promises workflow syntax/expressions, not shell analysis.
    gate.run(
        "actionlint",
        "actionlint",
        &[
            "-shellcheck=",
            "-pyflakes=",
            "-no-color",
            "-format",
            "{{json .}}",
        ],
    )?;
    gate.run(
        "zizmor",
        "zizmor",
        &[
            "--offline",
            "--no-config",
            "--no-ignores",
            "--strict-collection",
            "--no-progress",
            "--format",
            "json",
            ".github/workflows",
        ],
    )?;
    let snapshot = tempfile::tempdir()?;
    copy_scan_inputs(&gate.root, snapshot.path(), base, all_tracked)?;
    let snapshot = path_text(snapshot.path())?;
    let report = gate.output.join("secrets.redacted.json");
    let report = path_text(&report)?;
    gate.run(
        "gitleaks",
        "gitleaks",
        &[
            "dir",
            "--redact=100",
            "--no-banner",
            "--no-color",
            "--ignore-gitleaks-allow",
            "--gitleaks-ignore-path",
            snapshot,
            "--timeout",
            "300",
            "--report-format",
            "json",
            "--report-path",
            report,
            snapshot,
        ],
    )?;
    Ok(())
}

fn path_text(path: &Path) -> CheckResult<&str> {
    path.to_str()
        .ok_or_else(|| "scanner paths must be valid UTF-8".into())
}

fn git_paths(root: &Path, arguments: &[&str]) -> CheckResult<Vec<std::path::PathBuf>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "secret scan input discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let mut paths = Vec::new();
    for value in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
    {
        let path = Path::new(std::str::from_utf8(value)?);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("non-repository secret scan path".into());
        }
        paths.push(path.to_owned());
    }
    Ok(paths)
}

fn copy_scan_inputs(root: &Path, destination: &Path, base: &str, all_tracked: bool) -> CheckResult {
    let mut files: BTreeSet<_> = if all_tracked {
        git_paths(root, &["ls-files", "-z", "--cached"])?
    } else {
        // Resolve first, so malformed/unavailable event bases cannot become an empty green scan.
        let output = Command::new("git").current_dir(root).args(["rev-parse", "--verify", "--end-of-options", &format!("{base}^{{commit}}")]).output()?;
        if !output.status.success() { return Err("secret scan base is not an available commit; fetch the event base or explicitly run --all-tracked".into()); }
        let commit = String::from_utf8(output.stdout)?;
        git_paths(root, &["diff", "--name-only", "-z", commit.trim(), "--"])?
    }.into_iter().collect();
    files.extend(git_paths(
        root,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )?);
    for relative in files {
        let source = root.join(&relative);
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue, // Deleted content is absent in this changed-content scan.
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() {
            return Err(format!(
                "unsupported scan input {} (symlink/submodule requires explicit review)",
                relative.display()
            )
            .into());
        }
        let target = destination.join(relative);
        fs::create_dir_all(target.parent().ok_or("scan path has no parent")?)?;
        fs::copy(source, target)?;
    }
    // No inherited ignore file, source annotation, or ambient config may exempt changed content.
    for forbidden in [".gitleaksignore", ".gitleaks.toml"] {
        if destination.join(forbidden).exists() {
            return Err(
                format!("{forbidden} needs a reviewed exact exception policy before use").into(),
            );
        }
    }
    Ok(())
}

fn expect_refusal(
    gate: &mut Gate,
    name: &str,
    program: &str,
    arguments: &[&str],
    expected_text: &str,
) -> CheckResult {
    if gate.run(name, program, arguments)? {
        return Err(format!("{name} accepted the invalid specimen").into());
    }
    let result = gate.checks.last_mut().ok_or("missing probe result")?;
    if result.exit_code.is_none() {
        return Err(format!("{name} did not start").into());
    }
    let output = format!(
        "{}{}",
        fs::read_to_string(&result.stdout)?,
        fs::read_to_string(&result.stderr)?
    );
    if !output.contains(expected_text) {
        return Err(
            format!("{name} failed for an unexpected reason; inspect its redacted logs").into(),
        );
    }
    result.classification = "expected_probe_refusal";
    gate.save()
}

pub(super) fn probes(gate: &mut Gate) -> CheckResult {
    let directory = tempfile::tempdir()?;
    let workflow = directory.path().join("workflow.yml");
    let good = "name: probe\non: pull_request\npermissions:\n  contents: read\njobs:\n  probe:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo safe\n";
    fs::write(&workflow, good)?;
    let workflow_path = path_text(&workflow)?;
    if !gate.run(
        "actionlint-valid",
        "actionlint",
        &["-shellcheck=", "-pyflakes=", workflow_path],
    )? {
        return Err("actionlint valid specimen failed".into());
    }
    fs::write(
        &workflow,
        good.replace("echo safe", "echo ${{ steps.missing.outputs.value }}"),
    )?;
    expect_refusal(
        gate,
        "actionlint-invalid",
        "actionlint",
        &["-shellcheck=", "-pyflakes=", workflow_path],
        "missing",
    )?;
    fs::write(&workflow, good)?;
    if !gate.run(
        "zizmor-valid",
        "zizmor",
        &[
            "--offline",
            "--no-config",
            "--strict-collection",
            "--no-progress",
            workflow_path,
        ],
    )? {
        return Err("zizmor valid specimen failed".into());
    }
    fs::write(
        &workflow,
        good.replace("echo safe", "echo ${{ github.event.pull_request.title }}"),
    )?;
    expect_refusal(
        gate,
        "zizmor-invalid",
        "zizmor",
        &[
            "--offline",
            "--no-config",
            "--strict-collection",
            "--no-progress",
            workflow_path,
        ],
        "template-injection",
    )?;
    fs::write(&workflow, good)?;
    let candidate = directory.path().join("credential.txt");
    fs::write(&candidate, "public fixture without credentials\n")?;
    let directory_path = path_text(directory.path())?;
    let arguments = [
        "dir",
        "--redact=100",
        "--no-banner",
        "--ignore-gitleaks-allow",
        "--timeout",
        "30",
        directory_path,
    ];
    if !gate.run("gitleaks-valid", "gitleaks", &arguments)? {
        return Err("gitleaks valid specimen failed".into());
    }
    // Deliberately synthetic, assembled here so the test itself is not an ignored secret fixture.
    fs::write(
        &candidate,
        format!(
            "token = '{}{}'\n",
            "ghp_", "Q7m2Z9a4L8r6T1v3B5n0C2x8P4d9F7h6J1k3"
        ),
    )?;
    expect_refusal(
        gate,
        "gitleaks-invalid",
        "gitleaks",
        &arguments,
        "leaks found",
    )?;
    Ok(())
}
