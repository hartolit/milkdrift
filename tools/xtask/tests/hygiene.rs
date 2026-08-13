//! Integration coverage for Git-backed repository hygiene validation.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use xtask::validate_repository_hygiene;

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct FixtureRepository {
    root: PathBuf,
}

impl FixtureRepository {
    fn new(name: &str, files: &[(&str, &str)]) -> Result<Self, Box<dyn Error>> {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("xtask-hygiene-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root)?;

        for (relative, content) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)?;
        }

        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        run(
            Command::new(cargo)
                .args(["generate-lockfile", "--manifest-path"])
                .arg(root.join("Cargo.toml")),
            "generate fixture lockfile",
        )?;
        run(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&root),
            "initialize fixture repository",
        )?;
        run(
            Command::new("git").args(["add", "."]).current_dir(&root),
            "track fixture files",
        )?;

        Ok(Self { root })
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }
}

impl Drop for FixtureRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(command: &mut Command, operation: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("could not {operation}: process exited with {status}").into())
    }
}

const BASIC_MANIFEST: &str = "\
[package]
name = \"hygiene-fixture\"
version = \"0.1.0\"
edition = \"2024\"

[workspace]
";

const BASIC_SOURCE: &str = "pub fn fixture() {}\n";

#[test]
fn tracked_python_artifacts_and_maintained_invocations_fail() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "operational-python",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            (
                "build.rs",
                "fn main() { let _ = std::process::Command::new(\"python3\"); }\n",
            ),
            ("tools/check.py", "print('fixture')\n"),
            ("notebooks/analysis.ipynb", "{}\n"),
            ("pyproject.toml", "[project]\nname = \"fixture\"\n"),
            ("requirements-dev.txt", "package==1.0\n"),
            ("environment.yml", "name: fixture\n"),
            ("scripts/check", "#!/usr/bin/env python3\n"),
            ("build/tasks.toml", "command = \"python3 tools/check.py\"\n"),
            (
                "docs/project/validation.md",
                "Run `python3 tools/check.py` from the repository root.\n",
            ),
            (
                ".github/workflows/quality.yml",
                "steps:\n  - name: Download fixture\n    run: hf download owner/model\n",
            ),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    assert!(!report.is_valid());
    for path in [
        "tools/check.py",
        "notebooks/analysis.ipynb",
        "pyproject.toml",
        "requirements-dev.txt",
        "environment.yml",
    ] {
        assert!(report.violations().iter().any(|violation| {
            violation.rule() == "HYGIENE-PY-ARTIFACT-1" && violation.path() == Some(Path::new(path))
        }));
    }
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-PY-INVOKE-1"
            && violation.path() == Some(Path::new("docs/project/validation.md"))
            && violation.line() == Some(1)
            && violation.reason().contains("python3")
    }));
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-PY-INVOKE-1"
            && violation.path() == Some(Path::new(".github/workflows/quality.yml"))
            && violation.reason().contains("`hf`")
    }));
    for path in ["build.rs", "scripts/check", "build/tasks.toml"] {
        assert!(report.violations().iter().any(|violation| {
            violation.rule() == "HYGIENE-PY-INVOKE-1"
                && violation.path() == Some(Path::new(path))
                && violation.reason().contains("python3")
        }));
    }
    Ok(())
}

#[test]
fn direct_manifest_and_selected_graph_packages_fail_separately() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "forbidden-package",
        &[
            (
                "Cargo.toml",
                "\
[package]
name = \"hygiene-fixture\"
version = \"0.1.0\"
edition = \"2024\"

[dependencies]
embedded = { package = \"pyo3\", path = \"vendor/pyo3\" }

[workspace]
members = [\"vendor/pyo3\"]
",
            ),
            ("src/lib.rs", BASIC_SOURCE),
            (
                "vendor/pyo3/Cargo.toml",
                "\
[package]
name = \"pyo3\"
version = \"0.1.0\"
edition = \"2024\"
",
            ),
            ("vendor/pyo3/src/lib.rs", BASIC_SOURCE),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-MANIFEST-1"
            && violation.path() == Some(Path::new("Cargo.toml"))
            && violation.reason().contains("`pyo3`")
    }));
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-GRAPH-1"
            && violation.path().is_none()
            && violation.reason().contains("`pyo3`")
    }));
    Ok(())
}

#[test]
fn historical_names_and_superseded_adrs_do_not_bypass_scanning() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "historical-surfaces",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            (
                "docs/agent/decisions/0001-old.md",
                "# Old decision\n\n- **Status:** Superseded\n\nHistorical command:\n```sh\npython3 old-tool.py\n```\n",
            ),
            (
                "docs/history/runbook.md",
                "Historical command:\n```sh\nhf download old/model\n```\n",
            ),
            (
                "docs/agent/execution/history.md",
                "Historical command:\n```sh\npython old-tool.py\n```\n",
            ),
            (
                "docs/agent/execution/analyzer.md",
                "Recorded command:\n```sh\npip install old-package\n```\n",
            ),
            (
                "docs/agent/execution/example-cleanup-agent-brief.md",
                "Recorded command:\n```sh\npytest\n```\n",
            ),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    for path in [
        "docs/agent/decisions/0001-old.md",
        "docs/history/runbook.md",
        "docs/agent/execution/history.md",
        "docs/agent/execution/analyzer.md",
        "docs/agent/execution/example-cleanup-agent-brief.md",
    ] {
        assert!(report.violations().iter().any(|violation| {
            violation.rule() == "HYGIENE-PY-INVOKE-1" && violation.path() == Some(Path::new(path))
        }));
    }
    Ok(())
}

