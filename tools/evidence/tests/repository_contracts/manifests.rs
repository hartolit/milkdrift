//! Cargo owns package identity; TOML supplies declarations that may be inactive on this host.

use std::{collections::BTreeSet, path::Path, process::Command};

use serde::Deserialize;
use toml::Value;

use super::{TestResult, collect_files, read, root};

#[derive(Deserialize)]
pub(super) struct Metadata {
    pub(super) packages: Vec<Package>,
    pub(super) workspace_members: Vec<String>,
}

#[derive(Deserialize)]
pub(super) struct Package {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) manifest_path: std::path::PathBuf,
    pub(super) dependencies: Vec<CargoDependency>,
}

#[derive(Deserialize)]
pub(super) struct CargoDependency {
    pub(super) name: String,
    pub(super) rename: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) target: Option<String>,
}

#[derive(Debug)]
pub(super) struct Dependency {
    pub(super) alias: String,
    pub(super) package: String,
    pub(super) kind: &'static str,
    pub(super) target: Option<String>,
    pub(super) inherited: bool,
    pub(super) features: BTreeSet<String>,
}

pub(super) fn metadata(repository: &Path) -> TestResult<Metadata> {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
        ])
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Cargo member discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub(super) fn dependencies(manifest: &Value, workspace: &Value) -> TestResult<Vec<Dependency>> {
    let mut result = Vec::new();
    dependency_tables(manifest, workspace, None, &mut result)?;
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for (target, table) in targets {
            dependency_tables(table, workspace, Some(target), &mut result)?;
        }
    }
    Ok(result)
}

fn dependency_tables(
    manifest: &Value,
    workspace: &Value,
    target: Option<&str>,
    result: &mut Vec<Dependency>,
) -> TestResult {
    for (section, kind) in [
        ("dependencies", "normal"),
        ("dev-dependencies", "dev"),
        ("build-dependencies", "build"),
    ] {
        let Some(table) = manifest.get(section) else {
            continue;
        };
        for (alias, declaration) in table
            .as_table()
            .ok_or("dependency section must be a table")?
        {
            let inherited = declaration.get("workspace").and_then(Value::as_bool) == Some(true);
            let base = if inherited {
                workspace
                    .get("workspace")
                    .and_then(|v| v.get("dependencies"))
                    .and_then(|v| v.get(alias))
                    .ok_or_else(|| format!("missing inherited dependency {alias}"))?
            } else {
                declaration
            };
            let package = base
                .get("package")
                .and_then(Value::as_str)
                .unwrap_or(alias)
                .to_owned();
            let mut features = BTreeSet::new();
            for value in [base, declaration] {
                if let Some(values) = value.get("features") {
                    for feature in values
                        .as_array()
                        .ok_or("dependency features must be an array")?
                    {
                        features.insert(feature.as_str().ok_or("feature must be text")?.to_owned());
                    }
                }
            }
            result.push(Dependency {
                alias: alias.clone(),
                package,
                kind,
                target: target.map(str::to_owned),
                inherited,
                features,
            });
        }
    }
    Ok(())
}

fn inheritance_errors(manifest: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if manifest
        .get("lints")
        .and_then(|v| v.get("workspace"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        errors.push("missing workspace lint inheritance".to_owned());
    }
    for field in [
        "version",
        "edition",
        "rust-version",
        "license",
        "repository",
    ] {
        if manifest
            .get("package")
            .and_then(|v| v.get(field))
            .and_then(|v| v.get("workspace"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            errors.push(format!("{field} must inherit workspace policy"));
        }
    }
    if manifest.get("workspace").is_some() {
        errors.push("unauthorized nested workspace".to_owned());
    }
    errors
}

#[test]
fn every_backend_manifest_is_a_cargo_member_and_inherits_policy() -> TestResult {
    let repository = root()?;
    let metadata = metadata(&repository)?;
    let workspace: Value = toml::from_str(&read(repository.join("Cargo.toml"))?)?;
    let members: BTreeSet<_> = metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
        .map(|p| p.manifest_path.clone())
        .collect();
    let mut paths = Vec::new();
    collect_files(&repository, &mut paths, &|path| {
        path.file_name().is_some_and(|v| v == "Cargo.toml")
    })?;
    let mut errors = Vec::new();
    for path in paths {
        if path == repository.join("Cargo.toml") {
            continue;
        }
        // Slotbook is deliberately independent at deployment, but remains a checked Cargo member.
        // No untracked nested workspace is currently authorized. New clients need an explicit review.
        if !members.contains(&path) {
            errors.push(format!("unrecognized backend manifest: {}", path.display()));
        }
        let manifest: Value = toml::from_str(&read(&path)?)?;
        for error in inheritance_errors(&manifest) {
            errors.push(format!("{}: {error}", path.display()));
        }
        for dependency in dependencies(&manifest, &workspace)? {
            if dependency.package.starts_with("milkdrift-") && !dependency.inherited {
                errors.push(format!(
                    "{}: internal {} {} dependency must inherit workspace version/path",
                    path.display(),
                    dependency.kind,
                    dependency.alias
                ));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

#[test]
fn dependency_parser_preserves_aliases_tables_kinds_and_targets() -> TestResult {
    let workspace: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        renamed = { package = "milkdrift-runtime", path = "runtime", features = ["test-support"] }
    "#,
    )?;
    let manifest: Value = toml::from_str(
        r#"
        [dependencies.renamed]
        workspace = true
        optional = true
        [build-dependencies]
        storage = { package = "milkdrift-redb-store", version = "1" }
        [target.'cfg(windows)'.dependencies.web]
        package = "reqwest"
        version = "1"
        [dev-dependencies]
        database = { package = "redb", version = "2" }
    "#,
    )?;
    let edges = dependencies(&manifest, &workspace)?;
    assert_eq!(edges.len(), 4);
    assert!(edges.iter().any(|e| e.package == "milkdrift-runtime"
        && e.inherited
        && e.features.contains("test-support")));
    assert!(
        edges
            .iter()
            .any(|e| e.package == "milkdrift-redb-store" && e.kind == "build")
    );
    assert!(
        edges
            .iter()
            .any(|e| e.package == "reqwest" && e.target.as_deref() == Some("cfg(windows)"))
    );
    assert!(edges.iter().any(|e| e.package == "redb" && e.kind == "dev"));
    let good: Value = toml::from_str(
        r#"
        [package]
        version.workspace = true
        edition = { workspace = true }
        rust-version.workspace = true
        license.workspace = true
        repository.workspace = true
        [lints]
        workspace = true # Formatting and comments do not change inheritance.
    "#,
    )?;
    assert!(inheritance_errors(&good).is_empty());
    let bad: Value = toml::from_str("[package]\nversion = '0.1.0'\n[workspace]\n")?;
    let errors = inheritance_errors(&bad).join("\n");
    assert!(
        errors.contains("lint inheritance")
            && errors.contains("nested workspace")
            && errors.contains("version")
    );
    Ok(())
}
