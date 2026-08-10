use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::{Dependency, Package};

use super::exceptions::ExceptionRegistry;
use super::policy::{
    Layer, RULE_CUDA_BOUNDARY, RULE_CUDA_CONTRACT, RULE_CUDA_DEFAULT, RULE_CUDA_HARDWARE,
    RULE_CUDA_PROHIBITED, dependency_kind,
};
use super::report::{ValidationReport, Violation};
use crate::workspace::{
    CUDA_HARDWARE_FEATURE as HARDWARE_FEATURE, cuda_provider, inspect_cuda_hardware_target,
};

pub(super) fn validate_cuda_policy(
    packages: &[&Package],
    packages_by_name: &BTreeMap<String, &Package>,
    roles: &BTreeMap<String, Layer>,
    exceptions: &ExceptionRegistry,
    report: &mut ValidationReport,
) {
    let mut providers = Vec::new();
    let mut has_cuda_topology = false;

    for package in packages {
        let role = roles.get(package.name.as_ref()).copied();
        let provider = match cuda_provider(package) {
            Ok(provider) => provider,
            Err(reason) => {
                push_feature_violation(
                    report,
                    package,
                    role,
                    "cuda-provider",
                    RULE_CUDA_CONTRACT,
                    reason,
                );
                false
            }
        };
        has_cuda_topology |= provider || package_has_cuda_topology(package);

        validate_default_feature(report, package, role);
        validate_prohibited_features(report, package, role);
        validate_hardware_suite(report, package, role);
        validate_direct_dependency_features(report, package, role);

        if provider {
            providers.push(package.name.to_string());
            validate_provider(report, package, role);
        } else {
            validate_forwarding_source(report, package, role, packages_by_name, exceptions);
        }
        validate_cuda_references(report, package, role, provider, exceptions);
    }

    validate_provider_cardinality(report, has_cuda_topology, &providers);
}

fn package_has_cuda_topology(package: &Package) -> bool {
    package.features.contains_key("cuda")
        || package.features.contains_key(HARDWARE_FEATURE)
        || package
            .dependencies
            .iter()
            .any(|dependency| dependency.name == "cudarc")
        || package
            .features
            .values()
            .flatten()
            .any(|reference| reference_is_cuda_sensitive(reference))
}

fn validate_provider_cardinality(
    report: &mut ValidationReport,
    has_cuda_topology: bool,
    providers: &[String],
) {
    if !has_cuda_topology || providers.len() == 1 {
        return;
    }

    let provider_names = if providers.is_empty() {
        "none".to_owned()
    } else {
        providers.join(", ")
    };
    report.push(Violation::new(
        "workspace CUDA topology".to_owned(),
        "cuda-provider".to_owned(),
        None,
        None,
        None,
        RULE_CUDA_CONTRACT,
        format!(
            "a workspace with CUDA features or dependencies must declare exactly one cuda-provider; found {} ({provider_names})",
            providers.len()
        ),
    ));
}

fn validate_default_feature(report: &mut ValidationReport, package: &Package, role: Option<Layer>) {
    let mut visited = BTreeSet::new();
    if feature_reaches_cuda(package, "default", &mut visited) {
        push_feature_violation(
            report,
            package,
            role,
            "default",
            RULE_CUDA_DEFAULT,
            "no default feature graph may reach CUDA; hardware support must remain explicitly opt-in"
                .to_owned(),
        );
    }

    for feature in package.features.keys() {
        if matches!(feature.as_str(), "default" | "cuda" | HARDWARE_FEATURE) {
            continue;
        }
        let mut visited = BTreeSet::new();
        if feature_reaches_cuda(package, feature, &mut visited) {
            push_feature_violation(
                report,
                package,
                role,
                feature,
                RULE_CUDA_BOUNDARY,
                "CUDA may be exposed only as `cuda`; generic GPU and other feature aliases are denied"
                    .to_owned(),
            );
        }
    }
}

fn validate_provider(report: &mut ValidationReport, package: &Package, role: Option<Layer>) {
    if role != Some(Layer::Adapter) {
        push_feature_violation(
            report,
            package,
            role,
            "cuda-provider",
            RULE_CUDA_CONTRACT,
            "an intrinsic Candle CUDA provider must have the explicit adapter role".to_owned(),
        );
    }

    let required = BTreeSet::from([
        "dep:cudarc",
        "candle-core/cuda",
        "candle-nn/cuda",
        "candle-transformers/cuda",
    ]);
    let actual_values = package.features.get("cuda");
    let actual = actual_values
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != required || actual_values.map_or(0, Vec::len) != required.len() {
        push_feature_violation(
            report,
            package,
            role,
            "cuda",
            RULE_CUDA_CONTRACT,
            "a CUDA provider's `cuda` feature must contain exactly dep:cudarc and the three reviewed Candle cuda forwards".to_owned(),
        );
    }
    validate_provider_cudarc_activation(report, package, role);

    for target in ["candle-core", "candle-nn", "candle-transformers"] {
        let matching = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.name == target && dependency.rename.is_none())
            .collect::<Vec<_>>();
        if matching.len() != 1
            || matching.iter().any(|dependency| {
                dependency.path.is_some()
                    || dependency.uses_default_features
                    || dependency.optional
                    || !dependency.features.is_empty()
            })
        {
            push_feature_violation(
                report,
                package,
                role,
                target,
                RULE_CUDA_CONTRACT,
                "Candle CUDA dependencies must be exact unrenamed external dependencies with default features disabled and no directly selected features".to_owned(),
            );
        }
    }

    validate_cudarc_contract(report, package, role);
}

