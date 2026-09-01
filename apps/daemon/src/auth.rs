use std::{collections::BTreeMap, fmt, sync::Arc};

use milkdrift_authority::{ActorRef, AuthorityGrant, GrantId, SecretRef};
use milkdrift_capability_host::SecretResolver;
use milkdrift_control::{ActorAuthorityContext, AuthorityPreset};
use milkdrift_local_secret::LocalSecretResolver;
use milkdrift_runtime::CommandAuthorityClaim;
use subtle::ConstantTimeEq;

use crate::config::{ActorBindingConfig, AuthenticationPlan, AuthorityPresetConfig, ConfigError};

/// Immutable authenticated server-owned session facts.
#[derive(Clone)]
pub(crate) struct ActorSession {
    pub actor: ActorRef,
    pub context: ActorAuthorityContext,
    pub grant: AuthorityGrant,
    cursor_key: [u8; 32],
}

impl ActorSession {
    pub const fn cursor_key(&self) -> &[u8; 32] {
        &self.cursor_key
    }
}

impl fmt::Debug for ActorSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorSession")
            .field("actor", &self.actor)
            .field("context", &self.context)
            .field("grant", &self.grant.identity())
            .field("cursor_key", &"[redacted]")
            .finish()
    }
}

#[derive(Clone)]
struct Binding {
    reference: SecretRef,
    session: ActorSession,
    enabled: bool,
}

/// Request-time credential verifier supporting file-based rotation and configured revocation.
#[derive(Clone)]
pub(crate) struct AuthRegistry {
    bindings: Arc<Vec<Binding>>,
    resolver: Arc<LocalSecretResolver>,
    grants: Arc<Vec<AuthorityGrant>>,
    revocations: Arc<BTreeMap<GrantId, u64>>,
}

impl fmt::Debug for AuthRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthRegistry")
            .field("bindings", &self.bindings.len())
            .field("credentials", &"[redacted]")
            .finish()
    }
}

