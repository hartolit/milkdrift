//! Rust-owned repository hygiene validation.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_metadata::{Metadata, MetadataCommand};

const RULE_PYTHON_ARTIFACT: &str = "HYGIENE-PY-ARTIFACT-1";
const RULE_OPERATIONAL_INVOCATION: &str = "HYGIENE-PY-INVOKE-1";
const RULE_MANIFEST_DEPENDENCY: &str = "HYGIENE-MANIFEST-1";
const RULE_SELECTED_GRAPH: &str = "HYGIENE-GRAPH-1";

const FORBIDDEN_PACKAGES: &[&str] = &[
    "gguf-backend",
    "llama-cpp-2",
    "llama-cpp-sys-2",
    "pyo3",
    "pyo3-ffi",
    "pyo3-build-config",
    "pythonize",
    "rustpython",
    "rustpython-vm",
];

/// One actionable repository hygiene policy violation.
#[derive(Debug, PartialEq, Eq)]
pub struct HygieneViolation {
    path: Option<PathBuf>,
    line: Option<usize>,
    rule: &'static str,
    reason: String,
}

impl HygieneViolation {
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
    fn new(message: impl Into<String>) -> Self {
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

        if is_python_artifact(relative) {
            report.violations.push(HygieneViolation {
                path: Some(relative.clone()),
                line: None,
                rule: RULE_PYTHON_ARTIFACT,
                reason: "tracked project-owned Python, notebook, package, or environment artifacts are prohibited; replace the maintained operation with Rust/Cargo tooling and remove this file".to_owned(),
            });
        }

        if !file_metadata.file_type().is_file() {
            continue;
        }

        let manifest = is_cargo_manifest(relative);
        let operational = is_potential_operational_surface(relative);
        if !manifest && !operational {
            continue;
        }

        let content = fs::read_to_string(&absolute).map_err(|error| {
            HygieneError::new(format!(
                "could not read maintained text surface {}: {error}",
                relative.display()
            ))
        })?;

        if manifest {
            scan_manifest(relative, &content, &mut report);
        }
        if operational && !is_historical_explanation(relative, &content) {
            scan_operational_invocations(relative, &content, &mut report);
        }
    }

    scan_selected_graph(metadata, &mut report);
    Ok(report)
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

fn is_cargo_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "Cargo.toml")
}

fn is_potential_operational_surface(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "rs"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "toml"
            | "yml"
            | "yaml"
            | "nix"
            | "slint"
    ) || matches!(file_name, "Makefile" | "Justfile" | "Dockerfile")
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("scripts" | "tools" | "build" | "release" | "packaging")
            )
        })
}

fn is_historical_explanation(path: &Path, content: &str) -> bool {
    let in_docs = path.starts_with("docs");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();

    if in_docs
        && (path
            .components()
            .any(|component| component.as_os_str() == "history")
            || matches!(file_name, "history.md" | "analyzer.md")
            || (file_name.contains("cleanup") && file_name.contains("brief")))
    {
        return true;
    }

    path.starts_with("docs/agent/decisions")
        && content.lines().take(12).any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("status:") && lower.contains("superseded")
        })
}

fn scan_manifest(path: &Path, content: &str, report: &mut HygieneReport) {
    let mut dependency_table = None;
    let mut reported = BTreeSet::new();

    for (index, original_line) in content.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_toml_comment(original_line).trim();
        if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
            let header = line.trim_start_matches('[').trim_end_matches(']').trim();
            dependency_table = dependency_table_kind(header);
            if let Some(DependencyTable::Detail(alias)) = &dependency_table
                && let Some(package) = forbidden_package(alias)
            {
                push_manifest_violation(path, line_number, package, &mut reported, report);
            }
            continue;
        }

        let Some(table) = &dependency_table else {
            continue;
        };
        if line.is_empty() {
            continue;
        }

        if matches!(table, DependencyTable::Map)
            && let Some((key, _)) = line.split_once('=')
            && let Some(package) = forbidden_package(key)
        {
            push_manifest_violation(path, line_number, package, &mut reported, report);
        }

        if let Some(package_name) = package_override(line)
            && let Some(package) = forbidden_package(package_name)
        {
            push_manifest_violation(path, line_number, package, &mut reported, report);
        }
    }
}

