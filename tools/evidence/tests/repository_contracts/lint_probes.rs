//! Small compile probes establish selected pinned-tool behavior, not a copy of upstream lint tests.

use std::{fs, process::Command};

use super::{TestResult, read, root};

fn probe(
    lint: &str,
    source: &str,
    configuration: Option<&str>,
) -> TestResult<std::process::Output> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("src"))?;
    fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='policy-probe'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[features]\nprobe=[]\n",
    )?;
    fs::write(directory.path().join("src/lib.rs"), source)?;
    if source.contains("tokio::") {
        let workspace: toml::Value = toml::from_str(&read(root()?.join("Cargo.toml"))?)?;
        let version = workspace
            .get("workspace")
            .and_then(|v| v.get("dependencies"))
            .and_then(|v| v.get("tokio"))
            .and_then(|v| v.get("version"))
            .and_then(toml::Value::as_str)
            .ok_or("missing pinned Tokio declaration")?;
        let path = directory.path().join("Cargo.toml");
        let mut manifest = fs::read_to_string(&path)?;
        manifest.push_str(&format!(
            "[dependencies]\ntokio={{version='={version}',features=['sync']}}\n"
        ));
        fs::write(path, manifest)?;
    }
    if let Some(configuration) = configuration {
        fs::write(directory.path().join("clippy.toml"), configuration)?;
    }
    Ok(Command::new(env!("CARGO"))
        .current_dir(directory.path())
        .args([
            "clippy",
            "--offline",
            "--quiet",
            "--all-features",
            "--message-format=json",
            "--",
            "-D",
            lint,
            "-D",
            "unknown_lints",
            "-D",
            "renamed_and_removed_lints",
        ])
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("CLIPPY_CONF_DIR", directory.path())
        .output()?)
}

fn has_diagnostic(output: &std::process::Output, lint: &str) -> bool {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|value| value.pointer("/message/code/code").and_then(|v| v.as_str()) == Some(lint))
}

