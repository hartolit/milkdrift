use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

pub(super) const RULE_KNOWN_LOCATION: &str = "LAYOUT-1";
pub(super) const RULE_LOCAL_TARGET: &str = "LAYOUT-2";
pub(super) const RULE_KNOWN_KIND: &str = "DEPENDENCY-KIND-1";
pub(super) const RULE_PLATFORM_ROLE: &str = "PLATFORM-ROLE-1";
pub(super) const RULE_PRODUCTION_DIRECTION: &str = "LAYER-PROD-1";
pub(super) const RULE_RUNTIME_ROLE: &str = "RUNTIME-ROLE-1";
pub(super) const RULE_BENCHMARK_ROLE: &str = "BENCHMARK-ROLE-1";
pub(super) const RULE_CUDA_DEFAULT: &str = "CUDA-DEFAULT-1";
pub(super) const RULE_CUDA_BOUNDARY: &str = "CUDA-BOUNDARY-1";
pub(super) const RULE_CUDA_PROHIBITED: &str = "CUDA-PROHIBITED-1";
pub(super) const RULE_BENCHMARK_PACKAGE: &str = "BENCHMARK-PACKAGE-1";
pub(super) const RULE_BENCHMARK_PUBLISH: &str = "BENCHMARK-PUBLISH-1";
pub(super) const RULE_BENCHMARK_BUILD: &str = "BENCHMARK-BUILD-1";
const RULE_BENCHMARK_REVERSE: &str = "BENCHMARK-REVERSE-1";
const RULE_BENCHMARK_LOCAL_REVIEW: &str = "BENCHMARK-LOCAL-1";
const RULE_BENCHMARK_EXTERNAL_REVIEW: &str = "EXT-BENCHMARK-1";
const RULE_DOMAIN_LOCAL_REVIEW: &str = "DOMAIN-LOCAL-PROD-1";
const RULE_ENGINE_LOCAL_REVIEW: &str = "ENGINE-LOCAL-PROD-1";
const RULE_LOCAL_DEV_REVIEW: &str = "DEV-LOCAL-1";
const RULE_EXTERNAL_DEV_REVIEW: &str = "EXT-DEV-1";
const RULE_TOOLING_EXTERNAL: &str = "EXT-TOOLING-PROD-1";
const RULE_F0_EXTERNAL: &str = "EXT-F0-PROD-1";
const RULE_F1_EXTERNAL: &str = "EXT-F1-PROD-1";
const RULE_ENGINE_EXTERNAL: &str = "EXT-ENGINE-PROD-1";
pub(super) const RULE_REVIEW_REGISTRY: &str = "POLICY-REVIEW-1";
const RULE_DOMAIN_DAG: &str = "DOMAIN-DAG-1";

/// A package role in the workspace architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// The explicitly registered workspace-maintenance tooling package.
    Tooling,
    /// A non-production measurement package that consumes public production APIs.
    Benchmark,
    /// F0 portable shared contracts.
    FeatureFoundation,
    /// F1 portable algorithms.
    FeatureAlgorithm,
    /// Host-platform, infrastructure, and vendor adapters.
    Adapter,
    /// E0 inference lifecycle orchestration.
    EngineFoundation,
    /// Independently stateful reusable capability orchestration below E1.
    EngineCapability,
    /// E1 application orchestration.
    EngineApplication,
    /// Process and presentation boundaries.
    Application,
}

/// The Cargo dependency section that declares an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyKind {
    /// A normal production dependency.
    Normal,
    /// A build-script dependency, governed by production direction.
    Build,
    /// A development-only dependency, governed by separate review policy.
    Development,
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => formatter.write_str("normal"),
            Self::Build => formatter.write_str("build"),
            Self::Development => formatter.write_str("development"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ReviewedDependency {
    source: &'static str,
    target: &'static str,
    kind: DependencyKind,
    rationale: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReviewedFeatureForward {
    pub(super) source_package: &'static str,
    pub(super) source_feature: &'static str,
    pub(super) target_package: &'static str,
    pub(super) target_feature: &'static str,
    pub(super) dependency_kind: DependencyKind,
    pub(super) rationale: &'static str,
}

// Every production edge wholly inside the domain layer is exact, reviewed, and acyclic. The
// coarse matrix permits future F1 peers, but this registry keeps them denied until reviewed.
const REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "tokenization",
        target: "domain-contracts",
        kind: DependencyKind::Normal,
        rationale: "tokenization implements the shared token and caller-owned buffer contracts",
    },
    ReviewedDependency {
        source: "context-planner",
        target: "domain-contracts",
        kind: DependencyKind::Normal,
        rationale: "context planning consumes shared request, sequence, and token vocabulary",
    },
    ReviewedDependency {
        source: "sampling",
        target: "domain-contracts",
        kind: DependencyKind::Normal,
        rationale: "sampling implements shared generation and stop-policy contracts",
    },
    ReviewedDependency {
        source: "task-graph",
        target: "domain-contracts",
        kind: DependencyKind::Normal,
        rationale: "task graphs use shared artifact and workflow identifiers",
    },
];

// Engine dependencies on infrastructure or another engine are exact reviewed composition edges.
const REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "inference-runtime",
        target: "host-runtime",
        kind: DependencyKind::Normal,
        rationale: "E0 uses the bounded host worker integration that runs its command loop",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "candle-backend",
        kind: DependencyKind::Normal,
        rationale: "the closed E1 local composition constructs the supported Candle/Safetensors source",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "hf-hub-adapter",
        kind: DependencyKind::Normal,
        rationale: "the closed E1 local composition resolves immutable Hugging Face artifacts for the Candle product",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "hf-tokenizer",
        kind: DependencyKind::Normal,
        rationale: "the closed E1 local composition owns Hugging Face prompt encoding and streaming decode",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "host-runtime",
        kind: DependencyKind::Normal,
        rationale: "E1 hosts bounded workers and frontend-facing output accumulation",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "inference-runtime",
        kind: DependencyKind::Normal,
        rationale: "E1 delegates current local model execution and lifecycle ownership to E0",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "redb-storage",
        kind: DependencyKind::Normal,
        rationale: "E1 persists application preferences and the Hugging Face model catalogue with redb",
    },
];

