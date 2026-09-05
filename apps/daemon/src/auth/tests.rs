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
