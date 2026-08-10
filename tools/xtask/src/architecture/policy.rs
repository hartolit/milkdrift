use std::fmt;

pub use crate::workspace::Role as Layer;

pub(super) const RULE_ROLE: &str = "ROLE-1";
pub(super) const RULE_LOCATION: &str = "LAYOUT-1";
pub(super) const RULE_LOCAL_TARGET: &str = "LOCAL-TARGET-1";
pub(super) const RULE_KNOWN_KIND: &str = "DEPENDENCY-KIND-1";
pub(super) const RULE_LAYER_DAG: &str = "LAYER-DAG-1";
pub(super) const RULE_DOMAIN_DAG: &str = "DOMAIN-DAG-1";
pub(super) const RULE_OBSERVER: &str = "BENCHMARK-OBSERVER-1";
pub(super) const RULE_TOOLING: &str = "TOOLING-ISOLATION-1";
pub(super) const RULE_EXTERNAL: &str = "EXTERNAL-DEPENDENCY-1";
pub(super) const RULE_EXCEPTION: &str = "POLICY-EXCEPTION-1";
pub(super) const RULE_BENCHMARK_PACKAGE: &str = "BENCHMARK-PACKAGE-1";
pub(super) const RULE_BENCHMARK_BUILD: &str = "BENCHMARK-BUILD-1";
pub(super) const RULE_BENCHMARK_REGISTRY: &str = "BENCHMARK-REGISTRY-1";
pub(super) const RULE_CUDA_DEFAULT: &str = "CUDA-DEFAULT-1";
pub(super) const RULE_CUDA_BOUNDARY: &str = "CUDA-BOUNDARY-1";
pub(super) const RULE_CUDA_PROHIBITED: &str = "CUDA-PROHIBITED-1";
pub(super) const RULE_CUDA_CONTRACT: &str = "CUDA-CONTRACT-1";
pub(super) const RULE_CUDA_HARDWARE: &str = "CUDA-HARDWARE-TEST-1";

/// The Cargo dependency section that declares an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyKind {
    /// A normal production dependency.
    Normal,
    /// A build-script dependency.
    Build,
    /// A development-only dependency.
    Development,
}

impl DependencyKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "build" => Some(Self::Build),
            "development" | "dev" => Some(Self::Development),
            _ => None,
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Build => "build",
            Self::Development => "development",
        }
    }

    pub(super) const fn is_production(self) -> bool {
        matches!(self, Self::Normal | Self::Build)
    }
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub(super) struct PolicyFailure {
    pub(super) rule: &'static str,
    pub(super) reason: String,
}

#[derive(Debug)]
pub(super) enum ExternalDecision {
    Allowed,
    NeedsException,
    Denied(PolicyFailure),
}

pub(super) fn local_dependency_policy(
    source: Layer,
    target: Layer,
    kind: DependencyKind,
) -> Option<PolicyFailure> {
    if source == Layer::Tooling || target == Layer::Tooling {
        return Some(PolicyFailure {
            rule: RULE_TOOLING,
            reason: "tooling is isolated from every workspace-local product, benchmark, and tooling package for all dependency kinds".to_owned(),
        });
    }
    if target == Layer::BenchmarkObserver || source == Layer::BenchmarkObserver {
        if source == Layer::BenchmarkObserver
            && target != Layer::BenchmarkObserver
            && kind != DependencyKind::Build
        {
            return None;
        }
        return Some(PolicyFailure {
            rule: if kind == DependencyKind::Build && source == Layer::BenchmarkObserver {
                RULE_BENCHMARK_BUILD
            } else {
                RULE_OBSERVER
            },
            reason: if kind == DependencyKind::Build && source == Layer::BenchmarkObserver {
                "benchmark observers cannot use build dependencies or custom build-time behavior"
                    .to_owned()
            } else {
                "benchmark observers are outer-only consumers: no package may depend on them, and observers may not depend on tooling or peer observers".to_owned()
            },
        });
    }

    if permits_product_edge(source, target) {
        None
    } else {
        Some(PolicyFailure {
            rule: RULE_LAYER_DAG,
            reason: format!(
                "{kind} dependencies must follow the compact role DAG; `{source}` cannot depend on `{target}`"
            ),
        })
    }
}