// External production exceptions and all external development dependencies are exact and reviewed.
const REVIEWED_EXTERNAL_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "xtask",
        target: "cargo_metadata",
        kind: DependencyKind::Normal,
        rationale: "workspace tooling requires Cargo's typed workspace metadata API",
    },
    ReviewedDependency {
        source: "sampling",
        target: "libm",
        kind: DependencyKind::Normal,
        rationale: "sampling requires reviewed portable floating-point math",
    },
    ReviewedDependency {
        source: "candle-backend",
        target: "cudarc",
        kind: DependencyKind::Normal,
        rationale: "the optional CUDA adapter uses Candle's exact cudarc version only for safe device identity, capability, memory discovery, and native OOM classification",
    },
    ReviewedDependency {
        source: "inference-runtime",
        target: "candle-core",
        kind: DependencyKind::Development,
        rationale: "download-free hosted E0 compatibility tests derive temporary mixed-dtype Safetensors fixtures from the project-authored F32 fixture through Candle's safe CPU conversion APIs",
    },
    ReviewedDependency {
        source: "domain-contracts",
        target: "stats_alloc",
        kind: DependencyKind::Development,
        rationale: "allocation contract tests measure project-owned hot paths",
    },
    ReviewedDependency {
        source: "sampling",
        target: "stats_alloc",
        kind: DependencyKind::Development,
        rationale: "sampling allocation tests measure the declared zero-allocation region",
    },
    ReviewedDependency {
        source: "sampling",
        target: "criterion",
        kind: DependencyKind::Development,
        rationale: "Criterion compiles and runs the reviewed sampling benchmark",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "serde",
        kind: DependencyKind::Normal,
        rationale: "the controlled baseline runner serializes a stable typed measurement record",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "serde_json",
        kind: DependencyKind::Normal,
        rationale: "the controlled baseline runner emits its stable measurement record as JSON",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "sha2",
        kind: DependencyKind::Normal,
        rationale: "the benchmark harness verifies exact fixture identity and hashes fixed external workload inputs without retaining generated output",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "criterion",
        kind: DependencyKind::Development,
        rationale: "Criterion runs repeatable hosted E0 prefill and incremental-decode measurements",
    },
];

// The benchmark observer consumes only the public boundaries exercised by implemented system and
// component-like measurements. No product or tooling package may depend back on this package.
const REVIEWED_BENCHMARK_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "application-runtime",
        kind: DependencyKind::Normal,
        rationale: "the benchmark runners observe download-free lifecycle checks and the sole external model/device/scalar/generation workflow through public E1 APIs",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "candle-backend",
        kind: DependencyKind::Normal,
        rationale: "the benchmark package constructs the reviewed fixture and independently plans the exact external model while using safe public Candle device observation",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "domain-contracts",
        kind: DependencyKind::Normal,
        rationale: "the benchmark package uses public model, request, sequence, scalar, device, cancellation, and accounting vocabulary for E0 and external evidence",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "host-runtime",
        kind: DependencyKind::Normal,
        rationale: "the synthetic harness interprets public pull-boundary token ranges and state records",
    },
    ReviewedDependency {
        source: "runtime-benchmarks",
        target: "inference-runtime",
        kind: DependencyKind::Normal,
        rationale: "the synthetic and Criterion harnesses measure hosted E0 lifecycle, snapshots, prefill, decode, cancellation, backpressure, unload, and shutdown",
    },
];

// Workspace-local development edges are reviewed independently from production direction.
const REVIEWED_LOCAL_DEV_DEPENDENCIES: &[ReviewedDependency] = &[ReviewedDependency {
    source: "inference-runtime",
    target: "candle-backend",
    kind: DependencyKind::Development,
    rationale: "E0 compatibility tests exercise the Candle backend contract",
}];

// CUDA propagation is opt-in and exact. Each entry binds one source feature to one target
// feature through a dependency whose Cargo kind is part of the review.
const REVIEWED_CUDA_FEATURE_FORWARDS: &[ReviewedFeatureForward] = &[
    ReviewedFeatureForward {
        source_package: "application-runtime",
        source_feature: "cuda",
        target_package: "candle-backend",
        target_feature: "cuda",
        dependency_kind: DependencyKind::Normal,
        rationale: "E1 exposes the selected Candle backend's CUDA support without enabling it by default",
    },
    ReviewedFeatureForward {
        source_package: "desktop-slint",
        source_feature: "cuda",
        target_package: "application-runtime",
        target_feature: "cuda",
        dependency_kind: DependencyKind::Normal,
        rationale: "the desktop product exposes E1 CUDA selection without bypassing application orchestration",
    },
    ReviewedFeatureForward {
        source_package: "inference-runtime",
        source_feature: "cuda",
        target_package: "candle-backend",
        target_feature: "cuda",
        dependency_kind: DependencyKind::Development,
        rationale: "E0 compatibility tests expose development-only Candle CUDA without changing the production graph",
    },
    ReviewedFeatureForward {
        source_package: "runtime-benchmarks",
        source_feature: "cuda",
        target_package: "application-runtime",
        target_feature: "cuda",
        dependency_kind: DependencyKind::Normal,
        rationale: "the sole device-parameterized external benchmark and CUDA compile checks reach Candle only through E1's exact non-default CUDA feature",
    },
];

pub(super) struct PolicyFailure {
    pub(super) rule: &'static str,
    pub(super) reason: String,
}

pub(super) struct PolicyConfigurationFailure {
    pub(super) rule: &'static str,
    pub(super) source: String,
    pub(super) target: String,
    pub(super) reason: String,
}

