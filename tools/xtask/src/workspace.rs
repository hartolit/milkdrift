use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, MetadataCommand, Package};

pub(crate) const METADATA_NAMESPACE: &str = "milkdrift";
pub(crate) const CUDA_FEATURE: &str = "cuda";
pub(crate) const CUDA_HARDWARE_FEATURE: &str = "cuda-hardware-tests";
pub(crate) const CUDA_HARDWARE_TARGET: &str = "cuda_hardware";
const BENCHMARK_TARGETS_KEY: &str = "benchmark-targets";
const HARDWARE_SUITES_KEY: &str = "hardware-suites";

/// A package's explicit Milkdrift architecture role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Repository maintenance tooling, isolated from workspace-local packages.
    Tooling,
    /// An outer-only benchmark or evidence observer.
    BenchmarkObserver,
    /// F0 portable domain contracts and shared vocabulary.
    DomainFoundation,
    /// F1 portable domain algorithms and features.
    DomainFeature,
    /// Host and process platform infrastructure.
    Platform,
    /// Vendor, storage, model, or service integration.
    Adapter,
    /// E0 resource ownership and inference lifecycle.
    RuntimeFoundation,
    /// Independently stateful reusable orchestration below E1.
    RuntimeCapability,
    /// E1 application orchestration.
    RuntimeApplication,
    /// Process and presentation boundaries.
    Application,
}