#[test]
fn pinned_compiler_and_clippy_reject_config_and_representative_mistakes() -> TestResult {
    for (lint, invalid, valid) in [
        (
            "renamed_and_removed_lints",
            "#![allow(clippy::integer_arithmetic)] pub fn run() {}",
            "pub fn run() {}",
        ),
        (
            "unfulfilled_lint_expectations",
            "#[cfg_attr(feature=\"probe\", expect(dead_code, reason=\"Active fixture has no dead code\"))] pub fn run() {}",
            "#[cfg_attr(feature=\"probe\", expect(dead_code, reason=\"Active fixture contains an unused function\"))] fn run() {}",
        ),
        (
            "unknown_lints",
            "#[allow(milkdrift_misspelled)] pub fn run() {}",
            "pub fn run() {}",
        ),
        (
            "unexpected_cfgs",
            "#[cfg(feature=\"misspelled\")] pub fn run() {}",
            "#[cfg(feature=\"probe\")] pub fn run() {}",
        ),
        (
            "unfulfilled_lint_expectations",
            "#[expect(dead_code, reason=\"This fixture intentionally has no dead code\")] pub fn run() {}",
            "#[expect(dead_code, reason=\"This fixture intentionally contains dead code\")] fn run() {}",
        ),
        (
            "unreachable_pub",
            "mod private { pub struct Hidden; }",
            "pub mod public { pub struct Visible; }",
        ),
        (
            "clippy::mem_forget",
            "pub fn run(v: String) { std::mem::forget(v); }",
            "pub fn run(v: String) { drop(v); }",
        ),
        (
            "clippy::let_underscore_must_use",
            "pub fn run(v: Result<(), ()>) { let _ = v; }",
            "pub fn run(v: Result<(), ()>) -> Result<(), ()> { v }",
        ),
        (
            "clippy::cast_possible_truncation",
            "pub fn run(v: u64) -> u8 { v as u8 }",
            "pub fn run(v: u64) -> Result<u8, std::num::TryFromIntError> { u8::try_from(v) }",
        ),
        (
            "clippy::indexing_slicing",
            "pub fn run(v: &[u8], i: usize) -> u8 { v[i] }",
            "pub fn run(v: &[u8], i: usize) -> Option<&u8> { v.get(i) }",
        ),
        (
            "clippy::arithmetic_side_effects",
            "pub fn run(a: u64, b: u64) -> u64 { a + b }",
            "pub fn run(a: u64, b: u64) -> Option<u64> { a.checked_add(b) }",
        ),
        (
            "clippy::unused_io_amount",
            "pub fn run(w: &mut impl std::io::Write) -> std::io::Result<()> { w.write(b\"abc\")?; Ok(()) }",
            "pub fn run(w: &mut impl std::io::Write) -> std::io::Result<()> { w.write_all(b\"abc\") }",
        ),
        (
            "clippy::let_underscore_future",
            "pub fn run() { let _ = async {}; }",
            "pub async fn run() { async {}.await; }",
        ),
        (
            "clippy::missing_errors_doc",
            "pub fn run() -> Result<(), ()> { Ok(()) }",
            "/// # Errors\n/// Refuses invalid input before any effects.\npub fn run() -> Result<(), ()> { Ok(()) }",
        ),
        (
            "clippy::allow_attributes_without_reason",
            "#[allow(dead_code)] fn run() {}",
            "#[allow(dead_code, reason=\"Fixture deliberately contains an unused function\")] fn run() {}",
        ),
    ] {
        let output = probe(lint, invalid, None)?;
        assert!(
            !output.status.success() && has_diagnostic(&output, lint),
            "{lint} not enforced: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let output = probe(lint, valid, None)?;
        assert!(
            output.status.success(),
            "{lint} valid counterpart refused: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = probe(
        "warnings",
        "pub fn valid() {}",
        Some("milkdrift-unknown-configuration = true"),
    )?;
    assert!(!output.status.success());
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostics.contains("unknown field"),
        "unknown configuration failed for another reason: {diagnostics}"
    );
    Ok(())
}

#[test]
fn configured_disallowed_apis_are_resolved_through_aliases() -> TestResult {
    let path = root()?.join("clippy.toml");
    let configuration = read(path)?;
    let output = probe(
        "clippy::disallowed_methods",
        "use std::sync::mpsc::channel as hidden; pub fn run() { let (_send, _recv) = hidden::<u8>(); }",
        Some(&configuration),
    )?;
    assert!(!output.status.success() && has_diagnostic(&output, "clippy::disallowed_methods"));
    let output = probe(
        "clippy::disallowed_methods",
        "pub fn run() { let (_send, _recv) = std::sync::mpsc::sync_channel::<u8>(1); }",
        Some(&configuration),
    )?;
    assert!(output.status.success());
    for (lint, source) in [
        (
            "clippy::disallowed_methods",
            "use tokio::sync::mpsc::unbounded_channel as hidden; pub fn run() { let (_send, _recv) = hidden::<u8>(); }",
        ),
        (
            "clippy::disallowed_types",
            "use tokio::sync::mpsc::UnboundedSender as Hidden; pub fn run(_: Hidden<u8>) {}",
        ),
        (
            "clippy::disallowed_methods",
            "use std::boxed::Box as Owner; pub fn run() { let _v = Owner::leak(Owner::new(1)); }",
        ),
        (
            "clippy::disallowed_methods",
            "pub fn run(v: Vec<u8>) { let _v = v.leak(); }",
        ),
        (
            "clippy::disallowed_types",
            "use std::sync::mpsc::Sender as Hidden; pub fn run(_: Hidden<u8>) {}",
        ),
    ] {
        let output = probe(lint, source, Some(&configuration))?;
        assert!(
            !output.status.success() && has_diagnostic(&output, lint),
            "unresolved configured API: {lint}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
