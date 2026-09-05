//! Version refusal, path normalization and compilation into immutable owner plans.
use super::{
    ActorGrantConfig, AuthenticationPlan, CONFIG_DIGEST_LIMITS, ConfigError,
    DAEMON_CONFIG_SCHEMA_VERSION, DaemonConfig, DaemonPlan, PeerHostConfig, RuntimeHostConfig,
    SecretSourceConfig, StoragePlan, redaction::redacted_toml, redaction::redacted_value,
};
use milkdrift_authority::{
    ArtifactAuthorityScope, FilesystemScope, NetworkScope, PeerAuthorityScope, SecretRef,
    WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_capability::SideEffectClass;
use milkdrift_contracts::canonical_json_bytes;
use milkdrift_control_protocol::MAX_DOCUMENT_BYTES;
use milkdrift_local_secret::LocalSecretSource;
use milkdrift_peer_protocol::PROTOCOL_MINOR_V1;
use std::{
    collections::BTreeMap, collections::BTreeSet, fs, path::Component, path::Path, path::PathBuf,
};

impl DaemonConfig {
    /// Loads bounded duplicate-safe TOML and compiles it before storage is opened.
    pub fn load(path: &Path) -> Result<DaemonPlan, ConfigError> {
        let bytes = fs::read(path).map_err(|error| ConfigError::Read(error.kind().to_string()))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(ConfigError::Invalid(format!(
                "configuration exceeds {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        let source =
            std::str::from_utf8(&bytes).map_err(|error| ConfigError::Toml(error.to_string()))?;
        let config: Self =
            toml::from_str(source).map_err(|error| ConfigError::Toml(error.to_string()))?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        config.validate(parent)
    }

    /// Deterministically validates and normalizes a programmatically built config.
    pub fn validate(mut self, base: &Path) -> Result<DaemonPlan, ConfigError> {
        if self.schema_version != DAEMON_CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.schema_version));
        }
        if !self.bind.ip().is_loopback() {
            return Err(ConfigError::Invalid(
                "plaintext HTTP bind must be loopback".to_owned(),
            ));
        }
        if self.actors.is_empty() || self.actors.len() > 256 {
            return Err(ConfigError::Invalid(
                "authentication requires 1..=256 actor bindings".to_owned(),
            ));
        }
        if self.secret_sources.is_empty() || self.secret_sources.len() > 512 {
            return Err(ConfigError::Invalid(
                "secret source count must be in 1..=512".to_owned(),
            ));
        }
        validate_runtime(&self.runtime)?;
        if self.shutdown.deadline_ms == 0 || self.shutdown.deadline_ms > 300_000 {
            return Err(ConfigError::Invalid(
                "shutdown deadline must be in 1..=300000 milliseconds".to_owned(),
            ));
        }
        if self.application_receipts.hot_receipt_bound == 0
            || self.application_receipts.hot_receipt_bound > 1_000_000
            || self.application_receipts.archive_batch_size == 0
            || self.application_receipts.archive_batch_size
                > self.application_receipts.hot_receipt_bound
        {
            return Err(ConfigError::Invalid(
                "application hot receipt bound must be in 1..=1000000 and archive batch must be in 1..=hot bound"
                    .to_owned(),
            ));
        }
        if self.security_audit_record_bound == 0 || self.security_audit_record_bound > 1_000_000 {
            return Err(ConfigError::Invalid(
                "security audit record bound must be in 1..=1000000".to_owned(),
            ));
        }
        let base = base
            .canonicalize()
            .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
        self.data_root = normalize_owned_path(&base, &self.data_root)?;
        for source in self.secret_sources.values_mut() {
            if let SecretSourceConfig::File { path } = source {
                *path = normalize_existing_file(&base, path)?;
            }
        }
        let mut actors = BTreeSet::new();
        let mut grants = BTreeSet::new();
        let mut credential_refs = BTreeSet::new();
        for actor in &self.actors {
            validate_safe_identity("credential_ref", &actor.credential_ref)?;
            validate_safe_identity("actor", &actor.actor)?;
            validate_safe_identity("grant_id", &actor.grant_id)?;
            if actor.grant_revision == 0
                || !actors.insert(&actor.actor)
                || !grants.insert((&actor.grant_id, actor.grant_revision))
                || !credential_refs.insert(&actor.credential_ref)
                || !self.secret_sources.contains_key(&actor.credential_ref)
            {
                return Err(ConfigError::Invalid(
                    "actor, grant, credential reference, or revision mapping is invalid/duplicate"
                        .to_owned(),
                ));
            }
            validate_actor_authority(&actor.authority)?;
        }
        for path in &mut self.adapters.process_profiles {
            *path = normalize_existing_file(&base, path)?;
        }
        for model in &mut self.adapters.model_profiles {
            validate_safe_identity("model capability", &model.capability_id)?;
            model.profile = normalize_existing_file(&base, &model.profile)?;
        }
        validate_peers(&self.peers, &self.secret_sources)?;
        let redacted_toml = redacted_toml(&redacted_value(&self)?)?;
        let normalized = canonical_json_bytes(&self, CONFIG_DIGEST_LIMITS).map_err(|error| {
            ConfigError::Invalid(format!(
                "effective configuration cannot be canonicalized: {error:?}"
            ))
        })?;
        let normalized_digest = format!("b3_{}", blake3::hash(&normalized).to_hex());
        let local_secret_sources = self
            .secret_sources
            .iter()
            .map(|(reference, source)| {
                let reference = SecretRef::new(reference.clone())
                    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
                let source = match source {
                    SecretSourceConfig::Environment { variable } => {
                        LocalSecretSource::environment(variable.clone())
                    }
                    SecretSourceConfig::File { path } => LocalSecretSource::file(path.clone()),
                }
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
                Ok((reference, source))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        Ok(DaemonPlan {
            bind: self.bind,
            storage: StoragePlan {
                data_root: self.data_root,
                application_receipts: self.application_receipts,
                security_audit_record_bound: self.security_audit_record_bound,
            },
            authentication: AuthenticationPlan {
                secret_sources: local_secret_sources,
                actors: self.actors,
            },
            runtime: self.runtime,
            adapters: self.adapters,
            peers: self.peers,
            shutdown: self.shutdown,
            redacted_toml,
            normalized_digest,
        })
    }
}

fn validate_actor_authority(authority: &ActorGrantConfig) -> Result<(), ConfigError> {
    NetworkScope::new(
        authority.resources.network.profiles().clone(),
        authority.resources.network.destinations().clone(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    for filesystem in &authority.resources.filesystem {
        milkdrift_authority::FilesystemScope::new(
            filesystem.root().to_owned(),
            filesystem.access().clone(),
        )
        .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    }
    match (
        authority.resources.artifacts.identity_selection(),
        authority.resources.artifacts.sensitivities(),
    ) {
        (Some(identities), Some(sensitivities)) => {
            ArtifactAuthorityScope::new(identities.clone(), sensitivities.clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        }
        (None, None) => {}
        _ => {
            return Err(ConfigError::Invalid(
                "artifact authority selector and sensitivity scope disagree".to_owned(),
            ));
        }
    }
    PeerAuthorityScope::new(
        authority.resources.peers.identities().clone(),
        authority.resources.peers.allows_any(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    WorkspaceAuthorityScope::new(
        authority.resources.workspace.scopes().clone(),
        authority.resources.workspace.allows_any_in_run(),
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    if authority.valid_from > authority.valid_until {
        return Err(ConfigError::Invalid(
            "actor authority validity interval is inverted".to_owned(),
        ));
    }
    if authority.dangerous_allow_broad_authority {
        return Ok(());
    }
    let capability = &authority.resources.capability;
    let broad_workflow = matches!(authority.resources.workflow_run, WorkflowRunScope::Any);
    let broad_capability = !capability.denies_all()
        && ((capability
            .identity_selection()
            .is_some_and(milkdrift_authority::Selection::is_any)
            && capability
                .category_selection()
                .is_some_and(milkdrift_authority::Selection::is_any))
            || capability
                .operation_selection()
                .is_some_and(milkdrift_authority::Selection::is_any)
            || capability.maximum_side_effect() == SideEffectClass::Unknown);
    let unbounded_budget = authority.budget.cost_minor.is_none()
        || authority.budget.duration_ms.is_none()
        || authority.budget.invocations.is_none()
        || authority.budget.artifact_bytes.is_none()
        || authority.budget.units.is_none()
        || authority.budget.concurrency.is_none();
    let effectively_unbounded = authority.valid_until.get() == u64::MAX;
    let broad_reads = authority.resources.artifacts.has_any_selector()
        || authority.resources.peers.allows_any()
        || authority.resources.workspace.allows_any_in_run()
        || authority.resources.layouts.has_any_selector();
    if broad_workflow
        || broad_capability
        || broad_reads
        || unbounded_budget
        || effectively_unbounded
    {
        return Err(ConfigError::Invalid(
            "broad workflow/capability/read scope, unknown side effects, omitted ceilings, or infinite validity requires dangerous_allow_broad_authority=true"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_peers(
    peers: &PeerHostConfig,
    secrets: &BTreeMap<String, SecretSourceConfig>,
) -> Result<(), ConfigError> {
    let PeerHostConfig::Enabled {
        local_peer_id,
        relationships,
        serving,
    } = peers
    else {
        return Ok(());
    };
    if serving.worker_threads == 0
        || serving.worker_threads > 256
        || serving.maximum_global_active == 0
        || serving.maximum_dispatch_queue == 0
        || serving.maximum_dispatch_queue > serving.maximum_global_active
        || serving.maximum_hot_terminal_records < u64::from(serving.maximum_global_active)
        || serving.maximum_hot_terminal_records > 1_000_000
        || serving.archive_batch_size == 0
        || u64::from(serving.archive_batch_size) > serving.maximum_hot_terminal_records
        || serving.observation_hot_retention_ms == 0
        || serving.observation_hot_retention_ms > 31_536_000_000
        || serving.recovery_page == 0
        || serving.poll_interval_ms == 0
        || serving.poll_interval_ms > 60_000
    {
        return Err(ConfigError::Invalid(
            "peer serving active/queue/hot/archive/recovery bounds are invalid".to_owned(),
        ));
    }
    validate_safe_identity("local_peer_id", local_peer_id)?;
    if relationships.len() > 256 {
        return Err(ConfigError::Invalid(
            "peer relationship count must not exceed 256".to_owned(),
        ));
    }
    let mut identities = BTreeSet::new();
    for relationship in relationships {
        validate_safe_identity("peer_id", &relationship.peer_id)?;
        validate_safe_identity("peer credential_ref", &relationship.credential_ref)?;
        validate_safe_identity("peer trust_zone", &relationship.trust_zone)?;
        validate_safe_identity("peer delegation_ref", &relationship.delegation_ref)?;
        if relationship.peer_id == *local_peer_id
            || !identities.insert(&relationship.peer_id)
            || !secrets.contains_key(&relationship.credential_ref)
            || relationship.minimum_minor != PROTOCOL_MINOR_V1
            || relationship.maximum_minor != PROTOCOL_MINOR_V1
            || relationship.maximum_concurrent == 0
            || relationship.maximum_requests_per_minute == 0
            || relationship.maximum_requests_per_minute > 100_000
            || relationship.maximum_artifact_bytes == 0
            || relationship.maximum_duration_ms == 0
            || relationship.maximum_observations == 0
            || relationship.catalog_ttl_ms == 0
            || relationship.catalog_ttl_ms > 300_000
            || relationship.expires_at_unix_ms == 0
        {
            return Err(ConfigError::Invalid(
                "peer identity, credential, version, quota, TTL, or expiry is invalid".to_owned(),
            ));
        }
        for capability in relationship
            .capability_allow
            .iter()
            .chain(&relationship.capability_deny)
        {
            validate_safe_identity("peer capability filter", capability)?;
        }
        for operation in &relationship.operation_allow {
            validate_safe_identity("peer operation filter", operation)?;
        }
        for filesystem in &relationship.execution_filesystem {
            FilesystemScope::new(filesystem.root(), filesystem.access().clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        }
        NetworkScope::new(
            relationship.execution_network_profiles.clone(),
            relationship.execution_network_destinations.clone(),
        )
        .map_err(|error| ConfigError::Invalid(error.to_string()))?;
        if relationship
            .execution_secrets
            .iter()
            .any(|secret| !secrets.contains_key(secret.as_str()))
        {
            return Err(ConfigError::Invalid(
                "peer execution authority references an unknown secret source".to_owned(),
            ));
        }
        let endpoint = url::Url::parse(&relationship.endpoint)
            .map_err(|error| ConfigError::Invalid(format!("invalid peer endpoint: {error}")))?;
        let plaintext_loopback = endpoint.scheme() == "http"
            && relationship.insecure_loopback_development
            && matches!(
                endpoint.host(),
                Some(url::Host::Ipv4(address)) if address.is_loopback()
            )
            || endpoint.scheme() == "http"
                && relationship.insecure_loopback_development
                && matches!(
                    endpoint.host(),
                    Some(url::Host::Ipv6(address)) if address.is_loopback()
                )
            || endpoint.scheme() == "http"
                && relationship.insecure_loopback_development
                && endpoint
                    .host_str()
                    .is_some_and(|host| host.eq_ignore_ascii_case("localhost"));
        if endpoint.scheme() != "https" && !plaintext_loopback {
            return Err(ConfigError::Invalid(
                "peer endpoint must use HTTPS unless insecure loopback development mode is explicit"
                    .to_owned(),
            ));
        }
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ConfigError::Invalid(
                "peer endpoint must not contain credentials or fragments".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_runtime(config: &RuntimeHostConfig) -> Result<(), ConfigError> {
    if config.request_queue == 0
        || config.request_queue > 65_536
        || config.maintenance_interval_ms == 0
        || config.maintenance_interval_ms > 60_000
        || config.maximum_tick_items == 0
        || config.maximum_tick_items > 1_000
        || config.global_concurrency == 0
        || config.per_run_concurrency == 0
        || config.per_run_concurrency > config.global_concurrency
        || config.per_branch_concurrency == 0
        || config.per_branch_concurrency > config.per_run_concurrency
        || config.per_capability_concurrency == 0
        || config.effect_threads == 0
        || config.effect_threads > 256
        || config.effect_queue == 0
        || config.cancellation_queue == 0
        || config.maximum_effect_claim == 0
        || config.lease_duration_ms == 0
    {
        return Err(ConfigError::Invalid(
            "runtime scheduler, queue, lease, or worker bounds are invalid".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_owned_path(base: &Path, path: &Path) -> Result<PathBuf, ConfigError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(ConfigError::Invalid(
                    "configured paths must not contain parent traversal".to_owned(),
                ));
            }
            Component::CurDir => {}
            component => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn normalize_existing_file(base: &Path, path: &Path) -> Result<PathBuf, ConfigError> {
    let path = normalize_owned_path(base, path)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| ConfigError::Read(error.kind().to_string()))?;
    if !canonical.is_file() {
        return Err(ConfigError::Invalid(
            "configured source path is not a regular file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn validate_safe_identity(location: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 192
        || !value.is_ascii()
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(ConfigError::Invalid(format!(
            "{location} is not a safe bounded identity"
        )));
    }
    Ok(())
}
