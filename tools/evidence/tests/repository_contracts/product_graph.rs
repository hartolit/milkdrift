//! Product-only Cargo resolution must not inherit the evidence workspace's helper-feature union.

use std::{path::Path, process::Command};

use super::{TestResult, root};

pub(super) const HELPER_FEATURES: &[&str] = &["test-support", "test-admin", "operational-evidence"];

fn features(directory: &Path, packages: &[&str]) -> TestResult<String> {
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(directory).args([
        "tree",
        "--offline",
        "--locked",
        "--edges",
        "normal,build",
        "--prefix",
        "none",
        "--format",
        "{p}|{f}",
    ]);
    for package in packages {
        command.args(["--package", package]);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "product feature discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn leaked_features(tree: &str) -> Vec<&str> {
    tree.lines()
        .filter(|line| {
            line.split_once('|').is_some_and(|(_, features)| {
                features
                    .split([',', ' '])
                    .any(|feature| HELPER_FEATURES.contains(&feature))
            })
        })
        .collect()
}

#[test]
fn daemon_cli_and_default_library_features_exclude_evidence_helpers() -> TestResult {
    let repository = root()?;
    // Each selection resolves independently: a workspace-wide all-feature query hides defaults.
    for packages in [
        vec!["milkdrift-daemon", "milkdrift-cli"],
        vec!["milkdrift-runtime"],
        vec!["milkdrift-capability-host"],
        vec!["milkdrift-redb-store"],
        vec!["milkdrift-model-provider"],
    ] {
        let tree = features(&repository, &packages)?;
        assert!(
            leaked_features(&tree).is_empty(),
            "helper feature in {packages:?}: {:?}",
            leaked_features(&tree)
        );
    }
    Ok(())
}

#[test]
fn real_cargo_resolution_distinguishes_dev_union_from_transitive_product_leak() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver='3'\nmembers=['app','bridge','helper','evidence']\n",
    )?;
    for (name, extra) in [
        (
            "app",
            "[dependencies]\nbridge={path='../bridge'}\n[dev-dependencies]\nhelper={path='../helper',features=['test-support']}",
        ),
        (
            "bridge",
            "[dependencies]\nalias={package='helper',path='../helper'}",
        ),
        ("helper", "[features]\ndefault=[]\ntest-support=[]"),
        (
            "evidence",
            "[dependencies]\nhelper={path='../helper',features=['test-support']}",
        ),
    ] {
        std::fs::create_dir_all(root.join(name).join("src"))?;
        std::fs::write(root.join(name).join("src/lib.rs"), "pub fn valid() {}")?;
        std::fs::write(
            root.join(name).join("Cargo.toml"),
            format!("[package]\nname='{name}'\nversion='0.1.0'\nedition='2024'\n{extra}\n"),
        )?;
    }
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["generate-lockfile", "--offline"])
        .status()?;
    assert!(status.success());
    assert!(leaked_features(&features(root, &["app"])?).is_empty());
    assert!(!leaked_features(&features(root, &["evidence"])?).is_empty());
    let bridge = root.join("bridge/Cargo.toml");
    let text = std::fs::read_to_string(&bridge)?.replace(
        "path='../helper'",
        "path='../helper',features=['test-support']",
    );
    std::fs::write(&bridge, text)?;
    assert!(!leaked_features(&features(root, &["app"])?).is_empty());
    Ok(())
}
