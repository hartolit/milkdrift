//! Integration coverage for Git-backed repository hygiene validation.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use llm_app::validate_repository_hygiene;

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct FixtureRepository {
    root: PathBuf,
}

impl FixtureRepository {
    fn new(name: &str, files: &[(&str, &str)]) -> Result<Self, Box<dyn Error>> {
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "llm-app-hygiene-{name}-{}-{id}",
            std::process::id()
        ));
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
fn historical_and_negative_policy_text_is_allowed_without_substring_false_positives()
-> Result<(), Box<dyn Error>> {
    let repository = FixtureRepository::new(
        "allowed-explanations",
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
                "docs/agent/decisions/0001-old.md",
                "# Old decision\n\n- **Status:** Superseded\n\n```sh\npython3 old-tool.py\n```\n",
            ),
            (
                "docs/agent/execution/history.md",
                "Historical command:\n```sh\nhf download old/model\n```\n",
            ),
            (
                "docs/agent/execution/analyzer.md",
                "```sh\npip install old-package\n```\n",
            ),
            (
                "docs/agent/execution/example-cleanup-agent-brief.md",
                "```sh\npytest\n```\n",
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
        "allowed explanatory fixture violations: {:#?}",
        report.violations()
    );
    Ok(())
}