impl Role {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "tooling" => Some(Self::Tooling),
            "benchmark-observer" => Some(Self::BenchmarkObserver),
            "domain-foundation" => Some(Self::DomainFoundation),
            "domain-feature" => Some(Self::DomainFeature),
            "platform" => Some(Self::Platform),
            "adapter" => Some(Self::Adapter),
            "runtime-foundation" => Some(Self::RuntimeFoundation),
            "runtime-capability" => Some(Self::RuntimeCapability),
            "runtime-application" => Some(Self::RuntimeApplication),
            "application" => Some(Self::Application),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Tooling => "tooling",
            Self::BenchmarkObserver => "benchmark-observer",
            Self::DomainFoundation => "domain-foundation",
            Self::DomainFeature => "domain-feature",
            Self::Platform => "platform",
            Self::Adapter => "adapter",
            Self::RuntimeFoundation => "runtime-foundation",
            Self::RuntimeCapability => "runtime-capability",
            Self::RuntimeApplication => "runtime-application",
            Self::Application => "application",
        }
    }

    pub(crate) const fn is_domain(self) -> bool {
        matches!(self, Self::DomainFoundation | Self::DomainFeature)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub(crate) struct RoleMetadataError {
    pub(crate) reason: String,
}

pub(crate) fn package_role(package: &Package) -> Result<Role, RoleMetadataError> {
    let Some(namespace) = package.metadata.get(METADATA_NAMESPACE) else {
        return Err(RoleMetadataError {
            reason: format!(
                "missing mandatory [package.metadata.{METADATA_NAMESPACE}] role declaration"
            ),
        });
    };
    let Some(table) = namespace.as_object() else {
        return Err(RoleMetadataError {
            reason: format!(
                "[package.metadata.{METADATA_NAMESPACE}] must be a table containing a role string"
            ),
        });
    };
    let Some(value) = table.get("role") else {
        return Err(RoleMetadataError {
            reason: "missing mandatory role string".to_owned(),
        });
    };
    let Some(value) = value.as_str() else {
        return Err(RoleMetadataError {
            reason: "role must be a string".to_owned(),
        });
    };
    Role::parse(value).ok_or_else(|| RoleMetadataError {
        reason: format!(
            "unknown role `{value}`; roles fail closed and must use one of the ten documented Milkdrift role names"
        ),
    })
}

pub(crate) fn cuda_provider(package: &Package) -> Result<bool, String> {
    let Some(namespace) = package.metadata.get(METADATA_NAMESPACE) else {
        return Ok(false);
    };
    let Some(table) = namespace.as_object() else {
        return Ok(false);
    };
    let Some(value) = table.get("cuda-provider") else {
        return Ok(false);
    };
    value.as_bool().ok_or_else(|| {
        format!("[package.metadata.{METADATA_NAMESPACE}].cuda-provider must be a boolean")
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceInventoryIssue {
    pub(crate) package: String,
    pub(crate) target: String,
    pub(crate) reason: String,
}

pub(crate) fn workspace_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        let package_name = package.name.to_string();
        if !inventory.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name.clone(),
                target: "package identity".to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
        if let Err(error) = package_role(package) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: "role".to_owned(),
                reason: error.reason,
            });
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: "native verification ownership".to_owned(),
            reason: "at least one workspace package must declare a Milkdrift role".to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HardwareRunner {
    HarnessFree,
    SerialLibtest,
}

impl HardwareRunner {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "harness-free" => Some(Self::HarnessFree),
            "serial-libtest" => Some(Self::SerialLibtest),
            _ => None,
        }
    }

    pub(crate) const fn execution_arguments(self) -> &'static [&'static str] {
        match self {
            Self::HarnessFree => &[],
            Self::SerialLibtest => &["--", "--nocapture", "--test-threads=1"],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HardwareSuite {
    profile: String,
    package: String,
    target: String,
    feature: String,
    runner: HardwareRunner,
}

impl HardwareSuite {
    pub(crate) fn package(&self) -> &str {
        &self.package
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn feature(&self) -> &str {
        &self.feature
    }

    pub(crate) const fn runner(&self) -> HardwareRunner {
        self.runner
    }
}

pub(crate) fn domain_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut package_names = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        let package_name = package.name.to_string();
        if !package_names.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name.clone(),
                target: "package identity".to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
        match package_role(package) {
            Ok(role) if role.is_domain() => {
                inventory.insert(package_name);
            }
            Ok(_) => {}
            Err(error) => issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: "role".to_owned(),
                reason: error.reason,
            }),
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: "portable domain ownership".to_owned(),
            reason: "at least one workspace package must declare a Milkdrift domain role"
                .to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}

pub(crate) fn cuda_feature_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata
        .workspace_packages()
        .into_iter()
        .filter(|package| package.features.contains_key(CUDA_FEATURE))
    {
        let package_name = package.name.to_string();
        if !inventory.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: CUDA_FEATURE.to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: CUDA_FEATURE.to_owned(),
            reason: "at least one workspace package must declare the exact `cuda` feature"
                .to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}

pub(crate) fn cuda_hardware_target_inventory(
    metadata: &Metadata,
) -> Result<Vec<HardwareSuite>, Vec<WorkspaceInventoryIssue>> {
    hardware_suite_inventory(metadata, CUDA_FEATURE)
}

pub(crate) fn hardware_suite_inventory(
    metadata: &Metadata,
    profile: &str,
) -> Result<Vec<HardwareSuite>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut identities = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        match inspect_hardware_suites(package) {
            Ok(suites) => {
                for suite in suites {
                    let identity = (
                        suite.profile.clone(),
                        suite.package.clone(),
                        suite.target.clone(),
                    );
                    if !identities.insert(identity) || !inventory.insert(suite.clone()) {
                        issues.push(WorkspaceInventoryIssue {
                            package: package.name.to_string(),
                            target: suite.target,
                            reason: "each profile/package/target hardware suite identity must be registered exactly once"
                                .to_owned(),
                        });
                    }
                }
            }
            Err(mut package_issues) => issues.append(&mut package_issues),
        }
    }

    if profile == CUDA_FEATURE {
        for package in metadata.workspace_packages() {
            match inspect_cuda_hardware_target(package) {
                Ok(Some(target)) => {
                    let registered = inventory.iter().any(|suite| {
                        suite.profile == target.profile
                            && suite.package == target.package
                            && suite.target == target.target
                            && suite.feature == target.feature
                            && suite.runner == target.runner
                    });
                    if !registered {
                        issues.push(WorkspaceInventoryIssue {
                            package: package.name.to_string(),
                            target: CUDA_HARDWARE_TARGET.to_owned(),
                            reason: "the conventional CUDA hardware target must have one matching hardware-suites registration".to_owned(),
                        });
                    }
                }
                Ok(None) => {}
                Err(mut package_issues) => issues.append(&mut package_issues),
            }
        }
    }

    let selected = inventory
        .into_iter()
        .filter(|suite| suite.profile == profile)
        .collect::<Vec<_>>();
    if selected.is_empty() && issues.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: profile.to_owned(),
            reason: format!(
                "unknown or empty hardware profile `{profile}`; profile names fail closed"
            ),
        });
    }

    if issues.is_empty() {
        Ok(selected)
    } else {
        Err(issues)
    }
}

