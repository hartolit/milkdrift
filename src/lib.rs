//! Layered workspace architecture validation.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use cargo_metadata::{Dependency, Metadata, MetadataCommand, Package};

mod hygiene;

pub use hygiene::{HygieneError, HygieneReport, HygieneViolation, validate_repository_hygiene};

const RULE_KNOWN_LOCATION: &str = "LAYOUT-1";
const RULE_LOCAL_TARGET: &str = "LAYOUT-2";
const RULE_KNOWN_KIND: &str = "DEPENDENCY-KIND-1";
const RULE_PLATFORM_ROLE: &str = "PLATFORM-ROLE-1";
const RULE_PRODUCTION_DIRECTION: &str = "LAYER-PROD-1";
const RULE_RUNTIME_ROLE: &str = "RUNTIME-ROLE-1";
const RULE_ENGINE_LOCAL_REVIEW: &str = "ENGINE-LOCAL-PROD-1";
const RULE_LOCAL_DEV_REVIEW: &str = "DEV-LOCAL-1";
const RULE_EXTERNAL_DEV_REVIEW: &str = "EXT-DEV-1";
const RULE_ROOT_EXTERNAL: &str = "EXT-ROOT-PROD-1";
const RULE_F0_EXTERNAL: &str = "EXT-F0-PROD-1";
const RULE_F1_EXTERNAL: &str = "EXT-F1-PROD-1";
const RULE_ENGINE_EXTERNAL: &str = "EXT-ENGINE-PROD-1";

/// A dependency layer in the workspace's production architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// The workspace maintenance runner.
    Root,
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

/// One architecture policy violation.
#[derive(Debug, PartialEq, Eq)]
pub struct Violation {
    source: String,
    target: String,
    dependency_kind: Option<DependencyKind>,
    source_layer: Option<Layer>,
    target_layer: Option<Layer>,
    rule: &'static str,
    reason: String,
}

impl Violation {
    /// Returns the stable identifier of the policy rule that was violated.
    #[must_use]
    pub const fn rule(&self) -> &'static str {
        self.rule
    }

    /// Returns the human-readable reason the policy rejected the item.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Returns the source package or manifest description.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the dependency package or location description.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the dependency kind when the violation represents an edge.
    #[must_use]
    pub const fn dependency_kind(&self) -> Option<DependencyKind> {
        self.dependency_kind
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dependency_kind {
            Some(kind) => write!(
                formatter,
                "forbidden architecture dependency: {} ({:?}) --{}--> {} ({:?}); policy rule {}: {}",
                self.source,
                self.source_layer,
                kind,
                self.target,
                self.target_layer,
                self.rule,
                self.reason
            ),
            None => write!(
                formatter,
                "architecture policy violation: {} -> {}; policy rule {}: {}",
                self.source, self.target, self.rule, self.reason
            ),
        }
    }
}

/// The complete result of validating one Cargo workspace.
#[derive(Debug, Default)]
pub struct ValidationReport {
    violations: Vec<Violation>,
}

impl ValidationReport {
    /// Returns true when the workspace satisfies every architecture rule.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    /// Returns all violations discovered in the workspace.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }
}

/// An error that prevented Cargo metadata from being loaded.
#[derive(Debug)]
pub struct ValidationError(cargo_metadata::Error);

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not load locked Cargo metadata: {}",
            self.0
        )
    }
}

impl Error for ValidationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<cargo_metadata::Error> for ValidationError {
    fn from(error: cargo_metadata::Error) -> Self {
        Self(error)
    }
}

#[derive(Clone, Copy)]
struct ReviewedDependency {
    source: &'static str,
    target: &'static str,
    kind: DependencyKind,
    justification: &'static str,
}

// Engine dependencies on infrastructure or another engine are exact reviewed composition edges.
// Lower feature dependencies continue to follow the layer matrix without per-edge registration.
const REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "inference-runtime",
        target: "host-runtime",
        kind: DependencyKind::Normal,
        justification: "E0 uses the bounded host worker integration that runs its command loop",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "candle-backend",
        kind: DependencyKind::Normal,
        justification: "the closed E1 local composition constructs the supported Candle/Safetensors source",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "hf-hub-adapter",
        kind: DependencyKind::Normal,
        justification: "the closed E1 local composition resolves immutable Hugging Face artifacts for the Candle product",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "hf-tokenizer",
        kind: DependencyKind::Normal,
        justification: "the closed E1 local composition owns Hugging Face prompt encoding and streaming decode",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "host-runtime",
        kind: DependencyKind::Normal,
        justification: "E1 hosts bounded workers and frontend-facing output accumulation",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "inference-runtime",
        kind: DependencyKind::Normal,
        justification: "E1 delegates current local model execution and lifecycle ownership to E0",
    },
    ReviewedDependency {
        source: "application-runtime",
        target: "redb-storage",
        kind: DependencyKind::Normal,
        justification: "E1 persists application preferences and the Hugging Face model catalogue with redb",
    },
];