#[derive(Debug)]
enum DependencyTable {
    Map,
    Detail(String),
}

fn dependency_table_kind(header: &str) -> Option<DependencyTable> {
    let normalized = header.replace(['\'', '"'], "");
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if normalized == section {
            return Some(DependencyTable::Map);
        }
        if let Some(suffix) = normalized.strip_prefix(&format!("{section}.")) {
            return Some(DependencyTable::Detail(clean_toml_key(suffix)));
        }
        if let Some(workspace) = normalized.strip_prefix("workspace.") {
            if workspace == section {
                return Some(DependencyTable::Map);
            }
            if let Some(suffix) = workspace.strip_prefix(&format!("{section}.")) {
                return Some(DependencyTable::Detail(clean_toml_key(suffix)));
            }
        }
        if normalized.starts_with("target.") {
            let marker = format!(".{section}");
            if let Some(position) = normalized.rfind(&marker) {
                let suffix = &normalized[position + marker.len()..];
                if suffix.is_empty() {
                    return Some(DependencyTable::Map);
                }
                if let Some(alias) = suffix.strip_prefix('.') {
                    return Some(DependencyTable::Detail(clean_toml_key(alias)));
                }
            }
        }
    }
    None
}

fn strip_toml_comment(line: &str) -> &str {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if double_quoted => escaped = true,
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            '#' if !single_quoted && !double_quoted => return &line[..index],
            _ => {}
        }
    }
    line
}

fn clean_toml_key(key: &str) -> String {
    key.trim()
        .trim_matches(['\'', '"'])
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn package_override(line: &str) -> Option<&str> {
    for (position, _) in line.match_indices("package") {
        let preceding = line[..position].chars().next_back();
        if preceding.is_some_and(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
        }) {
            continue;
        }
        let remainder = line[position + "package".len()..].trim_start();
        let Some(value) = remainder.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let Some(quote) = value.chars().next() else {
            continue;
        };
        if !matches!(quote, '\'' | '"') {
            continue;
        }
        let quoted = &value[quote.len_utf8()..];
        let Some(end) = quoted.find(quote) else {
            continue;
        };
        return Some(&quoted[..end]);
    }
    None
}

fn forbidden_package(name: &str) -> Option<&'static str> {
    let canonical = clean_toml_key(name);
    FORBIDDEN_PACKAGES
        .iter()
        .copied()
        .find(|package| *package == canonical)
}

