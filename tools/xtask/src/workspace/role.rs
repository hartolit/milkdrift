use std::fmt;
use std::path::{Path, PathBuf};

use cargo_metadata::Package;

use super::METADATA_NAMESPACE;

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
