use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_metadata::{Metadata, MetadataCommand};

use crate::workspace::{Role, benchmark_inventory, package_role, role_location_is_compatible};

use super::documentation::scan_documentation_authority;
use super::invocation::{is_potential_operational_surface, scan_operational_invocations};
use super::manifest::{is_cargo_manifest, scan_manifest, scan_selected_graph};

const RULE_PYTHON_ARTIFACT: &str = "HYGIENE-PY-ARTIFACT-1";
const RULE_TARGET_ARTIFACT: &str = "HYGIENE-TARGET-1";
const RULE_BENCHMARK_LOCKFILE: &str = "HYGIENE-BENCHMARK-LOCK-1";
const RULE_BENCHMARK_OUTPUT: &str = "HYGIENE-BENCHMARK-OUTPUT-1";
const RULE_MODEL_CACHE: &str = "HYGIENE-MODEL-CACHE-1";
const RULE_BENCHMARK_BUILD: &str = "HYGIENE-BENCHMARK-BUILD-1";
const RULE_WORKSPACE_MEMBER: &str = "HYGIENE-WORKSPACE-MEMBER-1";
const RULE_BENCHMARK_LAYOUT: &str = "HYGIENE-BENCHMARK-LAYOUT-1";
const RULE_BENCHMARK_REGISTRY: &str = "HYGIENE-BENCHMARK-REGISTRY-1";
const RULE_TRACKED_WHITESPACE: &str = "HYGIENE-TRACKED-WHITESPACE-1";
const RULE_DOCUMENTATION_LAYOUT: &str = "HYGIENE-DOCUMENTATION-LAYOUT-1";
const RULE_DOCUMENTATION_ARCHIVE: &str = "HYGIENE-DOCUMENTATION-ARCHIVE-1";

/// One actionable repository hygiene policy violation.
#[derive(Debug, PartialEq, Eq)]
pub struct HygieneViolation {
    path: Option<PathBuf>,
    line: Option<usize>,
    rule: &'static str,
    reason: String,
}

impl HygieneViolation {
    pub(super) fn new(
        path: Option<PathBuf>,
        line: Option<usize>,
        rule: &'static str,
        reason: String,
    ) -> Self {
        Self {
            path,
            line,
            rule,
            reason,
        }
    }

    /// Returns the stable identifier of the policy rule that was violated.
    #[must_use]
    pub const fn rule(&self) -> &'static str {
        self.rule
    }

    /// Returns the repository-relative path associated with the violation, when applicable.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns the one-based source line associated with the violation, when applicable.
    #[must_use]
    pub const fn line(&self) -> Option<usize> {
        self.line
    }

    /// Returns the actionable reason the policy rejected the item.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for HygieneViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("repository hygiene violation")?;
        if let Some(path) = &self.path {
            write!(formatter, " at {}", path.display())?;
            if let Some(line) = self.line {
                write!(formatter, ":{line}")?;
            }
        }
        write!(formatter, "; policy rule {}: {}", self.rule, self.reason)
    }
}

/// The complete result of validating repository hygiene.
#[derive(Debug, Default)]
pub struct HygieneReport {
    violations: Vec<HygieneViolation>,
}

impl HygieneReport {
    pub(super) fn push(&mut self, violation: HygieneViolation) {
        self.violations.push(violation);
    }

    /// Returns true when the repository satisfies every hygiene rule.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    /// Returns all hygiene violations in deterministic validation order.
    #[must_use]
    pub fn violations(&self) -> &[HygieneViolation] {
        &self.violations
    }
}

/// An error that prevented repository hygiene validation from completing.
#[derive(Debug)]
pub struct HygieneError {
    message: String,
}

impl HygieneError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HygieneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for HygieneError {}