fn push_manifest_violation(
    path: &Path,
    line: usize,
    package: &'static str,
    reported: &mut BTreeSet<(usize, &'static str)>,
    report: &mut HygieneReport,
) {
    if !reported.insert((line, package)) {
        return;
    }
    report.violations.push(HygieneViolation {
        path: Some(path.to_path_buf()),
        line: Some(line),
        rule: RULE_MANIFEST_DEPENDENCY,
        reason: format!(
            "direct Cargo declaration selects forbidden package `{package}`; remove the declaration and regenerate Cargo.lock through Cargo"
        ),
    });
}

fn scan_selected_graph(metadata: &Metadata, report: &mut HygieneReport) {
    let Some(resolve) = &metadata.resolve else {
        return;
    };
    let mut forbidden = BTreeSet::new();

    for node in &resolve.nodes {
        if let Some(package) = metadata
            .packages
            .iter()
            .find(|package| package.id == node.id)
            && let Some(name) = forbidden_package(package.name.as_ref())
        {
            forbidden.insert(name);
        }
    }

    for package in forbidden {
        report.violations.push(HygieneViolation {
            path: None,
            line: None,
            rule: RULE_SELECTED_GRAPH,
            reason: format!(
                "locked selected Cargo graph contains forbidden package `{package}`; remove the selecting dependency and regenerate Cargo.lock through Cargo"
            ),
        });
    }
}

fn scan_operational_invocations(path: &Path, content: &str, report: &mut HygieneReport) {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let matches = if extension.eq_ignore_ascii_case("md") {
        markdown_invocations(content)
    } else {
        text_surface_invocations(path, content)
    };

    for (line, command) in matches {
        report.violations.push(HygieneViolation {
            path: Some(path.to_path_buf()),
            line: Some(line),
            rule: RULE_OPERATIONAL_INVOCATION,
            reason: format!(
                "maintained operational surface invokes prohibited `{command}` tooling; replace it with a Rust/Cargo-native command"
            ),
        });
    }
}

fn markdown_invocations(content: &str) -> Vec<(usize, String)> {
    let mut matches = Vec::new();
    let mut in_fence = false;
    let mut negative_fence = false;
    let mut previous_nonempty = "";

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_fence {
                in_fence = false;
                negative_fence = false;
            } else {
                in_fence = true;
                negative_fence = is_negative_policy_line(previous_nonempty);
            }
            continue;
        }

        if in_fence {
            if !negative_fence {
                collect_line_invocations(trimmed, line_number, &mut matches);
            }
        } else if !is_negative_policy_line(trimmed) && is_instructional_line(trimmed) {
            for code_span in inline_code_spans(trimmed) {
                collect_line_invocations(code_span, line_number, &mut matches);
            }
        }

        if !trimmed.is_empty() {
            previous_nonempty = trimmed;
        }
    }

    matches
}

fn text_surface_invocations(path: &Path, content: &str) -> Vec<(usize, String)> {
    let mut matches = Vec::new();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let shell_surface = matches!(
        extension.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "nix"
    ) || path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "Makefile" | "Justfile" | "Dockerfile"));
    let config_surface = matches!(
        extension.to_ascii_lowercase().as_str(),
        "toml" | "yml" | "yaml"
    );
    let source_surface = matches!(extension.to_ascii_lowercase().as_str(), "rs" | "slint");

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if line_number == 1
            && trimmed.starts_with("#!")
            && let Some(command) = prohibited_shell_command(trimmed.trim_start_matches("#!"))
        {
            matches.push((line_number, command));
        }
        if shell_surface {
            collect_line_invocations(trimmed, line_number, &mut matches);
        }
        if config_surface {
            if let Some(command_line) = configured_command(trimmed) {
                collect_line_invocations(command_line, line_number, &mut matches);
            } else if trimmed.starts_with(|character: char| {
                character.is_ascii_alphabetic() || matches!(character, '$' | '/' | '.')
            }) {
                collect_line_invocations(trimmed, line_number, &mut matches);
            }
        }
        if source_surface && let Some(command) = source_command_constructor(trimmed) {
            matches.push((line_number, command));
        }
    }

    matches
}

fn collect_line_invocations(line: &str, line_number: usize, matches: &mut Vec<(usize, String)>) {
    if let Some(command_line) = configured_command(line) {
        if let Some(command) = prohibited_shell_command(command_line) {
            matches.push((line_number, command));
        }
    } else if let Some(command) = prohibited_shell_command(line) {
        matches.push((line_number, command));
    }
    if let Some(command) = source_command_constructor(line) {
        matches.push((line_number, command));
    }
}

fn configured_command(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches(['-', ' ']).trim_start();
    for key in ["run", "command", "script", "entrypoint", "shell"] {
        let Some(remainder) = trimmed.strip_prefix(key) else {
            continue;
        };
        let remainder = remainder.trim_start();
        let Some(value) = remainder
            .strip_prefix(':')
            .or_else(|| remainder.strip_prefix('='))
        else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || matches!(value, "|" | ">" | "|-" | ">-") {
            return None;
        }
        return Some(value.trim_matches(['\'', '"']));
    }
    None
}