fn validate_provider_cudarc_activation(
    report: &mut ValidationReport,
    package: &Package,
    role: Option<Layer>,
) {
    let activations = package
        .features
        .iter()
        .flat_map(|(feature, references)| {
            references
                .iter()
                .filter(|reference| directly_activates_cudarc(reference))
                .map(move |reference| (feature.as_str(), reference.as_str()))
        })
        .collect::<Vec<_>>();
    if activations.as_slice() != [("cuda", "dep:cudarc")] {
        push_feature_violation(
            report,
            package,
            role,
            "cudarc activation",
            RULE_CUDA_CONTRACT,
            "the sole CUDA provider must activate cudarc exactly once, through one `dep:cudarc` entry in its exact `cuda` feature; default, alias, hardware, and `cudarc/*` activation paths are denied"
                .to_owned(),
        );
    }
}

fn validate_cudarc_contract(report: &mut ValidationReport, package: &Package, role: Option<Layer>) {
    let expected_features = BTreeSet::from([
        "cuda-version-from-build-system",
        "driver",
        "dynamic-linking",
        "std",
    ]);
    let matching = package
        .dependencies
        .iter()
        .filter(|dependency| dependency.name == "cudarc")
        .collect::<Vec<_>>();
    let exact = matching.len() == 1
        && matching.iter().all(|dependency| {
            dependency.rename.is_none()
                && dependency.path.is_none()
                && dependency.req.to_string() == "=0.19.8"
                && dependency.optional
                && !dependency.uses_default_features
                && dependency.features.len() == expected_features.len()
                && dependency
                    .features
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    == expected_features
        });
    if !exact {
        push_feature_violation(
            report,
            package,
            role,
            "cudarc",
            RULE_CUDA_CONTRACT,
            "cudarc must be one optional unrenamed external =0.19.8 dependency with default features disabled and exactly std, driver, cuda-version-from-build-system, and dynamic-linking enabled".to_owned(),
        );
    }
}

fn validate_forwarding_source(
    report: &mut ValidationReport,
    package: &Package,
    role: Option<Layer>,
    packages_by_name: &BTreeMap<String, &Package>,
    exceptions: &ExceptionRegistry,
) {
    let forwards = exceptions
        .cuda_forwards_from(package.name.as_ref())
        .collect::<Vec<_>>();
    let expected = forwards
        .iter()
        .map(|record| format!("{}/cuda", record.target))
        .collect::<BTreeSet<_>>();
    let actual_values = package.features.get("cuda");
    let actual = actual_values
        .into_iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();

    if (!expected.is_empty() || actual_values.is_some())
        && (actual != expected || actual_values.map_or(0, Vec::len) != expected.len())
    {
        push_feature_violation(
            report,
            package,
            role,
            "cuda",
            RULE_CUDA_BOUNDARY,
            "a forwarding `cuda` feature must contain exactly the dependency-feature references registered as cuda-forward exceptions"
                .to_owned(),
        );
    }

    for record in forwards {
        if !packages_by_name
            .get(&record.target)
            .is_some_and(|target| target.features.contains_key("cuda"))
        {
            push_feature_violation(
                report,
                package,
                role,
                &format!("{}/cuda", record.target),
                RULE_CUDA_BOUNDARY,
                "a reviewed CUDA forward must target a workspace package that declares `cuda`"
                    .to_owned(),
            );
        }
    }
}

fn validate_cuda_references(
    report: &mut ValidationReport,
    package: &Package,
    role: Option<Layer>,
    provider: bool,
    exceptions: &ExceptionRegistry,
) {
    for (feature, references) in &package.features {
        for reference in references {
            let Some((target, target_feature)) = reference.split_once('/') else {
                continue;
            };
            if target_feature == HARDWARE_FEATURE {
                push_feature_violation(
                    report,
                    package,
                    role,
                    reference,
                    RULE_CUDA_HARDWARE,
                    "cuda-hardware-tests is a package-local test alias and must never be forwarded through a dependency"
                        .to_owned(),
                );
                continue;
            }
            if target_feature != "cuda" {
                continue;
            }
            if provider
                && feature == "cuda"
                && matches!(target, "candle-core" | "candle-nn" | "candle-transformers")
            {
                continue;
            }

            let reviewed = package.dependencies.iter().any(|dependency| {
                dependency.rename.is_none()
                    && dependency.name == target
                    && dependency_kind(dependency.kind).is_some_and(|kind| {
                        feature == "cuda"
                            && exceptions.has_cuda_forward(package.name.as_ref(), target, kind)
                    })
            });
            if !reviewed {
                push_feature_violation(
                    report,
                    package,
                    role,
                    reference,
                    RULE_CUDA_BOUNDARY,
                    "dependency CUDA forwards require an exact source, target, kind, feature name, and nonempty rationale in the root exception registry"
                        .to_owned(),
                );
            }
        }
    }
}

