use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use cargo_metadata::{Dependency, Package};

use super::exceptions::ExceptionRegistry;
use super::policy::{
    Layer, RULE_CUDA_BOUNDARY, RULE_CUDA_CONTRACT, RULE_CUDA_DEFAULT, RULE_CUDA_HARDWARE,
    RULE_CUDA_PROHIBITED, dependency_kind,
};
use super::report::{ValidationReport, Violation};
use crate::workspace::cuda_provider;

const HARDWARE_FEATURE: &str = "cuda-hardware-tests";
const HARDWARE_TARGET: &str = "cuda_hardware";

pub(super) fn validate_cuda_policy(
    packages: &[&Package],
    packages_by_name: &BTreeMap<String, &Package>,
    roles: &BTreeMap<String, Layer>,
    exceptions: &ExceptionRegistry,
    report: &mut ValidationReport,
) {
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

        validate_default_feature(report, package, role);
        validate_prohibited_features(report, package, role);
        validate_hardware_suite(report, package, role);
        validate_direct_dependency_features(report, package, role);

        if provider {
            validate_provider(report, package, role);
        } else {
            validate_forwarding_source(report, package, role, packages_by_name, exceptions);
        }
        validate_cuda_references(report, package, role, provider, exceptions);
    }
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
        .filter(|dependency| dependency.name == "cudarc" && dependency.rename.is_none())
        .collect::<Vec<_>>();
    let exact = matching.len() == 1
        && matching.iter().all(|dependency| {
            dependency.path.is_none()
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
    let feature = package.features.get(HARDWARE_FEATURE);
    let targets = package
        .targets
        .iter()
        .filter(|target| target.name == HARDWARE_TARGET)
        .collect::<Vec<_>>();
    if feature.is_none() && targets.is_empty() {
        return;
    }

    if feature.is_none_or(|values| {
        values.len() != 1 || values.first().is_none_or(|value| value != "cuda")
    }) {
        push_feature_violation(
            report,
            package,
            role,
            HARDWARE_FEATURE,
            RULE_CUDA_HARDWARE,
            "cuda-hardware-tests must be a package-local non-default alias containing exactly `cuda`"
                .to_owned(),
        );
    }

    let expected_path = package
        .manifest_path
        .parent()
        .map(|directory| directory.join("tests/cuda_hardware.rs"));
    let metadata_target_is_exact = targets.len() == 1
        && targets.iter().all(|target| {
            target.is_test()
                && target.required_features == [HARDWARE_FEATURE]
                && expected_path
                    .as_ref()
                    .is_some_and(|expected| &target.src_path == expected)
        });
    let manifest_target_is_exact =
        exact_hardware_manifest_target(package).unwrap_or_else(|reason| {
            push_feature_violation(
                report,
                package,
                role,
                HARDWARE_TARGET,
                RULE_CUDA_HARDWARE,
                reason,
            );
            false
        });
    let exact_target = metadata_target_is_exact && manifest_target_is_exact;
    if !exact_target {
        push_feature_violation(
            report,
            package,
            role,
            HARDWARE_TARGET,
            RULE_CUDA_HARDWARE,
            "the CUDA hardware suite must be one explicit harness-free [[test]] named cuda_hardware at tests/cuda_hardware.rs with required-features = [\"cuda-hardware-tests\"]"
                .to_owned(),
        );
    }
}

fn exact_hardware_manifest_target(package: &Package) -> Result<bool, String> {
    let content = fs::read_to_string(package.manifest_path.as_std_path()).map_err(|error| {
        format!("could not read package manifest to verify harness = false: {error}")
    })?;
    let manifest = toml::from_str::<toml::Table>(&content).map_err(|error| {
        format!("could not parse package manifest to verify harness = false: {error}")
    })?;
    let Some(tests) = manifest.get("test").and_then(toml::Value::as_array) else {
        return Ok(false);
    };
    let matching = tests
        .iter()
        .filter_map(toml::Value::as_table)
        .filter(|test| test.get("name").and_then(toml::Value::as_str) == Some(HARDWARE_TARGET))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Ok(false);
    }
    let test = matching.first().copied().ok_or_else(|| {
        "internal hardware target selection did not retain its sole entry".to_owned()
    })?;
    let required_features = test
        .get("required-features")
        .and_then(toml::Value::as_array)
        .is_some_and(|features| {
            features.len() == 1
                && features
                    .first()
                    .and_then(toml::Value::as_str)
                    .is_some_and(|feature| feature == HARDWARE_FEATURE)
        });
    Ok(
        test.get("path").and_then(toml::Value::as_str) == Some("tests/cuda_hardware.rs")
            && test.get("harness").and_then(toml::Value::as_bool) == Some(false)
            && required_features,
    )
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
            reference == "cuda"
                || reference.ends_with("/cuda")
                || (package.features.contains_key(reference)
                    && feature_reaches_cuda(package, reference, visited))
        })
    })
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