pub(super) fn policy_configuration_failures() -> Vec<PolicyConfigurationFailure> {
    let mut failures = Vec::new();
    review_table_failures(
        "domain production reviews",
        REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES,
        &mut failures,
    );
    review_table_failures(
        "engine production reviews",
        REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
        &mut failures,
    );
    review_table_failures(
        "external dependency reviews",
        REVIEWED_EXTERNAL_DEPENDENCIES,
        &mut failures,
    );
    review_table_failures(
        "benchmark dependency reviews",
        REVIEWED_BENCHMARK_DEPENDENCIES,
        &mut failures,
    );
    review_table_failures(
        "local development reviews",
        REVIEWED_LOCAL_DEV_DEPENDENCIES,
        &mut failures,
    );
    feature_forward_table_failures(
        "CUDA feature-forward reviews",
        REVIEWED_CUDA_FEATURE_FORWARDS,
        &mut failures,
    );

    if !reviewed_graph_is_acyclic(REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES) {
        failures.push(PolicyConfigurationFailure {
            rule: RULE_DOMAIN_DAG,
            source: "domain production reviews".to_owned(),
            target: "registered domain dependency graph".to_owned(),
            reason: "reviewed domain production dependencies must form a directed acyclic graph"
                .to_owned(),
        });
    }

    failures
}

fn review_table_failures(
    registry: &str,
    reviewed_dependencies: &[ReviewedDependency],
    failures: &mut Vec<PolicyConfigurationFailure>,
) {
    for (index, reviewed) in reviewed_dependencies.iter().enumerate() {
        if reviewed.rationale.trim().is_empty() {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: format!(
                    "{} --{}--> {}",
                    reviewed.source, reviewed.kind, reviewed.target
                ),
                reason: "every reviewed dependency requires a nonempty rationale".to_owned(),
            });
        }

        if reviewed_dependencies.iter().take(index).any(|prior| {
            prior.source == reviewed.source
                && prior.target == reviewed.target
                && prior.kind == reviewed.kind
        }) {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: format!(
                    "{} --{}--> {}",
                    reviewed.source, reviewed.kind, reviewed.target
                ),
                reason: "reviewed dependency entries must be unique by source, target, and kind"
                    .to_owned(),
            });
        }
    }
}

fn feature_forward_table_failures(
    registry: &str,
    reviewed_forwards: &[ReviewedFeatureForward],
    failures: &mut Vec<PolicyConfigurationFailure>,
) {
    for (index, reviewed) in reviewed_forwards.iter().enumerate() {
        let rendered = format!(
            "{}[{}] --{}--> {}[{}]",
            reviewed.source_package,
            reviewed.source_feature,
            reviewed.dependency_kind,
            reviewed.target_package,
            reviewed.target_feature
        );
        if reviewed.rationale.trim().is_empty() {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: rendered.clone(),
                reason: "every reviewed feature forward requires a nonempty rationale".to_owned(),
            });
        }
        if [
            reviewed.source_package,
            reviewed.source_feature,
            reviewed.target_package,
            reviewed.target_feature,
        ]
        .into_iter()
        .any(|value| value.trim().is_empty())
        {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: rendered.clone(),
                reason: "reviewed feature-forward package and feature names must be nonempty"
                    .to_owned(),
            });
        }
        if reviewed.source_feature != "cuda" || reviewed.target_feature != "cuda" {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: rendered.clone(),
                reason: "reviewed CUDA forwards must preserve the exact cuda feature name at both ends; aliases are not allowed".to_owned(),
            });
        }
        if reviewed.source_package == reviewed.target_package {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: rendered.clone(),
                reason: "reviewed feature forwards must cross a package dependency".to_owned(),
            });
        }
        if reviewed_forwards.iter().take(index).any(|prior| {
            prior.source_package == reviewed.source_package
                && prior.source_feature == reviewed.source_feature
                && prior.target_package == reviewed.target_package
                && prior.target_feature == reviewed.target_feature
        }) {
            failures.push(PolicyConfigurationFailure {
                rule: RULE_REVIEW_REGISTRY,
                source: registry.to_owned(),
                target: rendered,
                reason: "reviewed feature-forward entries must be unique by source package, source feature, target package, and target feature; dependency kinds may not conflict".to_owned(),
            });
        }
    }
}