fn validate_hardware_suite(report: &mut ValidationReport, package: &Package, role: Option<Layer>) {
    if let Err(issues) = inspect_cuda_hardware_target(package) {
        for issue in issues {
            push_feature_violation(
                report,
                package,
                role,
                &issue.target,
                RULE_CUDA_HARDWARE,
                issue.reason,
            );
        }
    }
}

fn validate_direct_dependency_features(
    report: &mut ValidationReport,
    package: &Package,
    role: Option<Layer>,
) {
    for dependency in &package.dependencies {
        if prohibited_cuda_feature(&dependency.name)
            || dependency
                .rename
                .as_deref()
                .is_some_and(prohibited_cuda_feature)
        {
            push_dependency_feature_violation(
                report,
                package,
                role,
                dependency,
                RULE_CUDA_PROHIBITED,
                "cuDNN, flash attention, and NCCL dependencies require a separate architecture decision",
            );
        }
        for feature in &dependency.features {
            if feature == "cuda" {
                push_dependency_feature_violation(
                    report,
                    package,
                    role,
                    dependency,
                    RULE_CUDA_BOUNDARY,
                    "dependencies may not enable CUDA directly; use an exact reviewed non-default feature forward",
                );
            }
            if feature == HARDWARE_FEATURE {
                push_dependency_feature_violation(
                    report,
                    package,
                    role,
                    dependency,
                    RULE_CUDA_HARDWARE,
                    "cuda-hardware-tests is local to its owning test target and may not be selected by dependencies",
                );
            }
            if prohibited_cuda_feature(feature) {
                push_dependency_feature_violation(
                    report,
                    package,
                    role,
                    dependency,
                    RULE_CUDA_PROHIBITED,
                    "cuDNN, flash attention, and NCCL require a separate architecture decision",
                );
            }
        }
    }
}

fn validate_prohibited_features(
    report: &mut ValidationReport,
    package: &Package,
    role: Option<Layer>,
) {
    for (feature, references) in &package.features {
        if prohibited_cuda_feature(feature)
            || references
                .iter()
                .any(|reference| prohibited_cuda_feature(reference))
        {
            push_feature_violation(
                report,
                package,
                role,
                feature,
                RULE_CUDA_PROHIBITED,
                "cuDNN, flash attention, and NCCL require a separate architecture decision and are denied in the current feature graph"
                    .to_owned(),
            );
        }
    }
}

fn feature_reaches_cuda(package: &Package, feature: &str, visited: &mut BTreeSet<String>) -> bool {
    if !visited.insert(feature.to_owned()) {
        return false;
    }
    package.features.get(feature).is_some_and(|references| {
        references.iter().any(|reference| {
            reference_is_cuda_sensitive(reference)
                || (package.features.contains_key(reference)
                    && feature_reaches_cuda(package, reference, visited))
        })
    })
}

fn reference_is_cuda_sensitive(reference: &str) -> bool {
    reference == "cuda" || reference.ends_with("/cuda") || directly_activates_cudarc(reference)
}

fn directly_activates_cudarc(reference: &str) -> bool {
    reference == "dep:cudarc" || reference.starts_with("cudarc/")
}

fn prohibited_cuda_feature(value: &str) -> bool {
    let feature = value.rsplit('/').next().unwrap_or(value);
    let feature = feature.strip_prefix("dep:").unwrap_or(feature);
    matches!(feature, "cudnn" | "flash-attn" | "nccl")
}

fn push_dependency_feature_violation(
    report: &mut ValidationReport,
    package: &Package,
    package_role: Option<Layer>,
    dependency: &Dependency,
    policy_rule: &'static str,
    reason: &'static str,
) {
    push_feature_violation(
        report,
        package,
        package_role,
        &dependency.name,
        policy_rule,
        reason.to_owned(),
    );
}

fn push_feature_violation(
    report: &mut ValidationReport,
    package: &Package,
    package_role: Option<Layer>,
    target: &str,
    policy_rule: &'static str,
    reason: String,
) {
    report.push(Violation::new(
        package.name.to_string(),
        target.to_owned(),
        None,
        package_role,
        None,
        policy_rule,
        reason,
    ));
}
