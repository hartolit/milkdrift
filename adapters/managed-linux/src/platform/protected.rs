//! Protected single-artifact applications. Working-area adapters never publish a descriptor for
//! these installations; only the managed owner may prepare or activate their read-only bytes.
mod service;
mod verification;
use super::{LinuxManagedPlatform, private_directory, regular_file};
use crate::{ContainerLimits, digest, recipe, rejected};
use milkdrift_authority::{
    AccessMode, CapabilityExecutionRequirements, FilesystemScope, ProtectedEffectPolicy,
};
use milkdrift_capability::{
    BoundedJson,
    managed::{
        DataDisposition, ManagedName, ManagedResourceKind, ManagedResourceView, RecipeReference,
        ResourceOwnership,
    },
};
use milkdrift_capability_host::managed::ManagedError;
use milkdrift_persistence::managed::{
    ApprovedSetup, InstallationRecord, ManagedObservation, ManagedStep, ProtectedDeployment,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Operator-approved target and separate verifier. Candidate bytes are supplied through artifact
/// authority at evaluation, never through an agent-writable served path or an unbound rebuild.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedServiceRecipe {
    /// Exact format for the protected service mechanism.
    pub schema_version: u32,
    /// Must be `protected_service`; distinguishes this closed recipe from working environments.
    pub kind: String,
    /// Operator recipe identity.
    pub name: ManagedName,
    /// Exact preloaded runtime image, including the interpreter and libraries.
    pub image: String,
    /// Container-internal interpreter executable; the candidate is its sole source argument.
    pub executable: String,
    /// Explicit independent service limits.
    pub limits: ContainerLimits,
    /// Bounded lifecycle timeouts.
    pub startup_ms: u64,
    /// Bounded supervisor stop grace.
    pub shutdown_ms: u64,
    /// Private host-loopback port.
    pub port: u16,
    /// Non-secret application configuration; included in candidate applicability.
    pub application: BoundedJson,
    /// Synthetic test credential file outside every worker mount.
    pub token_file: PathBuf,
    /// Exact synthetic credential generation, bound without disclosing its bytes.
    pub token_digest: String,
    /// Operator-controlled UTC test clock file outside every worker mount.
    pub clock_file: PathBuf,
    /// Immutable agreement accepted for this target.
    pub agreement: String,
    /// Fixed check set, trusted verifier generation and evidence lifetime.
    pub policy: ProtectedEffectPolicy,
    /// Operator-owned Python verifier source, executed outside the candidate's container.
    pub verifier_file: PathBuf,
    /// Exact operator-owned verifier source bytes.
    pub verifier_source_digest: String,
    /// Exact native isolated-mode Python executable used by the trusted host.
    pub verifier_runtime_digest: String,
    /// Maximum verifier process duration, including its isolated service stop/start checks.
    /// Platform fencing after exit or timeout uses the bounded administrative helper deadline.
    pub verification_timeout_ms: u64,
    /// Explicit retained data disposition.
    pub data_disposition: DataDisposition,
}
impl ProtectedServiceRecipe {
    /// Strict bounded operator reader; accepts no raw engine arguments or writable served mounts.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ManagedError> {
        if bytes.len() > 65_536 {
            return Err(rejected("protected recipe exceeds 64 KiB"));
        }
        let value = milkdrift_contracts::parse_json_without_duplicates(bytes).map_err(rejected)?;
        let recipe: Self = serde_json::from_value(value).map_err(rejected)?;
        recipe.validate()?;
        Ok(recipe)
    }
    fn validate(&self) -> Result<(), ManagedError> {
        self.policy.validate().map_err(rejected)?;
        self.limits.validate("protected service limits")?;
        crate::recipe::validate_timeout(self.startup_ms, "protected startup_ms")?;
        crate::recipe::validate_timeout(self.shutdown_ms, "protected shutdown_ms")?;
        if self.schema_version != 1
            || self.kind != "protected_service"
            || !recipe::exact_image(&self.image)
            || !recipe::safe_absolute(Path::new(&self.executable))
            || !recipe::safe_absolute(&self.token_file)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.token_digest)
            || !recipe::safe_absolute(&self.clock_file)
            || !recipe::safe_absolute(&self.verifier_file)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.verifier_source_digest)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.verifier_runtime_digest)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.agreement)
            || self.port < 1024
            || self.verification_timeout_ms == 0
            || self.verification_timeout_ms > 600_000
        {
            return Err(rejected("invalid protected service recipe"));
        }
        let identity = serde_json::json!({"source":self.verifier_source_digest,"runtime":self.verifier_runtime_digest});
        if self.policy.verifier != digest(serde_json::to_vec(&identity).map_err(rejected)?) {
            return Err(rejected(
                "verifier generation differs from pinned source/runtime",
            ));
        }
        Ok(())
    }
    /// Exact approved reference. The verifier source hash is separately checked at every entry.
    pub fn reference(&self) -> Result<RecipeReference, ManagedError> {
        self.validate()?;
        let bytes = milkdrift_contracts::canonical_json_bytes(
            self,
            milkdrift_contracts::JsonLimits {
                maximum_depth: 12,
                maximum_string_bytes: 4096,
                maximum_key_bytes: 128,
                maximum_container_items: 64,
            },
        )
        .map_err(|e| rejected(format!("{e:?}")))?;
        Ok(RecipeReference {
            name: self.name.clone(),
            digest: digest(bytes),
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    recipe: ProtectedServiceRecipe,
    installation: ManagedName,
    generation: u64,
    unit: String,
    data_volume: String,
    root: PathBuf,
    systemd_directory: PathBuf,
    candidate: Option<PathBuf>,
}
fn decode(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<Deployment, ManagedError> {
    let d: Deployment =
        serde_json::from_value(setup.configuration.value().clone()).map_err(rejected)?;
    let prefix = crate::units::volume_prefix(&platform.owner, &setup.ownership);
    if setup.mechanism != "linux-protected-service-v1"
        || setup.platform_owner != platform.owner
        || d.root != platform.config.state_root.join(&prefix)
        || d.systemd_directory != platform.config.systemd_directory
        || d.unit != format!("{prefix}g{}", d.generation)
        || d.data_volume != format!("{prefix}data")
        || d.recipe.reference()? != setup.recipe
        || d.generation == 0
        || !setup.capabilities.is_empty()
        || setup
            .protection
            .as_ref()
            .is_none_or(|p| p.agreement != d.recipe.agreement || p.policy != d.recipe.policy)
        || {
            let expected = resources(&d)?;
            setup.resources.len() != expected.len()
                || setup.resources.iter().zip(&expected).any(|(a, b)| {
                    a.name != b.name
                        || a.kind != b.kind
                        || a.ownership != b.ownership
                        || a.identity != b.identity
                })
        }
    {
        return Err(rejected(
            "protected deployment differs from owned configuration",
        ));
    }
    if let Some(e) = setup.protection.as_ref().and_then(|p| p.evidence.as_ref()) {
        let expected = d.root.join(format!("{}.py", e.subject.artifact.digest()));
        if d.candidate.as_ref() != Some(&expected) {
            return Err(rejected(
                "protected candidate path differs from immutable evidence",
            ));
        }
    } else if d.candidate.is_some() {
        return Err(rejected("candidate has no accepted evidence"));
    }
    private_directory(&platform.config.state_root)?;
    crate::units::systemd_directory(&d.systemd_directory)?;
    Ok(d)
}
fn resources(d: &Deployment) -> Result<Vec<ManagedResourceView>, ManagedError> {
    [
        ("data", ManagedResourceKind::Data, d.data_volume.clone()),
        ("application", ManagedResourceKind::Service, d.unit.clone()),
    ]
    .into_iter()
    .map(|(name, kind, identity)| {
        Ok(ManagedResourceView {
            name: ManagedName::new(name).map_err(rejected)?,
            kind,
            identity,
            ownership: ResourceOwnership::Owned,
            disposition: d.recipe.data_disposition,
        })
    })
    .collect()
}
pub(super) fn plan(
    platform: &LinuxManagedPlatform,
    installation: &ManagedName,
    recipe: &ProtectedServiceRecipe,
    ownership: &str,
    generation: u64,
) -> Result<ApprovedSetup, ManagedError> {
    let prefix = crate::units::volume_prefix(&platform.owner, ownership);
    let d = Deployment {
        recipe: recipe.clone(),
        installation: installation.clone(),
        generation,
        unit: format!("{prefix}g{generation}"),
        data_volume: format!("{prefix}data"),
        root: platform.config.state_root.join(prefix),
        systemd_directory: platform.config.systemd_directory.clone(),
        candidate: None,
    };
    let setup = ApprovedSetup {
        protection: Some(ProtectedDeployment {
            agreement: recipe.agreement.clone(),
            policy: recipe.policy.clone(),
            evidence: None,
        }),
        recipe: recipe.reference()?,
        mechanism: "linux-protected-service-v1".to_owned(),
        configuration: BoundedJson::new(serde_json::to_value(&d).map_err(rejected)?)
            .map_err(rejected)?,
        platform_owner: platform.owner.clone(),
        ownership: ownership.to_owned(),
        resources: resources(&d)?,
        capabilities: Vec::new(),
    };
    setup.validate().map_err(rejected)?;
    Ok(setup)
}
pub(super) fn diagnose(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<Vec<String>, ManagedError> {
    active_policy(platform, setup)?;
    let d = decode(platform, setup)?;
    #[cfg(target_os = "linux")]
    d.recipe.limits.validate_page_alignment(
        rustix::param::page_size() as u64,
        "protected service limits",
    )?;
    // Verification can run while the last accepted generation remains live.
    let version = LinuxManagedPlatform::diagnose_host(2, true)?;
    LinuxManagedPlatform::image_identity(&d.recipe.image)?;
    Ok(vec![
        format!(
            "Podman {version}; delegated cpu/memory/pids; private user namespaces for service and verifier; user lingering"
        ),
        "protected target policy, verifier generation and local image verified".to_owned(),
    ])
}

pub(super) fn active_policy(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<(), ManagedError> {
    let d = decode(platform, setup)?;
    // Re-read operator approval, not merely the startup cache: removal or change revokes future entry.
    let approved = platform.config.recipes.iter().any(|path| {
        regular_file(path, 65_536).is_ok()
            && fs::read(path)
                .ok()
                .and_then(|bytes| ProtectedServiceRecipe::from_json(&bytes).ok())
                .and_then(|r| r.reference().ok())
                .as_ref()
                == Some(&setup.recipe)
    });
    if !approved {
        return Err(rejected("protected target approval changed or was revoked"));
    }
    regular_file(&d.recipe.verifier_file, 1_048_576)?;
    let bytes = fs::read(&d.recipe.verifier_file).map_err(rejected)?;
    if digest(&bytes) != d.recipe.verifier_source_digest {
        return Err(rejected("trusted verifier generation changed"));
    }
    let runtime = Path::new("/usr/bin/python3");
    let metadata = fs::metadata(runtime).map_err(rejected)?;
    if !metadata.is_file()
        || metadata.len() > 16_777_216
        || digest(fs::read(runtime).map_err(rejected)?) != d.recipe.verifier_runtime_digest
    {
        return Err(rejected("trusted verifier runtime generation changed"));
    }
    regular_file(&d.recipe.token_file, 1024)?;
    if digest(fs::read(&d.recipe.token_file).map_err(rejected)?) != d.recipe.token_digest {
        return Err(rejected("protected credential generation changed"));
    }
    regular_file(&d.recipe.clock_file, 128)?;
    Ok(())
}
fn immutable(path: &Path, bytes: &[u8]) -> Result<(), ManagedError> {
    if path.exists() {
        regular_file(path, bytes.len() as u64)?;
        if fs::read(path).map_err(rejected)? != bytes {
            return Err(rejected("immutable candidate/configuration drift"));
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| rejected("candidate parent absent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(rejected)?;
    file.write_all(bytes).map_err(rejected)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o444))
            .map_err(rejected)?;
    }
    file.as_file().sync_all().map_err(rejected)?;
    file.persist_noclobber(path).map_err(rejected)?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(rejected)?;
    Ok(())
}
fn ensure_root(d: &Deployment) -> Result<(), ManagedError> {
    if !d.root.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&d.root).map_err(rejected)?;
    }
    private_directory(&d.root)
}
pub(super) fn prepare(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
    evidence: &milkdrift_workspace::CandidateEvaluation,
    bytes: &[u8],
    generation: u64,
) -> Result<ApprovedSetup, ManagedError> {
    active_policy(platform, setup)?;
    let old = decode(platform, setup)?;
    if !evidence.subject.artifact.verifies(bytes) {
        return Err(rejected("candidate bytes differ"));
    }
    let mut candidate = plan(
        platform,
        &old.installation,
        &old.recipe,
        &setup.ownership,
        generation,
    )?;
    let mut d = decode(platform, &candidate)?;
    ensure_root(&d)?;
    let path = d
        .root
        .join(format!("{}.py", evidence.subject.artifact.digest()));
    immutable(&path, bytes)?;
    d.candidate = Some(path);
    candidate.configuration =
        BoundedJson::new(serde_json::to_value(d).map_err(rejected)?).map_err(rejected)?;
    candidate
        .protection
        .as_mut()
        .ok_or_else(|| rejected("protection absent"))?
        .evidence = Some(evidence.clone());
    Ok(candidate)
}
pub(super) fn requirements(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<CapabilityExecutionRequirements, ManagedError> {
    let d = decode(platform, setup)?;
    Ok(CapabilityExecutionRequirements {
        filesystem: vec![
            FilesystemScope::from_canonical_host_path(
                &platform.config.state_root,
                std::collections::BTreeSet::from([AccessMode::Read, AccessMode::Write]),
            )
            .map_err(rejected)?,
            FilesystemScope::from_canonical_host_path(
                &d.systemd_directory,
                std::collections::BTreeSet::from([AccessMode::Read, AccessMode::Write]),
            )
            .map_err(rejected)?,
        ],
        ..Default::default()
    })
}
pub(super) fn reconcile(
    platform: &LinuxManagedPlatform,
    record: &InstallationRecord,
    step: ManagedStep,
) -> Result<ManagedObservation, ManagedError> {
    service::reconcile(platform, record, step)
}
pub(super) fn observe(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
) -> Result<ManagedObservation, ManagedError> {
    service::observe(platform, setup)
}
pub(super) fn evaluate(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
    evaluation: &milkdrift_workspace::CandidateEvaluation,
    bytes: &[u8],
) -> Result<Vec<milkdrift_workspace::CandidateCheck>, ManagedError> {
    verification::evaluate(platform, setup, evaluation, bytes)
}

pub(super) fn recover_verifications(platform: &LinuxManagedPlatform) -> Result<(), ManagedError> {
    verification::recover(platform)
}