#[test]
fn documentation_authority_rejects_retired_paths_archives_and_missing_maps()
-> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "documentation-authority-invalid",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            ("README.md", "# Project\n"),
            ("docs/README.md", "# Documentation\n"),
            ("docs/vision.md", "# Vision\n"),
            ("docs/project/README.md", "# Project docs\n"),
            ("docs/project/architecture.md", "# Architecture\n"),
            ("docs/project/implementation-status.md", "# Status\n"),
            ("docs/project/validation.md", "# Validation\n"),
            ("docs/project/performance.md", "# Performance\n"),
            ("docs/project/implementation-plan.md", "# Old plan\n"),
            (
                "docs/agent/application-runtime-architecture-warning.md",
                "# Resolved warning\n",
            ),
            ("docs/agent/decisions/README.md", "# Decisions\n"),
            ("docs/agent/execution/README.md", "# Execution\n"),
            ("docs/agent/execution/current.md", "# Current\n"),
            ("docs/agent/execution/execution-plan.md", "# Plan\n"),
            ("docs/agent/execution/history.md", "# History\n"),
            ("docs/agent/execution/analyzer.md", "# Old analysis\n"),
            (
                "docs/agent/execution/archive/completed-prompt.md",
                "# Completed prompt\n",
            ),
            ("docs/agent/execution/archive/README.md", "# Provenance\n"),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    for path in [
        "docs/agent/application-runtime-architecture-warning.md",
        "docs/agent/execution/analyzer.md",
        "docs/project/implementation-plan.md",
    ] {
        assert!(report.violations().iter().any(|violation| {
            violation.rule() == "HYGIENE-DOCUMENTATION-LAYOUT-1"
                && violation.path() == Some(Path::new(path))
        }));
    }
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-DOCUMENTATION-ARCHIVE-1"
            && violation.path()
                == Some(Path::new(
                    "docs/agent/execution/archive/completed-prompt.md",
                ))
    }));
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-DOCUMENTATION-LAYOUT-1"
            && violation.path() == Some(Path::new("docs/project/operation.md"))
    }));
    assert!(report.violations().iter().any(|violation| {
        violation.rule() == "HYGIENE-DOCUMENTATION-LAYOUT-1"
            && violation.path() == Some(Path::new("docs/agent/execution/README.md"))
            && violation.reason().contains("execution-plan.md")
    }));
    Ok(())
}