fn source_command_constructor(line: &str) -> Option<String> {
    for marker in ["Command::new(", "cmd!("] {
        let Some(position) = line.find(marker) else {
            continue;
        };
        let argument = line[position + marker.len()..].trim_start();
        let Some(literal) = first_string_literal(argument) else {
            continue;
        };
        if let Some(command) = prohibited_executable(literal) {
            return Some(command);
        }
    }
    None
}

fn first_string_literal(value: &str) -> Option<&str> {
    let value = value.strip_prefix('r').unwrap_or(value);
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let quoted = &value[quote.len_utf8()..];
    let end = quoted.find(quote)?;
    Some(&quoted[..end])
}

fn prohibited_shell_command(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || (line.starts_with('#') && !line.starts_with("#!")) {
        return None;
    }

    for segment in line.split([';', '|', '&']) {
        let words = segment
            .trim()
            .trim_start_matches("#!")
            .trim_start_matches(['$', '>', ' '])
            .split_ascii_whitespace();

        for word in words {
            let cleaned = clean_shell_word(word);
            if cleaned.is_empty() || is_environment_assignment(&cleaned) {
                continue;
            }
            if matches!(
                cleaned.as_str(),
                "!" | "if"
                    | "then"
                    | "elif"
                    | "do"
                    | "sudo"
                    | "env"
                    | "command"
                    | "exec"
                    | "time"
                    | "nohup"
                    | "RUN"
            ) || cleaned.starts_with('-')
            {
                continue;
            }
            if let Some(command) = prohibited_executable(&cleaned) {
                return Some(command);
            }
            break;
        }
    }
    None
}

fn clean_shell_word(word: &str) -> String {
    let unquoted = word.trim_matches(|character: char| {
        matches!(
            character,
            '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ':' | '\\'
        )
    });
    unquoted
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(unquoted)
        .to_owned()
}

fn is_environment_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn prohibited_executable(executable: &str) -> Option<String> {
    let name = clean_shell_word(executable).to_ascii_lowercase();
    let python_version = name.strip_prefix("python").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().any(|character| character.is_ascii_digit())
            && suffix
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
    });
    let pip_version = name.strip_prefix("pip").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().any(|character| character.is_ascii_digit())
            && suffix
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
    });

    if name == "python"
        || python_version
        || name == "python-config"
        || name == "pip"
        || pip_version
        || matches!(
            name.as_str(),
            "pipx" | "uv" | "conda" | "poetry" | "pytest" | "maturin" | "hf" | "huggingface-cli"
        )
    {
        Some(name)
    } else {
        None
    }
}

fn is_negative_policy_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "do not",
        "don't",
        "does not",
        "must not",
        "mustn't",
        "never",
        "forbid",
        "prohibit",
        "reject",
        "disallow",
        "not require",
        "without python",
        "no python",
    ]
    .into_iter()
    .any(|phrase| lower.contains(phrase))
}

fn is_instructional_line(line: &str) -> bool {
    [
        "run", "invoke", "execute", "install", "use", "call", "launch", "require",
    ]
    .into_iter()
    .any(|word| contains_ascii_word(line, word))
}

fn contains_ascii_word(value: &str, word: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.match_indices(word).any(|(position, _)| {
        let before = lower[..position].chars().next_back();
        let after = lower[position + word.len()..].chars().next();
        before.is_none_or(|character| !is_word_character(character))
            && after.is_none_or(|character| !is_word_character(character))
    })
}

const fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn inline_code_spans(line: &str) -> Vec<&str> {
    line.split('`')
        .enumerate()
        .filter_map(|(index, part)| (index % 2 == 1 && !part.is_empty()).then_some(part))
        .collect()
}
