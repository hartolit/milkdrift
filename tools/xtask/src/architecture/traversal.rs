use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};

use cargo_metadata::{Dependency, Metadata, MetadataCommand, Package};

use super::policy::{
    DependencyKind, Layer, PolicyFailure, RULE_BENCHMARK_BUILD, RULE_BENCHMARK_PACKAGE,
    RULE_BENCHMARK_PUBLISH, RULE_BENCHMARK_ROLE, RULE_CUDA_BOUNDARY, RULE_CUDA_DEFAULT,
    RULE_CUDA_PROHIBITED, RULE_KNOWN_KIND, RULE_KNOWN_LOCATION, RULE_LOCAL_TARGET,
    RULE_PLATFORM_ROLE, RULE_RUNTIME_ROLE, classify_manifest, dependency_kind, external_policy,
    is_direct_child, local_dependency_policy, policy_configuration_failures,
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
        if source_layer == Layer::Benchmark {
            validate_benchmark_package(&mut report, package, source_layer);
        }
        validate_cuda_feature_policy(&mut report, package, source_layer);

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

fn validate_cuda_feature_policy(report: &mut ValidationReport, package: &Package, layer: Layer) {
    validate_candle_cuda_feature(report, package, layer);
    validate_forwarded_cuda_features(report, package, layer);
    validate_direct_cuda_dependency_features(report, package, layer);
}

fn validate_candle_cuda_feature(report: &mut ValidationReport, package: &Package, layer: Layer) {
    if package.name != "candle-backend" {
        return;
    }
    let mut visited = BTreeSet::new();
    if feature_reaches_cuda(package, "default", &mut visited) {
        report.push(feature_violation(
            package,
            layer,
            "default",
            RULE_CUDA_DEFAULT,
            "candle-backend CUDA support must remain non-default so ordinary CPU builds never require a CUDA toolkit",
        ));
    }

    let required = BTreeSet::from([
        "candle-core/cuda",
        "candle-nn/cuda",
        "candle-transformers/cuda",
        "dep:cudarc",
    ]);
    let actual = package
        .features
        .get("cuda")
        .map(|features| features.iter().map(String::as_str).collect::<BTreeSet<_>>())
        .unwrap_or_default();
    if actual != required {
        report.push(feature_violation(
            package,
            layer,
            "cuda",
            RULE_CUDA_BOUNDARY,
            "candle-backend's cuda feature must enable exactly candle-core/cuda, candle-nn/cuda, candle-transformers/cuda, and the reviewed optional cudarc dependency",
        ));
    }
}

fn validate_forwarded_cuda_features(
    report: &mut ValidationReport,
    package: &Package,
    layer: Layer,
) {
    for (feature, references) in &package.features {
        if prohibited_cuda_feature(feature)
            || references
                .iter()
                .any(|reference| prohibited_cuda_feature(reference))
        {
            report.push(feature_violation(
                package,
                layer,
                feature,
                RULE_CUDA_PROHIBITED,
                "cuDNN, flash attention, and NCCL require a separate architecture decision and may not enter the current CUDA feature graph",
            ));
        }
        for reference in references {
            validate_forwarded_cuda_reference(report, package, layer, feature, reference);
        }
    }
}

fn validate_forwarded_cuda_reference(
    report: &mut ValidationReport,
    package: &Package,
    layer: Layer,
    feature: &str,
    reference: &str,
) {
    let Some((target, target_feature)) = reference.split_once('/') else {
        return;
    };
    if target_feature != "cuda" || package.name == "candle-backend" {
        return;
    }
    let production_dependency = package.dependencies.iter().any(|dependency| {
        dependency.name == target
            && matches!(
                dependency.kind,
                cargo_metadata::DependencyKind::Normal | cargo_metadata::DependencyKind::Build
            )
    });
    let reviewed_local_composition =
        package.name == "application-runtime" && target == "candle-backend" && feature == "cuda";
    if production_dependency && !reviewed_local_composition {
        report.push(feature_violation(
            package,
            layer,
            reference,
            RULE_CUDA_BOUNDARY,
            "only candle-backend and application-runtime's reviewed non-default local-composition feature may enable Candle CUDA in the production graph",
        ));
    }
}

fn validate_direct_cuda_dependency_features(
    report: &mut ValidationReport,
    package: &Package,
    layer: Layer,
) {
    for dependency in &package.dependencies {
        if dependency.features.iter().any(|feature| feature == "cuda")
            && package.name != "candle-backend"
            && matches!(
                dependency.kind,
                cargo_metadata::DependencyKind::Normal | cargo_metadata::DependencyKind::Build
            )
        {
            report.push(feature_violation(
                package,
                layer,
                &dependency.name,
                RULE_CUDA_BOUNDARY,
                "a production dependency may not enable CUDA unconditionally; CUDA must remain behind the reviewed non-default composition feature",
            ));
        }
        if dependency
            .features
            .iter()
            .any(|feature| prohibited_cuda_feature(feature))
        {
            report.push(feature_violation(
                package,
                layer,
                &dependency.name,
                RULE_CUDA_PROHIBITED,
                "cuDNN, flash attention, and NCCL require a separate architecture decision and may not enter direct dependency features",
            ));
        }
    }
}

fn feature_reaches_cuda(package: &Package, feature: &str, visited: &mut BTreeSet<String>) -> bool {
    if !visited.insert(feature.to_owned()) {
        return false;
    }
    package.features.get(feature).is_some_and(|references| {
        references.iter().any(|reference| {
            reference == "cuda"
                || reference.ends_with("/cuda")
                || (package.features.contains_key(reference)
                    && feature_reaches_cuda(package, reference, visited))
        })
    })
}

fn prohibited_cuda_feature(feature: &str) -> bool {
    let feature = feature.rsplit('/').next().unwrap_or(feature);
    let feature = feature.strip_prefix("dep:").unwrap_or(feature);
    matches!(feature, "cudnn" | "flash-attn" | "nccl")
}

fn feature_violation(
    package: &Package,
    layer: Layer,
    target: &str,
    rule: &'static str,
    reason: &'static str,
) -> Violation {
    Violation::new(
        package.name.to_string(),
        target.to_owned(),
        None,
        Some(layer),
        None,
        rule,
        reason.to_owned(),
    )
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
    let benchmark_location =
        package_directory.is_some_and(|directory| directory.starts_with("benchmarks"));
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
    } else if benchmark_location {
        (
            RULE_BENCHMARK_ROLE,
            "benchmarks/runtime is the only recognized cross-crate measurement package; unknown benchmark package paths fail closed",
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

fn validate_benchmark_package(report: &mut ValidationReport, package: &Package, layer: Layer) {
    if package.name != "runtime-benchmarks" {
        report.push(Violation::new(
            package.name.to_string(),
            package.manifest_path.to_string(),
            None,
            Some(layer),
            None,
            RULE_BENCHMARK_PACKAGE,
            "the package at benchmarks/runtime must be named runtime-benchmarks".to_owned(),
        ));
    }
    if package
        .publish
        .as_ref()
        .is_none_or(|registries| !registries.is_empty())
    {
        report.push(Violation::new(
            package.name.to_string(),
            package.manifest_path.to_string(),
            None,
            Some(layer),
            None,
            RULE_BENCHMARK_PUBLISH,
            "benchmark packages must declare publish = false".to_owned(),
        ));
    }
    if package
        .targets
        .iter()
        .any(cargo_metadata::Target::is_custom_build)
    {
        report.push(Violation::new(
            package.name.to_string(),
            package.manifest_path.to_string(),
            None,
            Some(layer),
            None,
            RULE_BENCHMARK_BUILD,
            "benchmark packages cannot declare a custom build target or build.rs".to_owned(),
        ));
    }
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

    let failure =
        local_dependency_policy(source_name, source_layer, target_name, target_layer, kind);

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