/// Validates tracked repository files, direct Cargo declarations, and the locked selected graph.
///
/// Tracked paths are obtained from Git. Deleted paths in an uncommitted cleanup are ignored once
/// they no longer exist in the working tree. Cargo metadata is loaded with `--locked`, including
/// resolved dependencies, so both dormant direct declarations and selected packages are checked.
/// Every tracked operational surface is scanned without filename-, directory-, or ADR-status-based
/// exemptions; negative policy prose is distinguished by the invocation parser itself.
///
/// # Errors
///
/// Returns an error if locked Cargo metadata, Git's tracked path list, or a maintained text surface
/// cannot be read.
pub fn validate_repository_hygiene(manifest_path: &Path) -> Result<HygieneReport, HygieneError> {
    let metadata = load_metadata(manifest_path)?;
    let root = metadata.workspace_root.as_std_path();
    let tracked_paths = tracked_paths(root)?;
    validate_hygiene(root, &tracked_paths, &metadata)
}

fn load_metadata(manifest_path: &Path) -> Result<Metadata, HygieneError> {
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest_path)
        .other_options(vec!["--locked".to_owned()]);
    if let Some(cargo) = env::var_os("CARGO") {
        command.cargo_path(cargo);
    }
    command.exec().map_err(|error| {
        HygieneError::new(format!("could not load locked Cargo metadata: {error}"))
    })
}

fn tracked_paths(root: &Path) -> Result<Vec<PathBuf>, HygieneError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--cached", "-z"])
        .output()
        .map_err(|error| HygieneError::new(format!("could not execute git ls-files: {error}")))?;
    if !output.status.success() {
        return Err(HygieneError::new(format!(
            "git ls-files failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(PathBuf::from)
                .map_err(|error| {
                    HygieneError::new(format!(
                        "git reported a tracked path that is not valid UTF-8: {error}"
                    ))
                })
        })
        .collect()
}

fn validate_hygiene(
    root: &Path,
    tracked_paths: &[PathBuf],
    metadata: &Metadata,
) -> Result<HygieneReport, HygieneError> {
    let mut report = HygieneReport::default();
    let workspace_manifests = metadata
        .workspace_packages()
        .iter()
        .filter_map(|package| {
            package
                .manifest_path
                .as_std_path()
                .strip_prefix(root)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<BTreeSet<_>>();
    let benchmark_manifests = metadata
        .workspace_packages()
        .iter()
        .filter(|package| {
            matches!(package_role(package), Ok(Role::BenchmarkObserver))
                && role_location_is_compatible(root, package, Role::BenchmarkObserver)
        })
        .filter_map(|package| {
            package
                .manifest_path
                .as_std_path()
                .strip_prefix(root)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<BTreeSet<_>>();

    scan_benchmark_registry(root, metadata, &mut report);
    scan_documentation_layout(root, tracked_paths, &mut report)?;

    for relative in tracked_paths {
        let absolute = root.join(relative);
        let file_metadata = match fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(HygieneError::new(format!(
                    "could not inspect tracked path {}: {error}",
                    relative.display()
                )));
            }
        };

        scan_tracked_path(
            relative,
            &workspace_manifests,
            &benchmark_manifests,
            &mut report,
        );

        if is_python_artifact(relative) {
            report.push(HygieneViolation::new(
                Some(relative.clone()),
                None,
                RULE_PYTHON_ARTIFACT,
                "tracked project-owned Python, notebook, package, or environment artifacts are prohibited; replace the maintained operation with Rust/Cargo tooling and remove this file".to_owned(),
            ));
        }

        if !file_metadata.file_type().is_file() {
            continue;
        }

        scan_tracked_text_whitespace(&absolute, relative, &mut report)?;

        let cargo_manifest = is_cargo_manifest(relative);
        let operational = is_potential_operational_surface(relative);
        if !cargo_manifest && !operational {
            continue;
        }

        let content = fs::read_to_string(&absolute).map_err(|error| {
            HygieneError::new(format!(
                "could not read maintained text surface {}: {error}",
                relative.display()
            ))
        })?;

        if cargo_manifest {
            scan_manifest(relative, &content, &mut report);
        }
        if operational {
            scan_operational_invocations(relative, &content, &mut report);
        }
    }

    scan_selected_graph(metadata, &mut report);
    Ok(report)
}

fn scan_documentation_layout(
    root: &Path,
    tracked_paths: &[PathBuf],
    report: &mut HygieneReport,
) -> Result<(), HygieneError> {
    let present = tracked_paths
        .iter()
        .filter(|path| root.join(path).exists())
        .cloned()
        .collect::<BTreeSet<_>>();

    scan_retired_documentation(&present, report);
    scan_documentation_authority(root, &present, report)?;

    if !present.contains(Path::new("docs/README.md")) {
        return Ok(());
    }

    for required in [
        "README.md",
        "docs/vision.md",
        "docs/project/README.md",
        "docs/project/architecture.md",
        "docs/project/operation.md",
        "docs/project/implementation-status.md",
        "docs/project/validation.md",
        "docs/project/performance.md",
        "docs/agent/decisions/README.md",
        "docs/agent/execution/README.md",
        "docs/agent/execution/current.md",
        "docs/agent/execution/execution-plan.md",
        "docs/agent/execution/history.md",
    ] {
        let path = Path::new(required);
        if !present.contains(path) {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                None,
                RULE_DOCUMENTATION_LAYOUT,
                "the current documentation authority spine requires this tracked file".to_owned(),
            ));
        }
    }

    for (map, required_links) in [
        (
            "docs/README.md",
            &[
                "(project/architecture.md)",
                "(project/operation.md)",
                "(project/implementation-status.md)",
            ][..],
        ),
        (
            "docs/project/README.md",
            &["(operation.md)", "(implementation-status.md)"][..],
        ),
        (
            "docs/agent/execution/README.md",
            &["(current.md)", "(execution-plan.md)"][..],
        ),
    ] {
        let map_path = Path::new(map);
        if !present.contains(map_path) {
            continue;
        }
        let content = fs::read_to_string(root.join(map_path)).map_err(|error| {
            HygieneError::new(format!(
                "could not read documentation map {}: {error}",
                map_path.display()
            ))
        })?;
        for required_link in required_links {
            if !content.contains(required_link) {
                report.push(HygieneViolation::new(
                    Some(map_path.to_path_buf()),
                    None,
                    RULE_DOCUMENTATION_LAYOUT,
                    format!("documentation map must index `{required_link}`"),
                ));
            }
        }
    }

    Ok(())
}

fn scan_retired_documentation(present: &BTreeSet<PathBuf>, report: &mut HygieneReport) {
    for retired in [
        "docs/agent/application-runtime-architecture-warning.md",
        "docs/agent/execution/analyzer.md",
        "docs/project/implementation-plan.md",
    ] {
        let path = Path::new(retired);
        if present.contains(path) {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                None,
                RULE_DOCUMENTATION_LAYOUT,
                "retired documentation authority must remain in Git history rather than the active tree"
                    .to_owned(),
            ));
        }
    }

    let archive = Path::new("docs/agent/execution/archive");
    for path in present.iter().filter(|path| {
        path.starts_with(archive)
            && path.extension().and_then(|extension| extension.to_str()) == Some("md")
            && path.as_path() != archive.join("README.md")
    }) {
        report.push(HygieneViolation::new(
            Some(path.clone()),
            None,
            RULE_DOCUMENTATION_ARCHIVE,
            "completed prompt bodies are prohibited in the active tree; retain only archive/README.md provenance and use Git history for the original text"
                .to_owned(),
        ));
    }
}