fn inspect_hardware_suites(
    package: &Package,
) -> Result<Vec<HardwareSuite>, Vec<WorkspaceInventoryIssue>> {
    let Some(namespace) = package.metadata.get(METADATA_NAMESPACE) else {
        return Ok(Vec::new());
    };
    let Some(table) = namespace.as_object() else {
        return Ok(Vec::new());
    };
    let Some(value) = table.get(HARDWARE_SUITES_KEY) else {
        return Ok(Vec::new());
    };
    let Some(entries) = value.as_array() else {
        return Err(vec![WorkspaceInventoryIssue {
            package: package.name.to_string(),
            target: HARDWARE_SUITES_KEY.to_owned(),
            reason: "hardware-suites must be an array of suite tables".to_owned(),
        }]);
    };

    let mut suites = Vec::new();
    let mut issues = Vec::new();
    for entry in entries {
        match parse_hardware_suite(package, entry) {
            Ok(suite) => suites.push(suite),
            Err(issue) => issues.push(issue),
        }
    }

    if issues.is_empty() {
        Ok(suites)
    } else {
        Err(issues)
    }
}

fn parse_hardware_suite(
    package: &Package,
    value: &serde_json::Value,
) -> Result<HardwareSuite, WorkspaceInventoryIssue> {
    let entry = value.as_object().ok_or_else(|| {
        hardware_issue(
            package,
            HARDWARE_SUITES_KEY,
            "each hardware suite must be a table",
        )
    })?;
    let string_field = |field: &str| entry.get(field).and_then(serde_json::Value::as_str);
    let (Some(profile), Some(target), Some(feature), Some(runner_name)) = (
        string_field("profile"),
        string_field("target"),
        string_field("feature"),
        string_field("runner"),
    ) else {
        return Err(hardware_issue(
            package,
            HARDWARE_SUITES_KEY,
            "each hardware suite requires string profile, target, feature, and runner fields",
        ));
    };
    if profile.is_empty()
        || !profile
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(hardware_issue(
            package,
            profile,
            "hardware profile names must be non-empty lowercase ASCII identifiers",
        ));
    }
    let runner = HardwareRunner::parse(runner_name).ok_or_else(|| {
        hardware_issue(
            package,
            target,
            format!(
                "unknown hardware runner `{runner_name}`; expected harness-free or serial-libtest"
            ),
        )
    })?;
    if !package.features.contains_key(feature) {
        return Err(hardware_issue(
            package,
            target,
            format!("hardware suite feature `{feature}` is not declared by the package"),
        ));
    }
    validate_hardware_suite_target(package, target, feature, runner)
        .map_err(|reason| hardware_issue(package, target, reason))?;
    Ok(HardwareSuite {
        profile: profile.to_owned(),
        package: package.name.to_string(),
        target: target.to_owned(),
        feature: feature.to_owned(),
        runner,
    })
}