#[test]
fn documentation_authority_accepts_the_current_spine() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "documentation-authority-valid",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            ("README.md", "# Project\n"),
            (
                "docs/README.md",
                "[Architecture](project/architecture.md) [Operation](project/operation.md) [Status](project/implementation-status.md)\n",
            ),
            ("docs/vision.md", "# Vision\n"),
            (
                "docs/project/README.md",
                "[Operation](operation.md) [Status](implementation-status.md)\n",
            ),
            ("docs/project/architecture.md", "# Architecture\n"),
            ("docs/project/operation.md", "# Operation\n"),
            ("docs/project/implementation-status.md", "# Status\n"),
            ("docs/project/validation.md", "# Validation\n"),
            ("docs/project/performance.md", "# Performance\n"),
            ("docs/agent/decisions/README.md", "# Decisions\n"),
            (
                "docs/agent/execution/README.md",
                "[Current](current.md) [Plan](execution-plan.md)\n",
            ),
            ("docs/agent/execution/current.md", "# Current\n"),
            ("docs/agent/execution/execution-plan.md", "# Plan\n"),
            ("docs/agent/execution/history.md", "# History\n"),
            ("docs/agent/execution/archive/README.md", "# Provenance\n"),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    assert!(
        report.is_valid(),
        "valid documentation authority violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn tracked_utf8_text_rejects_trailing_whitespace_outside_rust_sources() -> Result<(), Box<dyn Error>>
{
    let repository = FixtureRepository::new(
        "tracked-whitespace",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            ("docs/readme.md", "clean\ntrailing space \ntrailing tab\t\n"),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    let lines = report
        .violations()
        .iter()
        .filter(|violation| {
            violation.rule() == "HYGIENE-TRACKED-WHITESPACE-1"
                && violation.path() == Some(Path::new("docs/readme.md"))
        })
        .filter_map(xtask::HygieneViolation::line)
        .collect::<Vec<_>>();
    assert_eq!(lines, vec![2, 3]);
    Ok(())
}

#[test]
fn tracked_build_and_benchmark_artifacts_fail_closed() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "generated-artifacts",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            ("target/debug/output", "generated\n"),
            ("crates/domain/sampling/target/report", "generated\n"),
            ("benchmarks/runtime/Cargo.lock", "version = 4\n"),
            ("benchmarks/runtime/results/report.json", "{}\n"),
            ("benchmarks/runtime/model-cache/blob", "cached\n"),
            ("benchmarks/runtime/.cache/hub/blob", "cached\n"),
            ("benchmarks/runtime/build.rs", "fn main() {}\n"),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    for (path, rule) in [
        ("target/debug/output", "HYGIENE-TARGET-1"),
        ("crates/domain/sampling/target/report", "HYGIENE-TARGET-1"),
        ("benchmarks/runtime/Cargo.lock", "HYGIENE-BENCHMARK-LOCK-1"),
        (
            "benchmarks/runtime/results/report.json",
            "HYGIENE-BENCHMARK-OUTPUT-1",
        ),
        (
            "benchmarks/runtime/model-cache/blob",
            "HYGIENE-MODEL-CACHE-1",
        ),
        (
            "benchmarks/runtime/.cache/hub/blob",
            "HYGIENE-MODEL-CACHE-1",
        ),
        ("benchmarks/runtime/build.rs", "HYGIENE-BENCHMARK-BUILD-1"),
    ] {
        assert!(report.violations().iter().any(|violation| {
            violation.path() == Some(Path::new(path)) && violation.rule() == rule
        }));
    }
    Ok(())
}

#[test]
fn project_manifests_must_be_known_root_workspace_members() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "workspace-membership",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            (
                "benchmarks/runtime/Cargo.toml",
                "[package]\nname = \"runtime-benchmarks\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n",
            ),
            (
                "benchmarks/experimental/Cargo.toml",
                "[package]\nname = \"experimental-benchmarks\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n",
            ),
            (
                "crates/runtime/hidden/Cargo.toml",
                "[package]\nname = \"hidden-runtime\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("crates/runtime/hidden/src/lib.rs", BASIC_SOURCE),
            (
                "misc/hidden/Cargo.toml",
                "[package]\nname = \"hidden-misc\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("misc/hidden/src/lib.rs", BASIC_SOURCE),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    for path in [
        "benchmarks/runtime/Cargo.toml",
        "benchmarks/experimental/Cargo.toml",
        "crates/runtime/hidden/Cargo.toml",
        "misc/hidden/Cargo.toml",
    ] {
        assert!(report.violations().iter().any(|violation| {
            violation.path() == Some(Path::new(path))
                && violation.rule() == "HYGIENE-WORKSPACE-MEMBER-1"
        }));
    }
    Ok(())
}

#[test]
fn nearby_source_and_fixture_paths_remain_allowed() -> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "allowed-artifact-neighbors",
        &[
            ("Cargo.toml", BASIC_MANIFEST),
            ("src/lib.rs", BASIC_SOURCE),
            ("docs/target.md", "documentation\n"),
            ("fixtures/benches/generated_cases.rs", "pub fn cases() {}\n"),
            ("fixtures/model-cache-key.txt", "key\n"),
            ("crates/apps/desktop-slint/build.rs", "fn main() {}\n"),
            (
                "crates/runtime/inference-runtime/tests/fixtures/model.safetensors",
                "synthetic fixture\n",
            ),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    assert!(
        report.is_valid(),
        "allowed artifact-neighbor violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn repository_ignores_target_directories_at_every_depth() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "target/debug/output",
        "crates/domain/sampling/target/debug/output",
        "benchmarks/runtime/target/criterion/report",
    ] {
        run(
            Command::new("git")
                .args(["check-ignore", "--quiet", path])
                .current_dir(&root),
            "check recursive target ignore policy",
        )?;
    }
    Ok(())
}

#[test]
fn negative_policy_text_is_allowed_without_substring_false_positives() -> Result<(), Box<dyn Error>>
{
    let repository = FixtureRepository::new(
        "allowed-policy",
        &[
            (
                "Cargo.toml",
                "\
[package]
name = \"hygiene-fixture\"
version = \"0.1.0\"
edition = \"2024\"
# pyo3 = \"0.1\" is intentionally only explanatory text.

[workspace]
",
            ),
            (
                "src/lib.rs",
                "pub const PIPER: &str = \"piper\";\npub const PIPELINE: &str = \"sampling_pipeline\";\n",
            ),
            (
                "docs/policy.md",
                "\
Do not run `python3 tools/check.py`.
The `hf download` command is prohibited.

The following command is forbidden policy evidence:
```sh
pip install package
```

Run `piper` for the audio example.
Run `cargo bench --bench sampling_pipeline` for the benchmark.
",
            ),
        ],
    )?;
    let report = validate_repository_hygiene(&repository.manifest())?;

    assert!(
        report.is_valid(),
        "allowed policy fixture violations: {:#?}",
        report.violations()
    );
    Ok(())
}