fn reviewed_graph_is_acyclic(reviewed_dependencies: &[ReviewedDependency]) -> bool {
    let mut graph = BTreeMap::<&'static str, BTreeSet<&'static str>>::new();
    for reviewed in reviewed_dependencies {
        graph
            .entry(reviewed.source)
            .or_default()
            .insert(reviewed.target);
        graph.entry(reviewed.target).or_default();
    }

    let mut incoming = graph
        .keys()
        .copied()
        .map(|package| (package, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for targets in graph.values() {
        for target in targets {
            if let Some(count) = incoming.get_mut(target) {
                *count += 1;
            }
        }
    }

    let mut ready = incoming
        .iter()
        .filter_map(|(package, count)| (*count == 0).then_some(*package))
        .collect::<BTreeSet<_>>();
    let mut visited = 0_usize;
    while let Some(package) = ready.pop_first() {
        visited += 1;
        if let Some(targets) = graph.get(package) {
            for target in targets {
                if let Some(count) = incoming.get_mut(target) {
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(*target);
                    }
                }
            }
        }
    }

    visited == graph.len()
}

pub(super) fn local_dependency_policy(
    source_name: &str,
    source_layer: Layer,
    target_name: &str,
    target_layer: Layer,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    if target_layer == Layer::Benchmark {
        return Some(PolicyFailure {
            rule: RULE_BENCHMARK_REVERSE,
            reason: "production, tooling, and test packages cannot depend on benchmark packages; benchmark code is an outer consumer only".to_owned(),
        });
    }
    if source_layer == Layer::Benchmark {
        if kind == DependencyKind::Build {
            return Some(PolicyFailure {
                rule: RULE_BENCHMARK_BUILD,
                reason:
                    "benchmark packages cannot use build dependencies or custom build-time behavior"
                        .to_owned(),
            });
        }
        return reviewed_dependency(
            REVIEWED_BENCHMARK_DEPENDENCIES,
            source_name,
            target_name,
            kind,
        )
        .map_or_else(
            || {
                Some(PolicyFailure {
                    rule: RULE_BENCHMARK_LOCAL_REVIEW,
                    reason: "benchmark packages may consume only exact reviewed public production APIs needed by an implemented measurement".to_owned(),
                })
            },
            |_| None,
        );
    }

    match kind {
        DependencyKind::Normal | DependencyKind::Build => {
            local_production_policy(source_name, source_layer, target_name, target_layer, kind)
        }
        DependencyKind::Development => local_development_policy(source_name, target_name, kind),
    }
}

pub(super) fn local_production_policy(
    source_name: &str,
    source_layer: Layer,
    target_name: &str,
    target_layer: Layer,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    if !allows_production(source_layer, target_layer) {
        return Some(PolicyFailure {
            rule: RULE_PRODUCTION_DIRECTION,
            reason: "normal and build dependencies must follow the declared 9-role workspace dependency matrix".to_owned(),
        });
    }

    if is_domain_layer(source_layer) && is_domain_layer(target_layer) {
        return reviewed_dependency(
            REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES,
            source_name,
            target_name,
            kind,
        )
        .map_or_else(
            || {
                Some(PolicyFailure {
                    rule: RULE_DOMAIN_LOCAL_REVIEW,
                    reason: "every domain-to-domain production dependency requires an exact reviewed edge with a nonempty rationale in the acyclic domain graph".to_owned(),
                })
            },
            |_| None,
        );
    }

    if requires_engine_composition_review(source_layer, target_layer) {
        return reviewed_dependency(
            REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
            source_name,
            target_name,
            kind,
        )
        .map_or_else(
            || {
                Some(PolicyFailure {
                    rule: RULE_ENGINE_LOCAL_REVIEW,
                    reason: "engine dependencies on adapters or other engines require an exact reviewed composition edge with a rationale".to_owned(),
                })
            },
            |_| None,
        );
    }

    None
}

pub(super) fn local_development_policy(
    source_name: &str,
    target_name: &str,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    reviewed_dependency(
        REVIEWED_LOCAL_DEV_DEPENDENCIES,
        source_name,
        target_name,
        kind,
    )
    .map_or_else(
        || {
            Some(PolicyFailure {
                rule: RULE_LOCAL_DEV_REVIEW,
                reason: "workspace-local development dependencies require an explicit compatibility-test rationale, even when the production matrix would allow the edge".to_owned(),
            })
        },
        |_| None,
    )
}

const fn is_domain_layer(layer: Layer) -> bool {
    matches!(layer, Layer::FeatureFoundation | Layer::FeatureAlgorithm)
}

const fn requires_engine_composition_review(source: Layer, target: Layer) -> bool {
    matches!(
        source,
        Layer::EngineFoundation | Layer::EngineCapability | Layer::EngineApplication
    ) && matches!(
        target,
        Layer::Adapter
            | Layer::EngineFoundation
            | Layer::EngineCapability
            | Layer::EngineApplication
    )
}

pub(super) fn external_policy(
    source_name: &str,
    source_layer: Layer,
    target_name: &str,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    if source_layer == Layer::Benchmark {
        if kind == DependencyKind::Build {
            return Some(PolicyFailure {
                rule: RULE_BENCHMARK_BUILD,
                reason:
                    "benchmark packages cannot use build dependencies or custom build-time behavior"
                        .to_owned(),
            });
        }
        return reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_BENCHMARK_EXTERNAL_REVIEW,
            "benchmark external dependencies require an exact review tied to an implemented measurement",
        );
    }
    if kind == DependencyKind::Development {
        return reviewed_dependency(
            REVIEWED_EXTERNAL_DEPENDENCIES,
            source_name,
            target_name,
            kind,
        )
        .map_or_else(
            || {
                Some(PolicyFailure {
                    rule: RULE_EXTERNAL_DEV_REVIEW,
                    reason: "external development dependencies are allowed only after a separate, exact test or benchmark review".to_owned(),
                })
            },
            |_| None,
        );
    }

    match source_layer {
        Layer::Tooling => reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_TOOLING_EXTERNAL,
            "tooling production dependencies must be explicitly reviewed and tools/xtask is the only recognized tooling package",
        ),
        Layer::FeatureFoundation => reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_F0_EXTERNAL,
            "F0 has no production external dependencies; infrastructure and vendor crates are forbidden without an explicit exception",
        ),
        Layer::FeatureAlgorithm => reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_F1_EXTERNAL,
            "F1 production external dependencies are limited to reviewed portable dependencies (currently sampling -> libm)",
        ),
        Layer::EngineFoundation | Layer::EngineCapability | Layer::EngineApplication => {
            reviewed_external_or_failure(
                source_name,
                target_name,
                kind,
                RULE_ENGINE_EXTERNAL,
                "engine external production dependencies require an exact rationale and explicit orchestration review; frontend toolkits are prohibited",
            )
        }
        Layer::Adapter | Layer::Application => None,
        Layer::Benchmark => reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_BENCHMARK_EXTERNAL_REVIEW,
            "benchmark external dependencies require an exact review tied to an implemented measurement",
        ),
    }
}

fn reviewed_external_or_failure(
    source_name: &str,
    target_name: &str,
    kind: DependencyKind,
    rule: &'static str,
    reason: &'static str,
) -> Option<PolicyFailure> {
    reviewed_dependency(
        REVIEWED_EXTERNAL_DEPENDENCIES,
        source_name,
        target_name,
        kind,
    )
    .map_or_else(
        || {
            Some(PolicyFailure {
                rule,
                reason: reason.to_owned(),
            })
        },
        |_| None,
    )
}

