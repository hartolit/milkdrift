use std::collections::BTreeSet;
use std::path::Path;

use cargo_metadata::Metadata;

use super::orchestration::{HygieneReport, HygieneViolation};

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

pub(super) fn is_cargo_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "Cargo.toml")
}

pub(super) fn scan_manifest(path: &Path, content: &str, report: &mut HygieneReport) {
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
    report.push(HygieneViolation::new(
        Some(path.to_path_buf()),
        Some(line),
        RULE_MANIFEST_DEPENDENCY,
        format!(
            "direct Cargo declaration selects forbidden package `{package}`; remove the declaration and regenerate Cargo.lock through Cargo"
        ),
    ));
}

pub(super) fn scan_selected_graph(metadata: &Metadata, report: &mut HygieneReport) {
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
        report.push(HygieneViolation::new(
            None,
            None,
            RULE_SELECTED_GRAPH,
            format!(
                "locked selected Cargo graph contains forbidden package `{package}`; remove the selecting dependency and regenerate Cargo.lock through Cargo"
            ),
        ));
    }
}