fn validate_hardware_suite_target(
    package: &Package,
    target: &str,
    feature: &str,
    runner: HardwareRunner,
) -> Result<(), String> {
    let matching_targets = package
        .targets
        .iter()
        .filter(|candidate| candidate.name == target && candidate.is_test())
        .collect::<Vec<_>>();
    let [matching_target] = matching_targets.as_slice() else {
        return Err(
            "hardware suite must name exactly one package-local Cargo test target".to_owned(),
        );
    };
    let required_features_match = matching_target.required_features.is_empty()
        || (matching_target.required_features.len() == 1
            && matching_target
                .required_features
                .first()
                .is_some_and(|required| required == feature));
    if !required_features_match
        || (runner == HardwareRunner::HarnessFree
            && matching_target.required_features.as_slice() != [feature])
    {
        return Err(format!(
            "hardware suite target required-features must be empty for an ordinary serial target or exactly [`{feature}`] for the registered activation feature"
        ));
    }
    let harness = manifest_test_harness(package, target)?;
    match (runner, harness) {
        (HardwareRunner::HarnessFree, true) => {
            Err("harness-free hardware suites must declare harness = false".to_owned())
        }
        (HardwareRunner::SerialLibtest, false) => {
            Err("serial-libtest hardware suites must use the standard test harness".to_owned())
        }
        _ => Ok(()),
    }
}

fn hardware_issue(
    package: &Package,
    target: impl Into<String>,
    reason: impl Into<String>,
) -> WorkspaceInventoryIssue {
    WorkspaceInventoryIssue {
        package: package.name.to_string(),
        target: target.into(),
        reason: reason.into(),
    }
}

fn manifest_test_harness(package: &Package, target: &str) -> Result<bool, String> {
    let content = fs::read_to_string(package.manifest_path.as_std_path())
        .map_err(|error| format!("could not read package manifest: {error}"))?;
    let manifest = toml::from_str::<toml::Table>(&content)
        .map_err(|error| format!("could not parse package manifest: {error}"))?;
    let matching = manifest
        .get("test")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .filter(|test| test.get("name").and_then(toml::Value::as_str) == Some(target))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [] => Ok(true),
        [test] => Ok(test
            .get("harness")
            .and_then(toml::Value::as_bool)
            .unwrap_or(true)),
        _ => Err(format!(
            "Cargo test target `{target}` must not have duplicate explicit declarations"
        )),
    }
}

pub(crate) fn inspect_cuda_hardware_target(
    package: &Package,
) -> Result<Option<HardwareSuite>, Vec<WorkspaceInventoryIssue>> {
    let feature = package.features.get(CUDA_HARDWARE_FEATURE);
    let targets = package
        .targets
        .iter()
        .filter(|target| target.name == CUDA_HARDWARE_TARGET)
        .collect::<Vec<_>>();
    if feature.is_none() && targets.is_empty() {
        return Ok(None);
    }

    let mut issues = Vec::new();
    if feature.is_none_or(|values| {
        values.len() != 1 || values.first().is_none_or(|value| value != CUDA_FEATURE)
    }) {
        issues.push(WorkspaceInventoryIssue {
            package: package.name.to_string(),
            target: CUDA_HARDWARE_FEATURE.to_owned(),
            reason: "cuda-hardware-tests must be a package-local non-default alias containing exactly `cuda`"
                .to_owned(),
        });
    }

    let expected_path = package
        .manifest_path
        .parent()
        .map(|directory| directory.join("tests/cuda_hardware.rs"));
    let metadata_target_is_exact = targets.len() == 1
        && targets.iter().all(|target| {
            target.is_test()
                && target.required_features == [CUDA_HARDWARE_FEATURE]
                && expected_path
                    .as_ref()
                    .is_some_and(|expected| &target.src_path == expected)
        });
    let manifest_target_is_exact =
        exact_cuda_hardware_manifest_target(package).unwrap_or_else(|reason| {
            issues.push(WorkspaceInventoryIssue {
                package: package.name.to_string(),
                target: CUDA_HARDWARE_TARGET.to_owned(),
                reason,
            });
            false
        });
    if !metadata_target_is_exact || !manifest_target_is_exact {
        issues.push(WorkspaceInventoryIssue {
            package: package.name.to_string(),
            target: CUDA_HARDWARE_TARGET.to_owned(),
            reason: "the CUDA hardware suite must be one explicit harness-free [[test]] named cuda_hardware at tests/cuda_hardware.rs with required-features = [\"cuda-hardware-tests\"]"
                .to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(Some(HardwareSuite {
            profile: CUDA_FEATURE.to_owned(),
            package: package.name.to_string(),
            target: CUDA_HARDWARE_TARGET.to_owned(),
            feature: CUDA_HARDWARE_FEATURE.to_owned(),
            runner: HardwareRunner::HarnessFree,
        }))
    } else {
        Err(issues)
    }
}

fn exact_cuda_hardware_manifest_target(package: &Package) -> Result<bool, String> {
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
        .filter(|test| test.get("name").and_then(toml::Value::as_str) == Some(CUDA_HARDWARE_TARGET))
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
                    .is_some_and(|feature| feature == CUDA_HARDWARE_FEATURE)
        });
    Ok(
        test.get("path").and_then(toml::Value::as_str) == Some("tests/cuda_hardware.rs")
            && test.get("harness").and_then(toml::Value::as_bool) == Some(false)
            && required_features,
    )
}

