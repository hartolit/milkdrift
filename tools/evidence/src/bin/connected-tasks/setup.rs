use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, hash_file, reserve_endpoint, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) struct Configured {
    pub config: PathBuf,
    pub endpoint: String,
    pub token: String,
}

pub(super) fn fresh_root(path: &Path) -> EvidenceResult<PathBuf> {
    ensure(
        !path.try_exists()?,
        "browser fixture root already exists; choose a new empty root, never reset an existing generation",
    )?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path)?;
    private_directory(path)?;
    Ok(path.canonicalize()?)
}

pub(super) fn configure(
    root: &Path,
    executable: &Path,
    name: &str,
    model: &super::model::Model,
) -> EvidenceResult<Configured> {
    let directory = root.join(format!("daemon-{name}"));
    fs::create_dir(&directory)?;
    private_directory(&directory)?;
    let mut entropy = [0u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| "operating-system token entropy unavailable")?;
    let token = blake3::hash(&entropy).to_hex().to_string();
    write_private(&directory.join("operator.token"), token.as_bytes())?;
    let profile = process_profile(&directory, executable)?;
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let template: toml::Value = toml::from_str(&fs::read_to_string(
        repository.join("examples/operator/daemon.toml"),
    )?)?;
    let mut configuration = serde_json::to_value(template)?;
    let endpoint = reserve_endpoint()?;
    set(
        &mut configuration,
        "/schema_version",
        json!(milkdrift_daemon::DAEMON_CONFIG_SCHEMA_VERSION),
    )?;
    set(
        &mut configuration,
        "/data_root",
        json!(directory.join("data")),
    )?;
    set(&mut configuration, "/bind", json!(endpoint.to_string()))?;
    set(
        &mut configuration,
        "/host_id",
        json!(format!("host:browser-{name}")),
    )?;
    let serving_default = serde_json::to_value(milkdrift_daemon::ServingHostConfig::default())?;
    configuration
        .as_object_mut()
        .ok_or("config is not an object")?
        .entry("serving")
        .or_insert(serving_default)
        .as_object_mut()
        .ok_or("serving is not an object")?
        .insert(
            "browser_origins".into(),
            json!(["http://127.0.0.1:5173", "http://127.0.0.1:4173"]),
        );
    set(
        &mut configuration,
        "/secret_sources",
        json!({"credential:operator":{"type":"file","path":directory.join("operator.token")}}),
    )?;
    set(
        &mut configuration,
        "/actors/0/actor",
        json!(format!("human:browser-{name}")),
    )?;
    set(
        &mut configuration,
        "/actors/0/grant_id",
        json!(format!("grant:browser-{name}")),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/workflow_run",
        json!({"type":"any"}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/capability/identities",
        json!({"type":"only","values":["cargo-report","controlled-model"]}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/capability/maximum_side_effect",
        json!("unknown"),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/capability/operations",
        json!({"type":"only","values":["process.execute","model.generate"]}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/capability/trust_zones",
        json!({"type":"only","values":["local-process","operator-configured-local-model"]}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/network",
        json!({"profiles":["controlled-model-profile"],"destinations":[model.server.address().to_string()]}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/filesystem",
        json!([
            {"root":executable.parent().ok_or("fixture binary has no parent")?,"access":["execute"]},
            {"root":directory,"access":["read","write"]}
        ]),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/artifacts",
        json!({"type":"allow","identities":{"type":"any"},"sensitivities":["public","internal","restricted"]}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/resources/workspace",
        json!({"scopes":[],"allow_any_in_run":true}),
    )?;
    set(
        &mut configuration,
        "/actors/0/authority/dangerous_allow_broad_authority",
        json!(true),
    )?;
    set(
        &mut configuration,
        "/adapters",
        json!({"process_profiles":[profile],"model_profiles":[{"capability_id":"controlled-model","profile":model.profile(&directory)?}]}),
    )?;
    let validated: milkdrift_daemon::DaemonConfig = serde_json::from_value(configuration)?;
    validated.clone().validate(&directory)?;
    let config = write_private(
        &directory.join("daemon.toml"),
        toml::to_string_pretty(&validated)?.as_bytes(),
    )?;
    Ok(Configured {
        config,
        endpoint: format!("http://{endpoint}"),
        token,
    })
}

fn process_profile(directory: &Path, executable: &Path) -> EvidenceResult<PathBuf> {
    let (digest, size) = hash_file(executable)?;
    let profile = json!({
        "schema_version":2,
        "profile":{
            "profile_id":"cargo-report-profile","revision":1,"capability":"cargo-report","descriptor_revision":1,
            "provider_profile":null,"operation":"process.execute","side_effect":"read_only","idempotency":"unsupported",
            "cancellation":"best_effort","trust_class":"trusted_host_process","executable":executable,
            "implementation":{"content_digest":digest,"size_bytes":size,"package_revision":"connected-tasks-manifest-report-v1","documentation_reference":"urn:milkdrift:connected-tasks:manifest-report"},
            "arguments":["cargo-report","--file","Cargo.toml","--delay-ms","5000"],"substitutions":{},
            "working_directory":{"type":"isolated_root"},
            "filesystem_roots":[{"path":executable.parent().ok_or("fixture executable has no parent")?,"access":"execute"},{"path":directory,"access":"read_write"}],
            "inputs":[{"input":"file","relative_path":"Cargo.toml"}],
            "environment":{"allowed_non_secret":[],"secrets":{},"max_value_bytes":4096},"stdin":{"type":"disabled"},
            "stdout":{"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "stderr":{"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "outputs":[{"name":"report","relative_path":"report.txt","media_type":"text/plain","required":true}],
            "limits":{"max_argv_entries":8,"max_argv_bytes":4096,"max_children_observed":4,"max_files":8,"max_file_bytes":1048576,"max_total_materialized_bytes":2097152,
                "max_path_bytes":4096,"max_directory_depth":16,"artifact_chunk_bytes":65536,"max_output_files":4,"max_total_output_bytes":2097152,
                "wall_timeout_ms":30000,"graceful_termination_ms":100,"forced_termination_ms":100,"heartbeat_interval_ms":100},
            "restart":"retain_uncertain","platform":milkdrift_local_process::PlatformSupport::current(),"max_concurrent":2,"extensions":{}
        }
    });
    let validated =
        milkdrift_local_process::ProcessProfileDocument::from_json(&serde_json::to_vec(&profile)?)?;
    write_private(
        &directory.join("cargo-report-profile.json"),
        &validated.to_canonical_json()?,
    )
}

fn set(document: &mut Value, pointer: &str, value: Value) -> EvidenceResult {
    *document
        .pointer_mut(pointer)
        .ok_or_else(|| format!("maintained daemon template omits {pointer}"))? = value;
    Ok(())
}

fn private_directory(path: &Path) -> EvidenceResult {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_browser_config_compiles_complete_serving_defaults() -> EvidenceResult {
        let root = tempfile::tempdir()?;
        let mut model = super::super::model::Model::start(root.path())?;
        let configured = configure(
            root.path(),
            &std::env::current_exe()?.canonicalize()?,
            "fixture",
            &model,
        )?;
        let plan = milkdrift_daemon::DaemonConfig::load(&configured.config)?;
        assert!(plan.bind().ip().is_loopback());
        assert!(!configured.token.is_empty());
        model.finish()?;
        Ok(())
    }
}
