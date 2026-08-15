use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use cargo_metadata::{Metadata, Package};

use super::METADATA_NAMESPACE;

const BENCHMARK_TARGETS_KEY: &str = "benchmark-targets";

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
