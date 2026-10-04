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
    if let Some(configuration) = configuration {
        fs::write(directory.path().join("clippy.toml"), configuration)?;
    }
    Ok(Command::new(env!("CARGO"))
        .current_dir(directory.path())
        .args([
            "clippy",
            "--offline",
            "--quiet",
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
        .any(|value| value["message"]["code"]["code"] == lint)
}

#[test]
fn pinned_compiler_and_clippy_reject_config_and_representative_mistakes() -> TestResult {
    for (lint, invalid, valid) in [
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
    // This test becomes mandatory with activation of the root behavior configuration.
    let path = root()?.join("clippy.toml");
    let configuration = if path.exists() {
        read(path)?
    } else {
        "disallowed-methods = [{ path = \"std::sync::mpsc::channel\", reason = \"Unbounded queues need an explicit capacity owner\" }]".to_owned()
    };
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
    Ok(())
}
