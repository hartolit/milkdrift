use super::*;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/daemon-config-v9.toml")
}

fn fixture_document() -> Result<DaemonConfig, Box<dyn std::error::Error>> {
    Ok(toml::from_str(&fs::read_to_string(fixture_path())?)?)
}

#[test]
fn controller_activation_is_explicit_and_production_qualification_cannot_be_claimed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    let path = fixture_path();
    let base = path.parent().ok_or("fixture parent absent")?;
    assert_eq!(
        config.runtime.controller_activation,
        ControllerActivation::Disabled
    );
    config.runtime.controller_activation = ControllerActivation::Enabled;
    let error = config
        .clone()
        .validate(base)
        .err()
        .ok_or("production activation was admitted")?;
    assert!(error.to_string().contains("real external controller loop"));
    config.runtime.controller_activation = ControllerActivation::Qualification;
    let result = config.validate(base);
    if cfg!(feature = "controller-qualification") {
        let plan = result?;
        assert!(plan.redacted_toml().contains("qualification"));
        assert!(
            plan.redacted_toml()
                .contains("per-command/per-request permission")
        );
    } else {
        assert!(
            result
                .err()
                .ok_or("qualification feature was bypassed")?
                .to_string()
                .contains("controller-qualification build feature")
        );
    }
    Ok(())
}

#[test]
fn maintained_operator_configuration_uses_the_production_reader()
-> Result<(), Box<dyn std::error::Error>> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/operator/daemon.toml");
    let plan = DaemonConfig::load(&path)?;
    let document: DaemonConfig = toml::from_str(&fs::read_to_string(&path)?)?;
    assert!(plan.bind().ip().is_loopback());
    assert!(!document.actors[0].authority.dangerous_allow_broad_authority);
    assert_eq!(document.runtime, RuntimeHostConfig::default());
    assert_eq!(document.shutdown, ShutdownConfig::default());
    assert_eq!(
        document.application_receipts,
        ApplicationReceiptConfig::default()
    );
    assert!(document.adapters.process_profiles.is_empty());
    assert!(document.adapters.model_profiles.is_empty());
    assert_eq!(
        plan.storage.data_root,
        path.parent().ok_or("parent")?.canonicalize()?.join("data")
    );
    Ok(())
}

#[test]
fn schema_v9_fixture_is_explicit_safe_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let plan = DaemonConfig::load(&fixture_path())?;
    let document = fixture_document()?;
    let actor = &document.actors[0];
    assert_eq!(document.schema_version, DAEMON_CONFIG_SCHEMA_VERSION);
    assert!(!actor.authority.dangerous_allow_broad_authority);
    assert!(matches!(
        actor.authority.resources.workflow_run,
        WorkflowRunScope::Workflow { .. }
    ));
    assert_eq!(
        actor
            .authority
            .resources
            .capability
            .identity_selection()
            .and_then(milkdrift_authority::Selection::only_values)
            .map(BTreeSet::len),
        Some(1)
    );
    assert_eq!(
        actor
            .authority
            .resources
            .capability
            .operation_selection()
            .and_then(milkdrift_authority::Selection::only_values)
            .map(BTreeSet::len),
        Some(1)
    );
    assert_eq!(actor.authority.budget.concurrency, Some(4));
    let encoded = toml::to_string_pretty(&document)?;
    let decoded: DaemonConfig = toml::from_str(&encoded)?;
    decoded.validate(fixture_path().parent().ok_or("fixture parent absent")?)?;
    assert!(plan.redacted_toml().contains("values = \"[redacted]\""));
    assert_eq!(
        plan.storage.data_root,
        fixture_path()
            .parent()
            .ok_or("fixture parent absent")?
            .canonicalize()?
            .join("test-data")
    );
    assert!(plan.normalized_digest().starts_with("b3_"));
    Ok(())
}