fn scan_benchmark_registry(root: &Path, metadata: &Metadata, report: &mut HygieneReport) {
    let Err(issues) = benchmark_inventory(metadata) else {
        return;
    };
    for issue in issues {
        let path = metadata
            .workspace_packages()
            .iter()
            .find(|package| package.name == issue.package)
            .and_then(|package| {
                package
                    .manifest_path
                    .as_std_path()
                    .strip_prefix(root)
                    .ok()
                    .map(Path::to_path_buf)
            });
        report.push(HygieneViolation::new(
            path,
            None,
            RULE_BENCHMARK_REGISTRY,
            format!("{}: {}", issue.target, issue.reason),
        ));
    }
}

fn scan_tracked_text_whitespace(
    absolute: &Path,
    relative: &Path,
    report: &mut HygieneReport,
) -> Result<(), HygieneError> {
    let bytes = fs::read(absolute).map_err(|error| {
        HygieneError::new(format!(
            "could not read tracked file {} for whitespace validation: {error}",
            relative.display()
        ))
    })?;
    if bytes.contains(&0) {
        return Ok(());
    }
    let Ok(content) = std::str::from_utf8(&bytes) else {
        return Ok(());
    };

    for (line_index, line) in content.split('\n').enumerate() {
        if line
            .as_bytes()
            .last()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        {
            report.push(HygieneViolation::new(
                Some(relative.to_path_buf()),
                Some(line_index.saturating_add(1)),
                RULE_TRACKED_WHITESPACE,
                "tracked UTF-8 text must not contain trailing spaces, tabs, or carriage returns"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn scan_tracked_path(
    path: &Path,
    workspace_manifests: &BTreeSet<PathBuf>,
    benchmark_manifests: &BTreeSet<PathBuf>,
    report: &mut HygieneReport,
) {
    if has_component(path, "target") {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_TARGET_ARTIFACT,
            "tracked paths cannot contain a component named target; all Cargo output belongs under the ignored root target directory".to_owned(),
        ));
    }

    let benchmark_path = path.starts_with(Path::new("benchmarks"));
    let file_name = path.file_name().and_then(|name| name.to_str());
    if is_project_package_manifest(path) && !workspace_manifests.contains(path) {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_WORKSPACE_MEMBER,
            "every tracked non-fixture Cargo package must be a root workspace member so role policy and canonical Rust gates cannot be bypassed"
                .to_owned(),
        ));
    }
    if benchmark_path && file_name == Some("Cargo.lock") {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_BENCHMARK_LOCKFILE,
            "benchmark packages are root-workspace members and must use the root Cargo.lock"
                .to_owned(),
        ));
    }
    if benchmark_path && file_name == Some("build.rs") {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_BENCHMARK_BUILD,
            "benchmark packages cannot contain build.rs; generation, downloads, measurement, and machine probing must remain explicit runtime operations".to_owned(),
        ));
    }
    if benchmark_path
        && file_name == Some("Cargo.toml")
        && workspace_manifests.contains(path)
        && !benchmark_manifests.contains(path)
    {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_BENCHMARK_LAYOUT,
            "workspace members under benchmarks/ require the explicit benchmark-observer role at a compatible direct-child location"
                .to_owned(),
        ));
    }

    if [
        "criterion",
        "results",
        "generated-report",
        "generated-reports",
        "flamegraph",
        "flamegraphs",
        "profiler-output",
        "profiler-outputs",
        "heap-dumps",
    ]
    .into_iter()
    .any(|component| has_component(path, component))
    {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_BENCHMARK_OUTPUT,
            "generated benchmark, Criterion, flamegraph, profiler, and heap-dump trees are not repository artifacts; keep raw output under the root target directory".to_owned(),
        ));
    }

    if [
        ".cache",
        "cache",
        "model-cache",
        "model-caches",
        "downloaded-model",
        "downloaded-models",
        "huggingface-cache",
        "hf-cache",
    ]
    .into_iter()
    .any(|component| has_component(path, component))
    {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            None,
            RULE_MODEL_CACHE,
            "downloaded model and benchmark caches must remain external or under the ignored root target directory".to_owned(),
        ));
    }
}

fn is_project_package_manifest(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
        && path != Path::new("Cargo.toml")
        && !path.starts_with(Path::new("tools/xtask/tests/fixtures"))
}

fn has_component(path: &Path, expected: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == expected)
}

fn is_python_artifact(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();

    if ["py", "pyi", "pyw", "pyx", "pxd", "pxi", "ipynb"]
        .into_iter()
        .any(|extension| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        })
    {
        return true;
    }

    matches!(
        lower.as_str(),
        "pyproject.toml"
            | "pipfile"
            | "pipfile.lock"
            | "poetry.lock"
            | "uv.lock"
            | "setup.cfg"
            | "tox.ini"
            | "pytest.ini"
            | "mypy.ini"
            | ".mypy.ini"
            | ".pylintrc"
            | "ruff.toml"
            | ".ruff.toml"
            | ".coveragerc"
            | ".python-version"
            | "py.typed"
            | "environment.yml"
            | "environment.yaml"
            | "conda.yml"
            | "conda.yaml"
    ) || (lower.starts_with("requirements")
        && Path::new(&lower)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt")))
}
