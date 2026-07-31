use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use cargo_metadata::{Dependency, Metadata, MetadataCommand, Package};

use super::policy::{
    DependencyKind, Layer, PolicyFailure, RULE_KNOWN_KIND, RULE_KNOWN_LOCATION, RULE_LOCAL_TARGET,
    RULE_PLATFORM_ROLE, RULE_RUNTIME_ROLE, classify_manifest, dependency_kind, external_policy,
    is_direct_child, local_development_policy, local_production_policy,
    policy_configuration_failures,
};
use super::report::{ValidationError, ValidationReport, Violation};

/// Loads locked typed Cargo metadata and validates the workspace containing `manifest_path`.
///
/// The nested metadata command always uses `--locked` and `--no-deps`. Direct declarations are
/// sufficient for enforcing package boundaries and avoid conflating transitive vendor graphs with
/// workspace architecture.
///
/// # Errors
///
/// Returns an error if Cargo cannot produce or `cargo_metadata` cannot parse locked metadata.
pub fn validate_workspace(manifest_path: &Path) -> Result<ValidationReport, ValidationError> {
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest_path)
        .no_deps()
        .other_options(vec!["--locked".to_owned()]);
    if let Some(cargo) = env::var_os("CARGO") {
        command.cargo_path(cargo);
    }

    let metadata = command.exec()?;
    Ok(validate_metadata(&metadata))
}

fn validate_metadata(metadata: &Metadata) -> ValidationReport {
    let root = metadata.workspace_root.as_std_path();
    let packages = metadata.workspace_packages();
    let package_locations = packages
        .iter()
        .filter_map(|package| {
            package.manifest_path.parent().map(|directory| {
                (
                    directory.as_std_path().to_path_buf(),
                    (
                        package.name.to_string(),
                        classify_manifest(root, package.manifest_path.as_std_path()),
                    ),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut report = ValidationReport::default();

    for failure in policy_configuration_failures() {
        report.push(Violation::new(
            failure.source,
            failure.target,
            None,
            None,
            None,
            failure.rule,
            failure.reason,
        ));
    }

    for package in packages {
        let source_name = package.name.to_string();
        let Some(source_layer) = classify_manifest(root, package.manifest_path.as_std_path())
        else {
            report.push(unknown_package_location(package, root));
            continue;
        };

        for dependency in &package.dependencies {
            validate_dependency(
                &mut report,
                &source_name,
                source_layer,
                dependency,
                &package_locations,
            );
        }
    }

    report
}

fn unknown_package_location(package: &Package, root: &Path) -> Violation {
    let manifest = package.manifest_path.as_std_path();
    let relative = manifest.strip_prefix(root).unwrap_or(manifest);
    let package_directory = relative.parent();
    let runtime_location = package_directory
        .is_some_and(|directory| is_direct_child(directory, Path::new("crates/runtime")));
    let platform_location = package_directory
        .is_some_and(|directory| is_direct_child(directory, Path::new("crates/platform")));
    let tooling_location =
        package_directory.is_some_and(|directory| directory.starts_with("tools"));
    let (rule, reason) = if runtime_location {
        (
            RULE_RUNTIME_ROLE,
            "runtime crates require an explicitly classified E0, capability, or E1 role; directory placement does not grant a capability role",
        )
    } else if platform_location {
        (
            RULE_PLATFORM_ROLE,
            "platform crates require an explicitly classified host/platform role; directory placement does not grant infrastructure authority",
        )
    } else if tooling_location {
        (
            RULE_KNOWN_LOCATION,
            "tools/xtask is the only classified tooling package; unknown tools fail closed and require an explicit architecture review",
        )
    } else {
        (
            RULE_KNOWN_LOCATION,
            "workspace packages must be tools/xtask or an explicitly registered crate at an approved path under crates/domain, crates/platform, crates/adapters, crates/runtime, or crates/apps; unknown package names or locations never receive a fallback layer",
        )
    };

    Violation::new(
        package.name.to_string(),
        relative.display().to_string(),
        None,
        None,
        None,
        rule,
        reason.to_owned(),
    )
}

fn validate_dependency(
    report: &mut ValidationReport,
    source_name: &str,
    source_layer: Layer,
    dependency: &Dependency,
    package_locations: &BTreeMap<PathBuf, (String, Option<Layer>)>,
) {
    let Some(kind) = dependency_kind(dependency.kind) else {
        report.push(Violation::new(
            source_name.to_owned(),
            dependency.name.clone(),
            None,
            Some(source_layer),
            None,
            RULE_KNOWN_KIND,
            format!(
                "Cargo reported an unsupported dependency kind {:?}; unknown kinds fail closed",
                dependency.kind
            ),
        ));
        return;
    };

    if let Some(path) = dependency.path.as_ref() {
        validate_local_dependency(
            report,
            source_name,
            source_layer,
            kind,
            path.as_std_path(),
            package_locations,
        );
    } else if let Some(failure) = external_policy(source_name, source_layer, &dependency.name, kind)
    {
        report.push(edge_violation(
            source_name,
            Some(source_layer),
            &dependency.name,
            None,
            kind,
            failure,
        ));
    }
}

fn validate_local_dependency(
    report: &mut ValidationReport,
    source_name: &str,
    source_layer: Layer,
    kind: DependencyKind,
    dependency_path: &Path,
    package_locations: &BTreeMap<PathBuf, (String, Option<Layer>)>,
) {
    let Some((target_name, target_layer)) = package_locations.get(dependency_path) else {
        report.push(Violation::new(
            source_name.to_owned(),
            dependency_path.display().to_string(),
            Some(kind),
            Some(source_layer),
            None,
            RULE_LOCAL_TARGET,
            "path dependencies must resolve to a recognized member of this workspace; outside, excluded, and otherwise unknown local paths fail closed".to_owned(),
        ));
        return;
    };
    let Some(target_layer) = *target_layer else {
        report.push(Violation::new(
            source_name.to_owned(),
            target_name.clone(),
            Some(kind),
            Some(source_layer),
            None,
            RULE_LOCAL_TARGET,
            "the path dependency resolves to a workspace package whose location has no recognized architecture layer".to_owned(),
        ));
        return;
    };

    let failure = match kind {
        DependencyKind::Normal | DependencyKind::Build => {
            local_production_policy(source_name, source_layer, target_name, target_layer, kind)
        }
        DependencyKind::Development => local_development_policy(source_name, target_name, kind),
    };

    if let Some(failure) = failure {
        report.push(edge_violation(
            source_name,
            Some(source_layer),
            target_name,
            Some(target_layer),
            kind,
            failure,
        ));
    }
}

fn edge_violation(
    source: &str,
    source_layer: Option<Layer>,
    target: &str,
    target_layer: Option<Layer>,
    dependency_kind: DependencyKind,
    failure: PolicyFailure,
) -> Violation {
    Violation::new(
        source.to_owned(),
        target.to_owned(),
        Some(dependency_kind),
        source_layer,
        target_layer,
        failure.rule,
        failure.reason,
    )
}