/// Returns whether a product-role edge follows the generic inward dependency DAG.
pub(super) const fn permits_product_edge(source: Layer, target: Layer) -> bool {
    match source {
        Layer::Tooling | Layer::BenchmarkObserver => false,
        Layer::DomainFoundation => matches!(target, Layer::DomainFoundation),
        Layer::DomainFeature => {
            matches!(target, Layer::DomainFoundation | Layer::DomainFeature)
        }
        Layer::Platform => {
            matches!(target, Layer::DomainFoundation | Layer::DomainFeature)
        }
        Layer::Adapter => matches!(
            target,
            Layer::DomainFoundation | Layer::DomainFeature | Layer::Platform
        ),
        Layer::RuntimeFoundation => matches!(
            target,
            Layer::DomainFoundation | Layer::DomainFeature | Layer::Platform | Layer::Adapter
        ),
        Layer::RuntimeCapability => matches!(
            target,
            Layer::DomainFoundation
                | Layer::DomainFeature
                | Layer::Platform
                | Layer::Adapter
                | Layer::RuntimeFoundation
        ),
        Layer::RuntimeApplication => matches!(
            target,
            Layer::DomainFoundation
                | Layer::DomainFeature
                | Layer::Platform
                | Layer::Adapter
                | Layer::RuntimeFoundation
                | Layer::RuntimeCapability
        ),
        Layer::Application => matches!(target, Layer::RuntimeApplication),
    }
}

pub(super) fn external_dependency_policy(
    source: Layer,
    target: &str,
    kind: DependencyKind,
) -> ExternalDecision {
    if source == Layer::BenchmarkObserver && kind == DependencyKind::Build {
        return ExternalDecision::Denied(PolicyFailure {
            rule: RULE_BENCHMARK_BUILD,
            reason: String::new(),
        });
    }
    if kind == DependencyKind::Development || target_is_sensitive_cuda(target) {
        return ExternalDecision::NeedsException;
    }

    match source {
        Layer::Adapter | Layer::Platform | Layer::Application => ExternalDecision::Allowed,
        Layer::Tooling
        | Layer::BenchmarkObserver
        | Layer::DomainFoundation
        | Layer::DomainFeature
        | Layer::RuntimeFoundation
        | Layer::RuntimeCapability
        | Layer::RuntimeApplication => ExternalDecision::NeedsException,
    }
}

fn target_is_sensitive_cuda(target: &str) -> bool {
    target == "cudarc"
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

#[cfg(test)]
mod tests {
    use super::{DependencyKind, Layer, local_dependency_policy, permits_product_edge};

    #[test]
    fn runtime_layers_allow_only_explicitly_lower_runtime_roles() {
        assert!(permits_product_edge(
            Layer::RuntimeApplication,
            Layer::RuntimeCapability
        ));
        assert!(permits_product_edge(
            Layer::RuntimeCapability,
            Layer::RuntimeFoundation
        ));
        assert!(!permits_product_edge(
            Layer::RuntimeFoundation,
            Layer::RuntimeFoundation
        ));
        assert!(!permits_product_edge(
            Layer::RuntimeCapability,
            Layer::RuntimeCapability
        ));
        assert!(!permits_product_edge(
            Layer::RuntimeApplication,
            Layer::RuntimeApplication
        ));
    }

    #[test]
    fn observer_and_tool_edges_are_denied_for_all_kinds() {
        for kind in [
            DependencyKind::Normal,
            DependencyKind::Build,
            DependencyKind::Development,
        ] {
            assert!(
                local_dependency_policy(Layer::Application, Layer::BenchmarkObserver, kind)
                    .is_some()
            );
            assert!(local_dependency_policy(Layer::Application, Layer::Tooling, kind).is_some());
        }
    }
}