pub(crate) fn role_location_is_compatible(root: &Path, package: &Package, role: Role) -> bool {
    let manifest = package.manifest_path.as_std_path();
    let Ok(relative) = manifest.strip_prefix(root) else {
        return false;
    };
    if relative.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return false;
    }
    let Some(directory) = relative.parent() else {
        return false;
    };

    let expected_parent = match role {
        Role::Tooling => Path::new("tools"),
        Role::BenchmarkObserver => Path::new("benchmarks"),
        Role::DomainFoundation | Role::DomainFeature => Path::new("crates/domain"),
        Role::Platform => Path::new("crates/platform"),
        Role::Adapter => Path::new("crates/adapters"),
        Role::RuntimeFoundation | Role::RuntimeCapability | Role::RuntimeApplication => {
            Path::new("crates/runtime")
        }
        Role::Application => Path::new("crates/apps"),
    };

    directory
        .strip_prefix(expected_parent)
        .is_ok_and(|remainder| remainder.components().count() == 1)
}

pub(crate) fn relative_manifest(root: &Path, package: &Package) -> PathBuf {
    package
        .manifest_path
        .as_std_path()
        .strip_prefix(root)
        .unwrap_or(package.manifest_path.as_std_path())
        .to_path_buf()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaintainedBenchmark {
    package: String,
    target: String,
}

impl MaintainedBenchmark {
    pub(crate) fn new(package: String, target: String) -> Self {
        Self { package, target }
    }

    /// The owning Cargo package name.
    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    /// The exact Cargo bench target name.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BenchmarkRegistryIssue {
    pub(crate) package: String,
    pub(crate) target: String,
    pub(crate) reason: String,
}

pub(crate) fn benchmark_inventory(
    metadata: &Metadata,
) -> Result<Vec<MaintainedBenchmark>, Vec<BenchmarkRegistryIssue>> {
    let mut inventory = Vec::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        let actual = package
            .targets
            .iter()
            .filter(|target| target.is_bench())
            .map(|target| target.name.clone())
            .collect::<BTreeSet<_>>();
        let registered = registered_benchmark_names(package, &mut issues);
        let registered_set = registered.iter().cloned().collect::<BTreeSet<_>>();
        let targets_by_name = package.targets.iter().fold(
            BTreeMap::<String, Vec<&cargo_metadata::Target>>::new(),
            |mut targets, target| {
                targets.entry(target.name.clone()).or_default().push(target);
                targets
            },
        );

        for target in &registered {
            match targets_by_name.get(target) {
                None => issues.push(BenchmarkRegistryIssue {
                    package: package.name.to_string(),
                    target: target.clone(),
                    reason: "registered benchmark target does not exist in Cargo metadata"
                        .to_owned(),
                }),
                Some(targets) if !targets.iter().any(|target| target.is_bench()) => {
                    issues.push(BenchmarkRegistryIssue {
                        package: package.name.to_string(),
                        target: target.clone(),
                        reason: "registered target exists but is not a Cargo bench target"
                            .to_owned(),
                    });
                }
                Some(_) => match validate_benchmark_manifest_target(package, target) {
                    Ok(()) => inventory.push(MaintainedBenchmark::new(
                        package.name.to_string(),
                        target.clone(),
                    )),
                    Err(reason) => issues.push(BenchmarkRegistryIssue {
                        package: package.name.to_string(),
                        target: target.clone(),
                        reason,
                    }),
                },
            }
        }

        for target in actual.difference(&registered_set) {
            issues.push(BenchmarkRegistryIssue {
                package: package.name.to_string(),
                target: target.clone(),
                reason: format!(
                    "Cargo bench target is unregistered; add `{target}` to [package.metadata.{METADATA_NAMESPACE}].{BENCHMARK_TARGETS_KEY}"
                ),
            });
        }
    }

    inventory.sort();
    if issues.is_empty() {
        Ok(inventory)
    } else {
        Err(issues)
    }
}