// External production exceptions and all external development dependencies are exact and reviewed.
const REVIEWED_EXTERNAL_DEPENDENCIES: &[ReviewedDependency] = &[
    ReviewedDependency {
        source: "llm-app",
        target: "cargo_metadata",
        kind: DependencyKind::Normal,
        justification: "the maintenance runner requires Cargo's typed workspace metadata API",
    },
    ReviewedDependency {
        source: "sampling",
        target: "libm",
        kind: DependencyKind::Normal,
        justification: "sampling requires reviewed portable floating-point math",
    },
    ReviewedDependency {
        source: "domain-contracts",
        target: "stats_alloc",
        kind: DependencyKind::Development,
        justification: "allocation contract tests measure project-owned hot paths",
    },
    ReviewedDependency {
        source: "sampling",
        target: "stats_alloc",
        kind: DependencyKind::Development,
        justification: "sampling allocation tests measure the declared zero-allocation region",
    },
    ReviewedDependency {
        source: "sampling",
        target: "criterion",
        kind: DependencyKind::Development,
        justification: "Criterion compiles and runs the reviewed sampling benchmark",
    },
];

// Workspace-local development edges are reviewed independently from production direction.
const REVIEWED_LOCAL_DEV_DEPENDENCIES: &[ReviewedDependency] = &[ReviewedDependency {
    source: "inference-runtime",
    target: "candle-backend",
    kind: DependencyKind::Development,
    justification: "E0 compatibility tests exercise the Candle backend contract",
}];

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

    for package in packages {
        let source_name = package.name.to_string();
        let Some(source_layer) = classify_manifest(root, package.manifest_path.as_std_path())
        else {
            report
                .violations
                .push(unknown_package_location(package, root));
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
    } else {
        (
            RULE_KNOWN_LOCATION,
            "workspace packages must be the root runner or an explicitly registered crate at an approved path under crates/domain, crates/platform, crates/adapters, crates/runtime, or crates/apps; unknown package names or locations never receive a fallback layer",
        )
    };

    Violation {
        source: package.name.to_string(),
        target: relative.display().to_string(),
        dependency_kind: None,
        source_layer: None,
        target_layer: None,
        rule,
        reason: reason.to_owned(),
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
        report.violations.push(Violation {
            source: source_name.to_owned(),
            target: dependency.name.clone(),
            dependency_kind: None,
            source_layer: Some(source_layer),
            target_layer: None,
            rule: RULE_KNOWN_KIND,
            reason: format!(
                "Cargo reported an unsupported dependency kind {:?}; unknown kinds fail closed",
                dependency.kind
            ),
        });
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
        report.violations.push(edge_violation(
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
        report.violations.push(Violation {
            source: source_name.to_owned(),
            target: dependency_path.display().to_string(),
            dependency_kind: Some(kind),
            source_layer: Some(source_layer),
            target_layer: None,
            rule: RULE_LOCAL_TARGET,
            reason: "path dependencies must resolve to a recognized member of this workspace; outside, excluded, and otherwise unknown local paths fail closed".to_owned(),
        });
        return;
    };
    let Some(target_layer) = *target_layer else {
        report.violations.push(Violation {
            source: source_name.to_owned(),
            target: target_name.clone(),
            dependency_kind: Some(kind),
            source_layer: Some(source_layer),
            target_layer: None,
            rule: RULE_LOCAL_TARGET,
            reason: "the path dependency resolves to a workspace package whose location has no recognized architecture layer".to_owned(),
        });
        return;
    };

    let failure = match kind {
        DependencyKind::Normal | DependencyKind::Build => local_production_policy(
            source_name,
            source_layer,
            target_name,
            target_layer,
            kind,
        ),
        DependencyKind::Development => reviewed_dependency(
            REVIEWED_LOCAL_DEV_DEPENDENCIES,
            source_name,
            target_name,
            kind,
        )
        .map_or_else(
            || {
                Some(PolicyFailure {
                    rule: RULE_LOCAL_DEV_REVIEW,
                    reason: "workspace-local development dependencies require an explicit compatibility-test justification, even when the production matrix would allow the edge".to_owned(),
                })
            },
            |_| None,
        ),
    };

    if let Some(failure) = failure {
        report.violations.push(edge_violation(
            source_name,
            Some(source_layer),
            target_name,
            Some(target_layer),
            kind,
            failure,
        ));
    }
}

fn local_production_policy(
    source_name: &str,
    source_layer: Layer,
    target_name: &str,
    target_layer: Layer,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    if !allows_production(source_layer, target_layer) {
        return Some(PolicyFailure {
            rule: RULE_PRODUCTION_DIRECTION,
            reason: "normal and build dependencies must follow the declared 8-layer production direction matrix".to_owned(),
        });
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
                    reason: "engine dependencies on adapters or other engines require an exact reviewed composition edge with a justification".to_owned(),
                })
            },
            |_| None,
        );
    }

    None
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