pub(super) const fn reviewed_cuda_feature_forwards() -> &'static [ReviewedFeatureForward] {
    REVIEWED_CUDA_FEATURE_FORWARDS
}

pub(super) fn reviewed_cuda_feature_forward(
    source_package: &str,
    source_feature: &str,
    target_package: &str,
    target_feature: &str,
    dependency_kind: DependencyKind,
) -> Option<&'static ReviewedFeatureForward> {
    REVIEWED_CUDA_FEATURE_FORWARDS
        .iter()
        .find(|reviewed| {
            reviewed.source_package == source_package
                && reviewed.source_feature == source_feature
                && reviewed.target_package == target_package
                && reviewed.target_feature == target_feature
                && reviewed.dependency_kind == dependency_kind
        })
        .filter(|reviewed| !reviewed.rationale.trim().is_empty())
}

fn reviewed_dependency<'a>(
    policy: &'a [ReviewedDependency],
    source_name: &str,
    target_name: &str,
    kind: DependencyKind,
) -> Option<&'a ReviewedDependency> {
    policy
        .iter()
        .find(|reviewed| {
            reviewed.source == source_name
                && reviewed.target == target_name
                && reviewed.kind == kind
        })
        .filter(|reviewed| !reviewed.rationale.trim().is_empty())
}

pub(super) const fn dependency_kind(
    kind: cargo_metadata::DependencyKind,
) -> Option<DependencyKind> {
    match kind {
        cargo_metadata::DependencyKind::Normal => Some(DependencyKind::Normal),
        cargo_metadata::DependencyKind::Build => Some(DependencyKind::Build),
        cargo_metadata::DependencyKind::Development => Some(DependencyKind::Development),
        _ => None,
    }
}

pub(super) fn classify_manifest(root: &Path, manifest: &Path) -> Option<Layer> {
    let relative = manifest.strip_prefix(root).ok()?;
    if relative == Path::new("tools/xtask/Cargo.toml") {
        return Some(Layer::Tooling);
    }
    if relative == Path::new("benchmarks/runtime/Cargo.toml") {
        return Some(Layer::Benchmark);
    }
    if relative.file_name()? != "Cargo.toml" {
        return None;
    }
    let package_directory = relative.parent()?;

    if package_directory == Path::new("crates/domain/domain-contracts") {
        Some(Layer::FeatureFoundation)
    } else if is_direct_child(package_directory, Path::new("crates/domain")) {
        Some(Layer::FeatureAlgorithm)
    } else if package_directory == Path::new("crates/platform/host-runtime") {
        Some(Layer::Adapter)
    } else if is_direct_child(package_directory, Path::new("crates/platform")) {
        None
    } else if is_direct_child(package_directory, Path::new("crates/adapters")) {
        Some(Layer::Adapter)
    } else if package_directory == Path::new("crates/runtime/inference-runtime") {
        Some(Layer::EngineFoundation)
    } else if package_directory == Path::new("crates/runtime/corrective-workflow") {
        Some(Layer::EngineCapability)
    } else if package_directory == Path::new("crates/runtime/application-runtime") {
        Some(Layer::EngineApplication)
    } else if is_direct_child(package_directory, Path::new("crates/runtime")) {
        None
    } else if is_direct_child(package_directory, Path::new("crates/apps")) {
        Some(Layer::Application)
    } else {
        None
    }
}

pub(super) fn is_direct_child(path: &Path, parent: &Path) -> bool {
    path.strip_prefix(parent)
        .is_ok_and(|relative| relative.components().count() == 1)
}