impl AuthRegistry {
    pub fn from_plan(config: &AuthenticationPlan) -> Result<Self, ConfigError> {
        let resolver = Arc::new(LocalSecretResolver::new(config.secret_sources.clone()));
        let mut bindings = Vec::with_capacity(config.actors.len());
        let mut grants = Vec::with_capacity(config.actors.len());
        let mut revocations = BTreeMap::new();
        for configured in &config.actors {
            let actor = ActorRef::new(configured.actor.clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
            let grant = grant(configured, &actor)?;
            let session = session(configured, actor, grant.clone())?;
            let reference = SecretRef::new(configured.credential_ref.clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?;
            if !config.secret_sources.contains_key(&reference) {
                return Err(ConfigError::Invalid(
                    "credential source is absent".to_owned(),
                ));
            }
            if configured.enabled {
                resolver
                    .resolve(&reference)
                    .map_err(|_| {
                        ConfigError::Invalid("configured credential is unavailable".to_owned())
                    })?
                    .expose(|bytes| {
                        if bytes.is_empty() {
                            Err(ConfigError::Invalid(
                                "configured credential is empty".to_owned(),
                            ))
                        } else {
                            Ok(())
                        }
                    })?;
            }
            bindings.push(Binding {
                reference,
                session,
                enabled: configured.enabled,
            });
            grants.push(grant);
            if !configured.enabled {
                revocations.insert(
                    GrantId::new(configured.grant_id.clone())
                        .map_err(|error| ConfigError::Invalid(error.to_string()))?,
                    configured.revocation_generation.saturating_add(1),
                );
            }
        }
        Ok(Self {
            bindings: Arc::new(bindings),
            resolver,
            grants: Arc::new(grants),
            revocations: Arc::new(revocations),
        })
    }

    /// Authenticates one exact bearer value with constant-time digest comparison.
    pub fn authenticate(&self, supplied: &[u8]) -> Option<ActorSession> {
        if supplied.is_empty() || supplied.len() > 4_096 {
            return None;
        }
        let supplied_digest = blake3::hash(supplied);
        let mut matched = None;
        for binding in self.bindings.iter() {
            let candidate = self.resolver.resolve(&binding.reference);
            let equal = candidate
                .ok()
                .map(|candidate| {
                    candidate.expose(|bytes| {
                        blake3::hash(bytes)
                            .as_bytes()
                            .ct_eq(supplied_digest.as_bytes())
                            .into()
                    })
                })
                .unwrap_or(false);
            if equal && binding.enabled {
                if matched.is_some() {
                    return None;
                }
                matched = Some(binding.session.clone());
            }
        }
        matched.map(|mut session| {
            session.cursor_key = blake3::derive_key("milkdrift.cursor-key.v1", supplied);
            session
        })
    }

    pub fn grants(&self) -> Vec<AuthorityGrant> {
        self.grants.as_ref().clone()
    }

    pub fn resolver(&self) -> Arc<LocalSecretResolver> {
        self.resolver.clone()
    }

    pub fn revocations(&self) -> BTreeMap<GrantId, u64> {
        self.revocations.as_ref().clone()
    }
}

fn session(
    config: &ActorBindingConfig,
    actor: ActorRef,
    grant: AuthorityGrant,
) -> Result<ActorSession, ConfigError> {
    let grant_id = GrantId::new(config.grant_id.clone())
        .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    let claim = CommandAuthorityClaim::new(
        grant_id,
        config.grant_revision,
        grant
            .digest()
            .map_err(|error| ConfigError::Invalid(error.to_string()))?,
        config.revocation_generation,
    )
    .map_err(|error| ConfigError::Invalid(error.to_string()))?;
    Ok(ActorSession {
        actor: actor.clone(),
        context: ActorAuthorityContext::new(actor, claim),
        grant,
        cursor_key: [0; 32],
    })
}

fn grant(config: &ActorBindingConfig, actor: &ActorRef) -> Result<AuthorityGrant, ConfigError> {
    let preset = match config.preset {
        AuthorityPresetConfig::Observer => AuthorityPreset::Observer,
        AuthorityPresetConfig::Advisor => AuthorityPreset::Advisor,
        AuthorityPresetConfig::Supervisor => AuthorityPreset::Supervisor,
        AuthorityPresetConfig::Controller => AuthorityPreset::Controller,
    };
    preset
        .template(
            GrantId::new(config.grant_id.clone())
                .map_err(|error| ConfigError::Invalid(error.to_string()))?,
            config.grant_revision,
            actor.clone(),
            config.authority.resources.workflow_run.clone(),
            config.authority.resources.capability.clone(),
            config.authority.budget,
        )
        .resources(config.authority.resources.clone())
        .validity(config.authority.valid_from, config.authority.valid_until)
        .revocation_generation(config.revocation_generation)
        .build()
        .map_err(|error| ConfigError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AdapterConfig, ApplicationReceiptConfig, DaemonConfig, PeerHostConfig, RuntimeHostConfig,
        SecretSourceConfig, ShutdownConfig,
    };
    use std::{collections::BTreeMap, fs, net::SocketAddr};

    fn config(root: &std::path::Path, token: &std::path::Path) -> DaemonConfig {
        DaemonConfig {
            schema_version: crate::DAEMON_CONFIG_SCHEMA_VERSION,
            data_root: root.join("data"),
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            secret_sources: BTreeMap::from([(
                "credential:operator".to_owned(),
                SecretSourceConfig::File {
                    path: token.to_path_buf(),
                },
            )]),
            actors: vec![ActorBindingConfig {
                credential_ref: "credential:operator".to_owned(),
                actor: "human:operator".to_owned(),
                grant_id: "grant:operator".to_owned(),
                grant_revision: 1,
                revocation_generation: 0,
                preset: AuthorityPresetConfig::Controller,
                authority: crate::config::ActorGrantConfig::dangerous_administrator(),
                enabled: true,
            }],
            runtime: RuntimeHostConfig::default(),
            adapters: AdapterConfig::default(),
            peers: PeerHostConfig::default(),
            shutdown: ShutdownConfig::default(),
            application_receipts: ApplicationReceiptConfig {
                hot_receipt_bound: 100,
                archive_batch_size: 10,
            },
            security_audit_record_bound: 100,
        }
    }

    #[test]
    fn invalid_token_rotation_and_redaction() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let token = root.path().join("token");
        fs::write(&token, "first-token")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
        }
        let validated = config(root.path(), &token).validate(root.path())?;
        let parts = validated.into_parts();
        let registry = AuthRegistry::from_plan(&parts.authentication)?;
        assert!(registry.authenticate(b"wrong").is_none());
        assert!(registry.authenticate(b"first-token").is_some());
        fs::write(&token, "second-token")?;
        assert!(registry.authenticate(b"first-token").is_none());
        assert!(registry.authenticate(b"second-token").is_some());
        assert!(!format!("{registry:?}").contains("second-token"));
        Ok(())
    }
}
