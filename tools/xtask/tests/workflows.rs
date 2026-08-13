//! Static and integration coverage for maintained GitHub workflow resources.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(workspace_root().join(relative))?)
}

fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn replace_actions_expressions(input: &str) -> String {
    let mut remainder = input;
    let mut output = String::new();
    while let Some(start) = remainder.find("${{") {
        output.push_str(&remainder[..start]);
        let expression = &remainder[start + 3..];
        let Some(end) = expression.find("}}") else {
            output.push_str(&remainder[start..]);
            return output;
        };
        output.push_str("github_expression");
        remainder = &expression[end + 2..];
    }
    output.push_str(remainder);
    output
}

fn embedded_shell_bodies(yaml: &str) -> Vec<String> {
    let lines = yaml.lines().collect::<Vec<_>>();
    let mut bodies = Vec::new();
    let mut index = 0;
    while let Some(line) = lines.get(index).copied() {
        let trimmed = line.trim_start();
        let Some(run_value) = trimmed.strip_prefix("run:") else {
            index += 1;
            continue;
        };
        let run_value = run_value.trim_start();
        if run_value != "|" {
            bodies.push(replace_actions_expressions(run_value));
            index += 1;
            continue;
        }

        let run_indent = leading_spaces(line);
        let body_indent = run_indent + 2;
        index += 1;
        let mut body = String::new();
        while let Some(body_line) = lines.get(index).copied() {
            if !body_line.trim().is_empty() && leading_spaces(body_line) <= run_indent {
                break;
            }
            if body_line.len() >= body_indent {
                body.push_str(&body_line[body_indent..]);
            }
            body.push('\n');
            index += 1;
        }
        bodies.push(replace_actions_expressions(&body));
    }
    bodies
}

fn assert_shell_syntax(label: &str, body: &str) -> Result<(), Box<dyn Error>> {
    let output = Command::new("sh").args(["-n", "-c", body]).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "invalid shell in {label}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

fn prepare_thresholds(yaml: &str) -> Vec<u64> {
    yaml.lines()
        .filter_map(|line| line.trim().strip_prefix("minimum_free_kib: "))
        .filter_map(|value| value.parse().ok())
        .chain(yaml.lines().filter_map(|line| {
            let marker = "ci-resource.sh prepare ";
            let remainder = line.trim().split_once(marker)?.1;
            remainder.split_whitespace().next()?.parse().ok()
        }))
        .collect()
}

fn checkout_revisions(yaml: &str) -> Vec<&str> {
    yaml.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("uses: actions/checkout@")
                .and_then(|value| value.split_whitespace().next())
        })
        .collect()
}

#[test]
fn workflows_keep_heavy_profiles_on_fresh_standard_runners() -> Result<(), Box<dyn Error>> {
    let quality = read(".github/workflows/quality.yml")?;
    assert!(quality.contains("runs-on: ubuntu-24.04"));
    assert_eq!(quality.matches("if: ${{ always() }}").count(), 5);
    assert_eq!(quality.matches("continue-on-error: true").count(), 1);
    assert!(quality.contains("CARGO_INCREMENTAL: \"0\""));
    assert!(!quality.contains("cargo xtask verify\n"));
    assert!(!quality.contains("cargo bench --workspace"));
    assert!(!quality.contains("14680064"));
    assert!(!quality.contains("12582912"));

    let expected_components = ["structure", "check", "test", "clippy", "docs", "benches"];
    for component in expected_components {
        assert_eq!(
            quality
                .matches(&format!("component: {component}\n"))
                .count(),
            1
        );
    }
    for threshold in prepare_thresholds(&quality) {
        assert!(
            threshold <= 9 * 1024 * 1024,
            "standard-hosted preflight is too close to the 14 GB runner total: {threshold} KiB"
        );
    }
    Ok(())
}

#[test]
fn workflow_targets_are_unique_and_cleanup_is_centralized() -> Result<(), Box<dyn Error>> {
    let quality = read(".github/workflows/quality.yml")?;
    let cuda = read(".github/workflows/cuda-hardware.yml")?;
    let targets = [
        "milkdrift-native-structure-target",
        "milkdrift-native-check-target",
        "milkdrift-native-test-target",
        "milkdrift-native-clippy-target",
        "milkdrift-native-docs-target",
        "milkdrift-native-benches-target",
        "milkdrift-portable-wasm-target",
        "milkdrift-portable-thumb-target",
        "milkdrift-policy-target",
        "milkdrift-nursery-target",
        "milkdrift-external-links-target",
        "milkdrift-cuda-check-target",
        "milkdrift-cuda-hardware-target",
    ];
    assert_eq!(
        targets.into_iter().collect::<BTreeSet<_>>().len(),
        targets.len()
    );
    for target in targets {
        assert!(
            quality.contains(target) || cuda.contains(target),
            "missing isolated workflow target {target}"
        );
    }
    assert_eq!(cuda.matches("if: ${{ always() }}").count(), 1);
    assert!(quality.contains(".github/scripts/ci-resource.sh cleanup"));
    assert!(cuda.contains(".github/scripts/ci-resource.sh cleanup"));
    assert!(!quality.contains("rm -rf"));
    assert!(!cuda.contains("rm -rf"));
    Ok(())
}