#[test]
fn old_and_future_config_versions_are_rejected_truthfully() -> Result<(), Box<dyn std::error::Error>>
{
    let source = fs::read_to_string(fixture_path())?;
    for unsupported in [
        1_u32, 2_u32, 3_u32, 4_u32, 5_u32, 6_u32, 7_u32, 8_u32, 10_u32,
    ] {
        let directory = tempfile::tempdir()?;
        let value = source.replacen(
            "schema_version = 9",
            &format!("schema_version = {unsupported}"),
            1,
        );
        let path = directory.path().join("daemon.toml");
        fs::write(&path, value)?;
        assert!(matches!(
            DaemonConfig::load(&path),
            Err(ConfigError::UnsupportedVersion(found)) if found == unsupported
        ));
    }
    Ok(())
}

#[test]
fn duplicate_unknown_and_json_configuration_are_rejected() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let duplicate = directory.path().join("duplicate.toml");
    fs::write(&duplicate, "schema_version = 9\nschema_version = 9\n")?;
    assert!(matches!(
        DaemonConfig::load(&duplicate),
        Err(ConfigError::Toml(_))
    ));

    let json = directory.path().join("legacy.json");
    fs::write(&json, r#"{"schema_version":9}"#)?;
    assert!(matches!(
        DaemonConfig::load(&json),
        Err(ConfigError::Toml(_))
    ));

    let unknown = directory.path().join("unknown.toml");
    fs::write(
        &unknown,
        fs::read_to_string(fixture_path())?.replacen(
            "schema_version = 9",
            "schema_version = 9\nunexpected = true",
            1,
        ),
    )?;
    assert!(matches!(
        DaemonConfig::load(&unknown),
        Err(ConfigError::Toml(_))
    ));
    Ok(())
}

#[test]
fn peer_mode_decodes_only_complete_explicit_states() -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(fixture_path())?;
    let enabled = source.replacen(
        "mode = \"disabled\"",
        "mode = \"enabled\"\nlocal_peer_id = \"peer:local\"",
        1,
    );
    let document: DaemonConfig = toml::from_str(&enabled)?;
    assert!(matches!(document.peers, PeerHostConfig::Enabled { .. }));

    let incomplete = source.replacen("mode = \"disabled\"", "mode = \"enabled\"", 1);
    assert!(toml::from_str::<DaemonConfig>(&incomplete).is_err());

    let legacy = source.replacen("mode = \"disabled\"", "enabled = true", 1);
    assert!(toml::from_str::<DaemonConfig>(&legacy).is_err());
    Ok(())
}

#[test]
fn wildcard_or_unbounded_authority_requires_the_dangerous_flag()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    config.actors[0].authority.resources.workflow_run = WorkflowRunScope::Any;
    config.actors[0].authority.budget.duration_ms = None;
    config.actors[0].authority.valid_until = BoundaryTimeMillis::new(u64::MAX);
    config.actors[0].authority.dangerous_allow_broad_authority = false;
    let directory = tempfile::tempdir()?;
    assert!(matches!(
        config.validate(directory.path()),
        Err(ConfigError::Invalid(message))
            if message.contains("dangerous_allow_broad_authority=true")
    ));
    Ok(())
}

#[test]
fn empty_or_legacy_capability_selectors_are_rejected_not_widened()
-> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(fixture_path())?;
    let mut empty_only: toml::Value = toml::from_str(&source)?;
    empty_only["actors"][0]["authority"]["resources"]["capability"]["operations"]["values"] =
        toml::Value::Array(Vec::new());
    assert!(toml::from_str::<DaemonConfig>(&toml::to_string(&empty_only)?).is_err());

    let mut legacy_array: toml::Value = toml::from_str(&source)?;
    legacy_array["actors"][0]["authority"]["resources"]["capability"]["operations"] =
        toml::Value::Array(Vec::new());
    assert!(toml::from_str::<DaemonConfig>(&toml::to_string(&legacy_array)?).is_err());
    Ok(())
}