fn validate_benchmark_manifest_target(package: &Package, target: &str) -> Result<(), String> {
    let content = fs::read_to_string(package.manifest_path.as_std_path()).map_err(|error| {
        format!("could not read package manifest to verify benchmark harness: {error}")
    })?;
    let manifest = toml::from_str::<toml::Table>(&content).map_err(|error| {
        format!("could not parse package manifest to verify benchmark harness: {error}")
    })?;
    let matching = manifest
        .get("bench")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .filter(|bench| bench.get("name").and_then(toml::Value::as_str) == Some(target))
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(
            "maintained benchmarks require exactly one explicit [[bench]] entry in the owning manifest"
                .to_owned(),
        );
    }
    if matching
        .first()
        .and_then(|bench| bench.get("harness"))
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        return Err("maintained Criterion bench targets must declare harness = false".to_owned());
    }
    Ok(())
}

fn registered_benchmark_names(
    package: &Package,
    issues: &mut Vec<BenchmarkRegistryIssue>,
) -> Vec<String> {
    let Some(namespace) = package.metadata.get(METADATA_NAMESPACE) else {
        return Vec::new();
    };
    let Some(table) = namespace.as_object() else {
        return Vec::new();
    };
    let Some(value) = table.get(BENCHMARK_TARGETS_KEY) else {
        return Vec::new();
    };
    let Some(values) = value.as_array() else {
        issues.push(BenchmarkRegistryIssue {
            package: package.name.to_string(),
            target: BENCHMARK_TARGETS_KEY.to_owned(),
            reason: "benchmark-targets must be an array of exact Cargo target names".to_owned(),
        });
        return Vec::new();
    };

    let mut registered = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let Some(target) = value.as_str() else {
            issues.push(BenchmarkRegistryIssue {
                package: package.name.to_string(),
                target: value.to_string(),
                reason: "every benchmark-targets entry must be a string".to_owned(),
            });
            continue;
        };
        if target.trim().is_empty() {
            issues.push(BenchmarkRegistryIssue {
                package: package.name.to_string(),
                target: target.to_owned(),
                reason: "benchmark target names must be nonempty".to_owned(),
            });
            continue;
        }
        if !seen.insert(target.to_owned()) {
            issues.push(BenchmarkRegistryIssue {
                package: package.name.to_string(),
                target: target.to_owned(),
                reason: "benchmark target registrations must be unique within their owning package"
                    .to_owned(),
            });
            continue;
        }
        registered.push(target.to_owned());
    }
    registered
}

pub(crate) fn load_metadata(
    manifest_path: &Path,
    no_deps: bool,
) -> Result<Metadata, cargo_metadata::Error> {
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest_path)
        .other_options(vec!["--locked".to_owned()]);
    if no_deps {
        command.no_deps();
    }
    if let Some(cargo) = env::var_os("CARGO") {
        command.cargo_path(cargo);
    }
    command.exec()
}