#[test]
fn official_actions_are_immutable_and_all_shell_parses() -> Result<(), Box<dyn Error>> {
    for workflow in [
        ".github/workflows/quality.yml",
        ".github/workflows/cuda-hardware.yml",
    ] {
        let yaml = read(workflow)?;
        let revisions = checkout_revisions(&yaml);
        assert!(!revisions.is_empty());
        assert!(revisions.iter().all(|revision| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        }));
        for (index, body) in embedded_shell_bodies(&yaml).iter().enumerate() {
            assert_shell_syntax(&format!("{workflow} run body {}", index + 1), body)?;
        }
    }

    let script = workspace_root().join(".github/scripts/ci-resource.sh");
    let output = Command::new("sh").arg("-n").arg(&script).output()?;
    assert!(
        output.status.success(),
        "resource script did not parse: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn maintained_workflows_parse_as_yaml() -> Result<(), Box<dyn Error>> {
    for workflow in [
        ".github/workflows/quality.yml",
        ".github/workflows/cuda-hardware.yml",
    ] {
        let yaml = read(workflow)?;
        let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml)
            .map_err(|error| format!("invalid YAML in {workflow}: {error}"))?;
    }
    Ok(())
}

struct TempEnvironment {
    root: PathBuf,
    workspace: PathBuf,
    runner_temp: PathBuf,
    github_env: PathBuf,
    github_path: PathBuf,
}

impl TempEnvironment {
    fn new() -> Result<Self, Box<dyn Error>> {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("xtask-ci-resource-{}-{id}", std::process::id()));
        let workspace = root.join("workspace");
        let runner_temp = root.join("runner-temp");
        let github_env = root.join("github-env");
        let github_path = root.join("github-path");
        fs::create_dir_all(workspace.join(".git"))?;
        fs::create_dir_all(&runner_temp)?;
        fs::write(&github_env, "")?;
        fs::write(&github_path, "")?;
        Ok(Self {
            root,
            workspace,
            runner_temp,
            github_env,
            github_path,
        })
    }

    fn run(&self, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
        Ok(
            Command::new(workspace_root().join(".github/scripts/ci-resource.sh"))
                .args(arguments)
                .env("GITHUB_WORKSPACE", &self.workspace)
                .env("RUNNER_TEMP", &self.runner_temp)
                .env("GITHUB_ENV", &self.github_env)
                .env("GITHUB_PATH", &self.github_path)
                .output()?,
        )
    }
}

impl Drop for TempEnvironment {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn resource_script_prepares_shims_cleans_and_fails_closed() -> Result<(), Box<dyn Error>> {
    let environment = TempEnvironment::new()?;
    let target = environment.runner_temp.join("test-target");
    let tools = environment.runner_temp.join("test-tools");
    let target_text = target.to_string_lossy().into_owned();
    let tools_text = tools.to_string_lossy().into_owned();

    let output = environment.run(&["prepare", "1024", &target_text, &tools_text])?;
    assert!(output.status.success());
    assert!(target.is_dir());
    assert!(tools.is_dir());

    let output = environment.run(&["forbidden-shims", &tools_text])?;
    assert!(output.status.success());
    assert!(tools.join("cmake").is_symlink());
    assert!(tools.join("python").is_symlink());

    fs::write(target.join("artifact"), "artifact")?;
    let output = environment.run(&["cleanup", &target_text, &tools_text])?;
    assert!(output.status.success());
    assert!(!target.exists());
    assert!(!tools.exists());

    fs::create_dir(environment.workspace.join("target"))?;
    let output = environment.run(&["prepare", "1024", &target_text])?;
    assert!(!output.status.success());
    fs::remove_dir(environment.workspace.join("target"))?;

    let outside = environment.root.join("outside");
    let output = environment.run(&["prepare", "1024", &outside.to_string_lossy()])?;
    assert!(!output.status.success());
    Ok(())
}