#[test]
fn daemon_authority_uses_canonical_cross_platform_filesystem_roots()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    config.actors[0].authority.resources.filesystem = vec![FilesystemScope::new(
        "C:/work/tools",
        BTreeSet::from([milkdrift_authority::AccessMode::Execute]),
    )?];
    let encoded = toml::to_string(&config)?;
    let decoded: DaemonConfig = toml::from_str(&encoded)?;
    assert_eq!(
        decoded.actors[0].authority.resources.filesystem[0].root(),
        "C:/work/tools"
    );

    let hostile = encoded.replacen("C:/work/tools", "C:\\\\work\\\\tools", 1);
    assert!(toml::from_str::<DaemonConfig>(&hostile).is_err());
    Ok(())
}

#[test]
fn explicit_capability_wildcard_alone_requires_acknowledgement()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    config.actors[0].authority.resources.capability =
        CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly);
    config.actors[0].authority.dangerous_allow_broad_authority = false;
    let directory = tempfile::tempdir()?;
    assert!(matches!(
        config.validate(directory.path()),
        Err(ConfigError::Invalid(message))
            if message.contains("dangerous_allow_broad_authority=true")
    ));
    Ok(())
}

#[test]
fn deny_all_is_not_broad_and_redaction_preserves_selector_kinds()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    let plan = config
        .clone()
        .validate(fixture_path().parent().ok_or("fixture parent absent")?)?;
    let redacted: toml::Value = toml::from_str(plan.redacted_toml())?;
    assert_eq!(
        redacted["actors"][0]["authority"]["resources"]["capability"]["type"].as_str(),
        Some("allow")
    );
    assert_eq!(
        redacted["actors"][0]["authority"]["resources"]["capability"]["operations"]["type"]
            .as_str(),
        Some("only")
    );
    assert_eq!(
        redacted["secret_sources"]["values"].as_str(),
        Some("[redacted]")
    );

    config.actors[0].authority.resources.capability = CapabilityAuthorityScope::deny_all();
    config.actors[0].authority.dangerous_allow_broad_authority = false;
    let directory = tempfile::tempdir()?;
    config.validate(directory.path())?;
    Ok(())
}

#[test]
fn peer_execution_retention_is_independent_from_application_receipts()
-> Result<(), Box<dyn std::error::Error>> {
    let mut config = fixture_document()?;
    config.application_receipts.hot_receipt_bound = 1;
    config.application_receipts.archive_batch_size = 1;
    config.peers = PeerHostConfig::Enabled {
        local_peer_id: "peer:test".to_owned(),
        relationships: Vec::new(),
        serving: PeerServingConfig {
            maximum_global_active: 64,
            maximum_dispatch_queue: 32,
            maximum_hot_terminal_records: 77,
            archive_batch_size: 7,
            ..PeerServingConfig::default()
        },
    };
    let directory = tempfile::tempdir()?;
    let validated = config.validate(directory.path())?;
    let PeerHostConfig::Enabled { serving, .. } = &validated.peers else {
        return Err("peer serving plan absent".into());
    };
    assert_eq!(serving.maximum_hot_terminal_records, 77);
    assert_eq!(serving.archive_batch_size, 7);
    assert_eq!(validated.storage.application_receipts.hot_receipt_bound, 1);
    Ok(())
}

#[test]
fn non_loopback_plaintext_is_rejected_before_storage() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let secret = directory.path().join("token");
    fs::write(&secret, "secret")?;
    let config = DaemonConfig {
        schema_version: DAEMON_CONFIG_SCHEMA_VERSION,
        data_root: directory.path().join("data"),
        bind: "0.0.0.0:9734".parse()?,
        secret_sources: BTreeMap::from([(
            "credential:operator".to_owned(),
            SecretSourceConfig::File { path: secret },
        )]),
        actors: vec![ActorBindingConfig {
            credential_ref: "credential:operator".to_owned(),
            actor: "human:operator".to_owned(),
            grant_id: "grant:operator".to_owned(),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Controller,
            authority: ActorGrantConfig::dangerous_administrator(),
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
    };
    assert!(config.validate(directory.path()).is_err());
    assert!(!directory.path().join("data").exists());
    Ok(())
}
use milkdrift_authority::{
    BoundaryTimeMillis, CapabilityAuthorityScope, FilesystemScope, WorkflowRunScope,
};
use milkdrift_capability::SideEffectClass;
use std::{collections::BTreeSet, fs};