struct PolicyFailure {
    rule: &'static str,
    reason: String,
}

fn external_policy(
    source_name: &str,
    source_layer: Layer,
    target_name: &str,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
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
        Layer::Root => reviewed_external_or_failure(
            source_name,
            target_name,
            kind,
            RULE_ROOT_EXTERNAL,
            "root runner production dependencies must be explicitly reviewed tooling dependencies",
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
                "engine external production dependencies require an exact justification and explicit orchestration review; frontend toolkits are prohibited",
            )
        }
        Layer::Adapter | Layer::Application => None,
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
        .filter(|reviewed| !reviewed.justification.is_empty())
}

fn edge_violation(
    source: &str,
    source_layer: Option<Layer>,
    target: &str,
    target_layer: Option<Layer>,
    dependency_kind: DependencyKind,
    failure: PolicyFailure,
) -> Violation {
    Violation {
        source: source.to_owned(),
        target: target.to_owned(),
        dependency_kind: Some(dependency_kind),
        source_layer,
        target_layer,
        rule: failure.rule,
        reason: failure.reason,
    }
}

const fn dependency_kind(kind: cargo_metadata::DependencyKind) -> Option<DependencyKind> {
    match kind {
        cargo_metadata::DependencyKind::Normal => Some(DependencyKind::Normal),
        cargo_metadata::DependencyKind::Build => Some(DependencyKind::Build),
        cargo_metadata::DependencyKind::Development => Some(DependencyKind::Development),
        _ => None,
    }
}

fn classify_manifest(root: &Path, manifest: &Path) -> Option<Layer> {
    let relative = manifest.strip_prefix(root).ok()?;
    if relative == Path::new("Cargo.toml") {
        return Some(Layer::Root);
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

fn is_direct_child(path: &Path, parent: &Path) -> bool {
    path.strip_prefix(parent)
        .is_ok_and(|relative| relative.components().count() == 1)
}

const fn allows_production(source: Layer, target: Layer) -> bool {
    match source {
        Layer::Root | Layer::FeatureFoundation => false,
        Layer::FeatureAlgorithm => matches!(target, Layer::FeatureFoundation),
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
        DependencyKind, Layer, REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
        REVIEWED_EXTERNAL_DEPENDENCIES, REVIEWED_LOCAL_DEV_DEPENDENCIES, RULE_ENGINE_EXTERNAL,
        RULE_ENGINE_LOCAL_REVIEW, RULE_EXTERNAL_DEV_REVIEW, RULE_F0_EXTERNAL, RULE_F1_EXTERNAL,
        allows_production, classify_manifest, external_policy, local_production_policy,
        reviewed_dependency,
    };

    const LAYERS: [Layer; 8] = [
        Layer::Root,
        Layer::FeatureFoundation,
        Layer::FeatureAlgorithm,
        Layer::Adapter,
        Layer::EngineFoundation,
        Layer::EngineCapability,
        Layer::EngineApplication,
        Layer::Application,
    ];

    #[test]
    fn complete_eight_by_eight_production_layer_matrix_matches_policy() {
        #[rustfmt::skip]
        const EXPECTED: [[bool; 8]; 8] = [
            [false, false, false, false, false, false, false, false],
            [false, false, false, false, false, false, false, false],
            [false, true,  false, false, false, false, false, false],
            [false, true,  true,  false, false, false, false, false],
            [false, true,  true,  true,  false, false, false, false],
            [false, true,  true,  true,  true,  false, false, false],
            [false, true,  true,  true,  true,  true,  false, false],
            [false, false, false, false, false, false, true,  false],
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
    fn manifests_are_classified_without_runtime_fallbacks() {
        let root = Path::new("/workspace");
        let cases = [
            ("Cargo.toml", Some(Layer::Root)),
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
        ];

        for (relative, expected) in cases {
            assert_eq!(classify_manifest(root, &root.join(relative)), expected);
        }
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
    fn reviewed_dependencies_include_inspectable_justifications() {
        for policy in [
            REVIEWED_EXTERNAL_DEPENDENCIES,
            REVIEWED_LOCAL_DEV_DEPENDENCIES,
            REVIEWED_ENGINE_PRODUCTION_DEPENDENCIES,
        ] {
            for reviewed in policy {
                assert!(!reviewed.justification.is_empty());
                assert!(
                    reviewed_dependency(policy, reviewed.source, reviewed.target, reviewed.kind)
                        .is_some()
                );
            }
        }
    }
}