const fn allows_production(source: Layer, target: Layer) -> bool {
    match source {
        Layer::Tooling | Layer::FeatureFoundation => false,
        Layer::Benchmark => matches!(
            target,
            Layer::FeatureFoundation
                | Layer::FeatureAlgorithm
                | Layer::Adapter
                | Layer::EngineFoundation
                | Layer::EngineCapability
                | Layer::EngineApplication
        ),
        Layer::FeatureAlgorithm => {
            matches!(target, Layer::FeatureFoundation | Layer::FeatureAlgorithm)
        }
        Layer::Adapter => matches!(target, Layer::FeatureFoundation | Layer::FeatureAlgorithm),
        Layer::EngineFoundation => matches!(
            target,
            Layer::FeatureFoundation | Layer::FeatureAlgorithm | Layer::Adapter
        ),
        Layer::EngineCapability => matches!(
            target,
            Layer::FeatureFoundation
                | Layer::FeatureAlgorithm
                | Layer::Adapter
                | Layer::EngineFoundation
        ),
        Layer::EngineApplication => matches!(
            target,
            Layer::FeatureFoundation
                | Layer::FeatureAlgorithm
                | Layer::Adapter
                | Layer::EngineFoundation
                | Layer::EngineCapability
        ),
        Layer::Application => matches!(target, Layer::EngineApplication),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        DependencyKind, Layer, REVIEWED_BENCHMARK_DEPENDENCIES, REVIEWED_CUDA_FEATURE_FORWARDS,
        REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES, REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
        REVIEWED_EXTERNAL_DEPENDENCIES, REVIEWED_LOCAL_DEV_DEPENDENCIES, RULE_BENCHMARK_BUILD,
        RULE_BENCHMARK_EXTERNAL_REVIEW, RULE_BENCHMARK_LOCAL_REVIEW, RULE_BENCHMARK_REVERSE,
        RULE_DOMAIN_LOCAL_REVIEW, RULE_ENGINE_EXTERNAL, RULE_ENGINE_LOCAL_REVIEW,
        RULE_EXTERNAL_DEV_REVIEW, RULE_F0_EXTERNAL, RULE_F1_EXTERNAL, ReviewedDependency,
        ReviewedFeatureForward, allows_production, classify_manifest, external_policy,
        feature_forward_table_failures, local_dependency_policy, local_production_policy,
        policy_configuration_failures, review_table_failures, reviewed_cuda_feature_forward,
        reviewed_dependency, reviewed_graph_is_acyclic,
    };

    const LAYERS: [Layer; 9] = [
        Layer::Tooling,
        Layer::Benchmark,
        Layer::FeatureFoundation,
        Layer::FeatureAlgorithm,
        Layer::Adapter,
        Layer::EngineFoundation,
        Layer::EngineCapability,
        Layer::EngineApplication,
        Layer::Application,
    ];

    #[test]
    fn complete_nine_by_nine_workspace_role_matrix_matches_policy() {
        #[rustfmt::skip]
        const EXPECTED: [[bool; 9]; 9] = [
            [false, false, false, false, false, false, false, false, false],
            [false, false, true,  true,  true,  true,  true,  true,  false],
            [false, false, false, false, false, false, false, false, false],
            [false, false, true,  true,  false, false, false, false, false],
            [false, false, true,  true,  false, false, false, false, false],
            [false, false, true,  true,  true,  false, false, false, false],
            [false, false, true,  true,  true,  true,  false, false, false],
            [false, false, true,  true,  true,  true,  true,  false, false],
            [false, false, false, false, false, false, false, true,  false],
        ];

        for (source, expected_targets) in LAYERS.into_iter().zip(EXPECTED) {
            for (target, expected) in LAYERS.into_iter().zip(expected_targets) {
                assert_eq!(
                    allows_production(source, target),
                    expected,
                    "unexpected policy for {source:?} -> {target:?}"
                );
            }
        }
    }

    #[test]
    fn manifests_classify_only_the_exact_xtask_tool() {
        let root = Path::new("/workspace");
        let cases = [
            ("Cargo.toml", None),
            ("tools/xtask/Cargo.toml", Some(Layer::Tooling)),
            ("benchmarks/runtime/Cargo.toml", Some(Layer::Benchmark)),
            ("benchmarks/experimental/Cargo.toml", None),
            ("benchmarks/runtime/helper/Cargo.toml", None),
            ("benchmarks/Cargo.toml", None),
            (
                "crates/domain/domain-contracts/Cargo.toml",
                Some(Layer::FeatureFoundation),
            ),
            (
                "crates/domain/sampling/Cargo.toml",
                Some(Layer::FeatureAlgorithm),
            ),
            (
                "crates/platform/host-runtime/Cargo.toml",
                Some(Layer::Adapter),
            ),
            (
                "crates/adapters/candle-backend/Cargo.toml",
                Some(Layer::Adapter),
            ),
            (
                "crates/runtime/inference-runtime/Cargo.toml",
                Some(Layer::EngineFoundation),
            ),
            (
                "crates/runtime/corrective-workflow/Cargo.toml",
                Some(Layer::EngineCapability),
            ),
            (
                "crates/runtime/application-runtime/Cargo.toml",
                Some(Layer::EngineApplication),
            ),
            (
                "crates/apps/desktop-slint/Cargo.toml",
                Some(Layer::Application),
            ),
            ("crates/runtime/memory-runtime/Cargo.toml", None),
            ("crates/platform/native/Cargo.toml", None),
            ("crates/experimental/new-layer/Cargo.toml", None),
            ("crates/apps/nested/too-deep/Cargo.toml", None),
            ("tools/maintenance/Cargo.toml", None),
            ("tools/xtask/helper/Cargo.toml", None),
        ];

        for (relative, expected) in cases {
            assert_eq!(classify_manifest(root, &root.join(relative)), expected);
        }
    }

    #[test]
    fn benchmark_edges_are_outer_only_and_require_exact_review() {
        assert_benchmark_reverse_edges();
        assert_benchmark_local_edges();
        assert_benchmark_external_edges();
    }

    fn assert_benchmark_reverse_edges() {
        for source in LAYERS {
            if source == Layer::Benchmark {
                continue;
            }
            for kind in [
                DependencyKind::Normal,
                DependencyKind::Build,
                DependencyKind::Development,
            ] {
                let failure = local_dependency_policy(
                    "production-package",
                    source,
                    "runtime-benchmarks",
                    Layer::Benchmark,
                    kind,
                );
                assert_eq!(
                    failure.map(|failure| failure.rule),
                    Some(RULE_BENCHMARK_REVERSE)
                );
            }
        }
    }

    fn assert_benchmark_local_edges() {
        const EXPECTED: [(&str, Layer); 5] = [
            ("application-runtime", Layer::EngineApplication),
            ("candle-backend", Layer::Adapter),
            ("domain-contracts", Layer::FeatureFoundation),
            ("host-runtime", Layer::Adapter),
            ("inference-runtime", Layer::EngineFoundation),
        ];

        assert!(allows_production(
            Layer::Benchmark,
            Layer::EngineApplication
        ));
        assert_eq!(REVIEWED_BENCHMARK_DEPENDENCIES.len(), EXPECTED.len());
        for ((target, layer), reviewed) in EXPECTED.into_iter().zip(REVIEWED_BENCHMARK_DEPENDENCIES)
        {
            assert_eq!(reviewed.source, "runtime-benchmarks");
            assert_eq!(reviewed.target, target);
            assert_eq!(reviewed.kind, DependencyKind::Normal);
            assert!(!reviewed.rationale.trim().is_empty());
            assert!(
                local_dependency_policy(
                    "runtime-benchmarks",
                    Layer::Benchmark,
                    target,
                    layer,
                    DependencyKind::Normal,
                )
                .is_none()
            );
        }

        assert_eq!(
            local_dependency_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "application-runtime",
                Layer::EngineApplication,
                DependencyKind::Development,
            )
            .map(|failure| failure.rule),
            Some(RULE_BENCHMARK_LOCAL_REVIEW)
        );
        assert_eq!(
            local_dependency_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "application-runtime",
                Layer::EngineApplication,
                DependencyKind::Build,
            )
            .map(|failure| failure.rule),
            Some(RULE_BENCHMARK_BUILD)
        );
        assert_eq!(
            local_dependency_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "sampling",
                Layer::FeatureAlgorithm,
                DependencyKind::Normal,
            )
            .map(|failure| failure.rule),
            Some(RULE_BENCHMARK_LOCAL_REVIEW)
        );
    }

    fn assert_benchmark_external_edges() {
        for dependency in ["serde", "serde_json", "sha2"] {
            assert!(
                external_policy(
                    "runtime-benchmarks",
                    Layer::Benchmark,
                    dependency,
                    DependencyKind::Normal,
                )
                .is_none()
            );
        }
        assert!(
            external_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "criterion",
                DependencyKind::Development,
            )
            .is_none()
        );
        assert_eq!(
            external_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "stats_alloc",
                DependencyKind::Development,
            )
            .map(|failure| failure.rule),
            Some(RULE_BENCHMARK_EXTERNAL_REVIEW)
        );
        assert_eq!(
            external_policy(
                "runtime-benchmarks",
                Layer::Benchmark,
                "criterion",
                DependencyKind::Build,
            )
            .map(|failure| failure.rule),
            Some(RULE_BENCHMARK_BUILD)
        );
    }

    #[test]
    fn current_domain_edges_are_exactly_reviewed() {
        const EXPECTED: [(&str, &str); 4] = [
            ("tokenization", "domain-contracts"),
            ("context-planner", "domain-contracts"),
            ("sampling", "domain-contracts"),
            ("task-graph", "domain-contracts"),
        ];

        assert_eq!(
            REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES.len(),
            EXPECTED.len()
        );
        for ((source, target), reviewed) in EXPECTED
            .into_iter()
            .zip(REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES)
        {
            assert_eq!((reviewed.source, reviewed.target), (source, target));
            assert_eq!(reviewed.kind, DependencyKind::Normal);
            assert!(!reviewed.rationale.trim().is_empty());
            assert!(
                local_production_policy(
                    source,
                    Layer::FeatureAlgorithm,
                    target,
                    Layer::FeatureFoundation,
                    DependencyKind::Normal,
                )
                .is_none()
            );
        }
    }

    #[test]
    fn unreviewed_f1_peer_edge_fails_after_coarse_layer_acceptance() {
        assert!(allows_production(
            Layer::FeatureAlgorithm,
            Layer::FeatureAlgorithm
        ));
        let failure = local_production_policy(
            "sampling",
            Layer::FeatureAlgorithm,
            "tokenization",
            Layer::FeatureAlgorithm,
            DependencyKind::Normal,
        );

        assert_eq!(
            failure.map(|failure| failure.rule),
            Some(RULE_DOMAIN_LOCAL_REVIEW)
        );
    }

    #[test]
    fn reviewed_domain_production_graph_is_acyclic() {
        assert!(reviewed_graph_is_acyclic(
            REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES
        ));
    }

    #[test]
    fn review_registry_rejects_duplicate_entries_and_empty_rationales() {
        const INVALID: &[ReviewedDependency] = &[
            ReviewedDependency {
                source: "alpha",
                target: "beta",
                kind: DependencyKind::Normal,
                rationale: "",
            },
            ReviewedDependency {
                source: "alpha",
                target: "beta",
                kind: DependencyKind::Normal,
                rationale: "duplicate",
            },
        ];
        let mut failures = Vec::new();
        review_table_failures("test reviews", INVALID, &mut failures);

        assert!(
            failures
                .iter()
                .any(|failure| failure.reason.contains("nonempty rationale"))
        );
        assert!(
            failures
                .iter()
                .any(|failure| failure.reason.contains("must be unique"))
        );
    }

    #[test]
    fn cuda_feature_forwards_are_exactly_reviewed() {
        const EXPECTED: [(&str, &str, &str, &str, DependencyKind); 4] = [
            (
                "application-runtime",
                "cuda",
                "candle-backend",
                "cuda",
                DependencyKind::Normal,
            ),
            (
                "desktop-slint",
                "cuda",
                "application-runtime",
                "cuda",
                DependencyKind::Normal,
            ),
            (
                "inference-runtime",
                "cuda",
                "candle-backend",
                "cuda",
                DependencyKind::Development,
            ),
            (
                "runtime-benchmarks",
                "cuda",
                "application-runtime",
                "cuda",
                DependencyKind::Normal,
            ),
        ];

        assert_eq!(REVIEWED_CUDA_FEATURE_FORWARDS.len(), EXPECTED.len());
        for (reviewed, expected) in REVIEWED_CUDA_FEATURE_FORWARDS.iter().zip(EXPECTED) {
            assert_eq!(
                (
                    reviewed.source_package,
                    reviewed.source_feature,
                    reviewed.target_package,
                    reviewed.target_feature,
                    reviewed.dependency_kind,
                ),
                expected
            );
            assert!(!reviewed.rationale.trim().is_empty());
            assert!(
                reviewed_cuda_feature_forward(
                    reviewed.source_package,
                    reviewed.source_feature,
                    reviewed.target_package,
                    reviewed.target_feature,
                    reviewed.dependency_kind,
                )
                .is_some()
            );
        }
    }

    #[test]
    fn feature_forward_registry_rejects_aliases_duplicates_and_empty_configuration() {
        const INVALID: &[ReviewedFeatureForward] = &[
            ReviewedFeatureForward {
                source_package: "alpha",
                source_feature: "gpu",
                target_package: "beta",
                target_feature: "cuda",
                dependency_kind: DependencyKind::Normal,
                rationale: "",
            },
            ReviewedFeatureForward {
                source_package: "alpha",
                source_feature: "gpu",
                target_package: "beta",
                target_feature: "cuda",
                dependency_kind: DependencyKind::Development,
                rationale: "conflicting duplicate",
            },
            ReviewedFeatureForward {
                source_package: "",
                source_feature: "cuda",
                target_package: "beta",
                target_feature: "cuda",
                dependency_kind: DependencyKind::Normal,
                rationale: "missing source package",
            },
        ];
        let mut failures = Vec::new();
        feature_forward_table_failures("test feature forwards", INVALID, &mut failures);

        for expected in [
            "nonempty rationale",
            "exact cuda feature name",
            "must be unique",
            "package and feature names must be nonempty",
        ] {
            assert!(
                failures
                    .iter()
                    .any(|failure| failure.reason.contains(expected)),
                "missing feature-forward configuration failure containing {expected:?}"
            );
        }
    }

    #[test]
    fn cyclic_review_graph_is_rejected() {
        const CYCLE: &[ReviewedDependency] = &[
            ReviewedDependency {
                source: "alpha",
                target: "beta",
                kind: DependencyKind::Normal,
                rationale: "forward",
            },
            ReviewedDependency {
                source: "beta",
                target: "alpha",
                kind: DependencyKind::Normal,
                rationale: "backward",
            },
        ];

        assert!(!reviewed_graph_is_acyclic(CYCLE));
    }

    #[test]
    fn configured_review_registries_are_well_formed() {
        assert!(policy_configuration_failures().is_empty());
    }

    #[test]
    fn external_infrastructure_is_forbidden_in_f0_and_f1() {
        let f0 = external_policy(
            "domain-contracts",
            Layer::FeatureFoundation,
            "redb",
            DependencyKind::Normal,
        );
        let f1 = external_policy(
            "sampling",
            Layer::FeatureAlgorithm,
            "hf-hub",
            DependencyKind::Normal,
        );

        assert_eq!(f0.map(|failure| failure.rule), Some(RULE_F0_EXTERNAL));
        assert_eq!(f1.map(|failure| failure.rule), Some(RULE_F1_EXTERNAL));
        assert!(
            external_policy(
                "sampling",
                Layer::FeatureAlgorithm,
                "libm",
                DependencyKind::Normal,
            )
            .is_none()
        );
    }

    #[test]
    fn arbitrary_frontend_and_unreviewed_orchestration_dependencies_fail_for_engines() {
        let frontend = external_policy(
            "inference-runtime",
            Layer::EngineFoundation,
            "iced",
            DependencyKind::Normal,
        );
        let capability = external_policy(
            "corrective-workflow",
            Layer::EngineCapability,
            "tokio",
            DependencyKind::Normal,
        );
        let orchestration = external_policy(
            "application-runtime",
            Layer::EngineApplication,
            "tokio",
            DependencyKind::Normal,
        );

        for failure in [&frontend, &capability, &orchestration] {
            assert_eq!(
                failure.as_ref().map(|failure| failure.rule),
                Some(RULE_ENGINE_EXTERNAL)
            );
            assert!(failure.as_ref().is_some_and(|failure| {
                failure.reason.contains("explicit orchestration review")
                    && failure.reason.contains("frontend toolkits are prohibited")
            }));
        }
    }

    #[test]
    fn engine_composition_edges_require_exact_review() {
        assert!(
            local_production_policy(
                "application-runtime",
                Layer::EngineApplication,
                "inference-runtime",
                Layer::EngineFoundation,
                DependencyKind::Normal,
            )
            .is_none()
        );
        assert!(
            local_production_policy(
                "application-runtime",
                Layer::EngineApplication,
                "candle-backend",
                Layer::Adapter,
                DependencyKind::Normal,
            )
            .is_none()
        );
        assert!(
            local_production_policy(
                "application-runtime",
                Layer::EngineApplication,
                "tokenization",
                Layer::FeatureAlgorithm,
                DependencyKind::Normal,
            )
            .is_none()
        );

        let unreviewed_capability = local_production_policy(
            "application-runtime",
            Layer::EngineApplication,
            "corrective-workflow",
            Layer::EngineCapability,
            DependencyKind::Normal,
        );
        assert_eq!(
            unreviewed_capability.map(|failure| failure.rule),
            Some(RULE_ENGINE_LOCAL_REVIEW)
        );

        let unreviewed_e0 = local_production_policy(
            "corrective-workflow",
            Layer::EngineCapability,
            "inference-runtime",
            Layer::EngineFoundation,
            DependencyKind::Normal,
        );
        assert_eq!(
            unreviewed_e0.map(|failure| failure.rule),
            Some(RULE_ENGINE_LOCAL_REVIEW)
        );
    }

    #[test]
    fn external_dev_dependencies_have_an_exact_separate_review_list() {
        assert!(
            external_policy(
                "sampling",
                Layer::FeatureAlgorithm,
                "criterion",
                DependencyKind::Development,
            )
            .is_none()
        );
        assert!(
            external_policy(
                "inference-runtime",
                Layer::EngineFoundation,
                "candle-core",
                DependencyKind::Development,
            )
            .is_none()
        );
        assert_eq!(
            external_policy(
                "domain-contracts",
                Layer::FeatureFoundation,
                "criterion",
                DependencyKind::Development,
            )
            .map(|failure| failure.rule),
            Some(RULE_EXTERNAL_DEV_REVIEW)
        );
    }

    #[test]
    fn reviewed_dependencies_include_inspectable_rationales() {
        for policy in [
            REVIEWED_BENCHMARK_DEPENDENCIES,
            REVIEWED_DOMAIN_PRODUCTION_DEPENDENCIES,
            REVIEWED_EXTERNAL_DEPENDENCIES,
            REVIEWED_LOCAL_DEV_DEPENDENCIES,
            REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
        ] {
            for reviewed in policy {
                assert!(!reviewed.rationale.trim().is_empty());
                assert!(
                    reviewed_dependency(policy, reviewed.source, reviewed.target, reviewed.kind)
                        .is_some()
                );
            }
        }
        for reviewed in REVIEWED_CUDA_FEATURE_FORWARDS {
            assert!(!reviewed.rationale.trim().is_empty());
        }
    }
}
