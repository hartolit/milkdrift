use super::{
    BTreeSet, EndpointProfile, EvidenceResult, Mode, ModelFacts, Path, PathBuf, PlatformSupport,
    ProcessProfileDocument, SocketAddr, Value, ensure, fs, hash_file, json, required_text,
};

pub(super) fn parse_secret_source(value: &str) -> Result<(String, PathBuf), String> {
    let (reference, path) = value
        .split_once('=')
        .ok_or_else(|| "secret source must be REFERENCE=FILE".to_owned())?;
    if reference.is_empty() || path.is_empty() {
        return Err("secret source must contain a nonempty reference and file".to_owned());
    }
    Ok((reference.to_owned(), PathBuf::from(path)))
}

pub(super) fn write_text_evidence_profile(
    directory: &Path,
    executable: &Path,
    profile_id: &str,
    capability: &str,
    fixture_argument: &str,
) -> EvidenceResult<PathBuf> {
    let (content_digest, executable_size) = hash_file(executable)?;
    let executable_root = executable
        .parent()
        .ok_or("evidence executable has no parent")?;
    let value = json!({
        "schema_version": 2,
        "profile": {
            "profile_id": profile_id,
            "revision": 1,
            "capability": capability,
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": "read_only",
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": content_digest,
                "size_bytes": executable_size,
                "package_revision": "local-model-evidence-v1",
                "documentation_reference": "urn:milkdrift:local-model-evidence"
            },
            "arguments": [fixture_argument],
            "substitutions": {},
            "working_directory": {"type":"isolated_root"},
            "filesystem_roots": [
                {"path": executable_root, "access":"execute"},
                {"path": directory, "access":"read_write"}
            ],
            "inputs": [],
            "environment": {"allowed_non_secret":[],"secrets":{},"max_value_bytes":4096},
            "stdin": {"type":"disabled"},
            "stdout": {"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "stderr": {"max_capture_bytes":4096,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":null},
            "outputs": [{"name":"evidence","relative_path":"evidence.txt","media_type":"text/plain","required":true}],
            "limits": {
                "max_argv_entries":8,"max_argv_bytes":4096,"max_children_observed":4,
                "max_files":8,"max_file_bytes":1048576,"max_total_materialized_bytes":2097152,
                "max_path_bytes":4096,"max_directory_depth":16,"artifact_chunk_bytes":65536,
                "max_output_files":4,"max_total_output_bytes":2097152,"wall_timeout_ms":10000,
                "graceful_termination_ms":100,"forced_termination_ms":100,"heartbeat_interval_ms":100
            },
            "restart":"retain_uncertain",
            "platform": PlatformSupport::current(),
            "max_concurrent":1,
            "extensions":{"org.milkdrift/local-model-fixture":{"deterministic":true}}
        }
    });
    let document = ProcessProfileDocument::from_json(&serde_json::to_vec(&value)?)?;
    let path = directory.join(format!("{profile_id}.json"));
    fs::write(&path, document.to_canonical_json()?)?;
    Ok(path)
}

pub(super) fn write_model_profile(
    directory: &Path,
    identity: &str,
    endpoint: SocketAddr,
    model: &str,
) -> EvidenceResult<PathBuf> {
    let value = json!({
        "schema_version": 1,
        "identity": identity,
        "revision": 1,
        "protocol": {"type":"open_ai_compatible","path":"v1/chat/completions"},
        "base_url": format!("http://{endpoint}"),
        "model": model,
        "auth": {"type":"no_auth"},
        "limits": {
            "connect_timeout_ms": 2000,
            "request_timeout_ms": 10000,
            "idle_timeout_ms": 5000,
            "max_headers": 64,
            "max_header_bytes": 16384,
            "max_request_bytes": 1048576,
            "max_response_bytes": 1048576,
            "max_stream_line_bytes": 65536,
            "max_stream_event_bytes": 131072,
            "max_fragment_bytes": 4096
        },
        "redirect": "deny",
        "tls": "web_pki_roots",
        "proxy": "disabled",
        "features": ["streaming", "system_role"],
        "max_concurrent": 1,
        "local_development": true,
        "allowed_hosts": ["127.0.0.1"],
        "trust_zones": ["local-model-evidence"],
        "provider_options": {}
    });
    let bytes = serde_json::to_vec(&value)?;
    EndpointProfile::from_json(&bytes)?;
    let path = directory.join(format!("{identity}.model-profile.json"));
    fs::write(&path, bytes)?;
    Ok(path)
}

pub(super) fn inspect_profile(path: &Path) -> EvidenceResult<ModelFacts> {
    let bytes = fs::read(path)?;
    let profile = EndpointProfile::from_json(&bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let base_url = required_text(&value, &["base_url"])?;
    let url = url::Url::parse(&base_url)?;
    let host = match url.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Domain(name)) => name.to_owned(),
        None => return Err("model profile endpoint has no host".into()),
    };
    let port = url
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    let features = value["features"]
        .as_array()
        .ok_or("model profile features are absent")?;
    let secret_refs = value["auth"]
        .get("secret")
        .and_then(Value::as_str)
        .map(|reference| BTreeSet::from([reference.to_owned()]))
        .unwrap_or_default();
    Ok(ModelFacts {
        profile_id: profile.identity().as_str().to_owned(),
        revision: value["revision"]
            .as_u64()
            .ok_or("model profile revision is absent")?,
        protocol: required_text(&value, &["protocol", "type"])?,
        model_alias: required_text(&value, &["model"])?,
        endpoint_origin: format!("{}://{host}{port}", url.scheme()),
        streaming: features
            .iter()
            .any(|feature| feature.as_str() == Some("streaming")),
        secret_refs,
    })
}

pub(super) fn validate_real_profile(mode: Mode, path: &Path, facts: &ModelFacts) -> EvidenceResult {
    ensure(
        facts.protocol == "open_ai_compatible",
        "local-model lane reuses only the OpenAI-compatible chat-completions mapping",
    )?;
    if mode == Mode::Deterministic {
        return Ok(());
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let url = url::Url::parse(&required_text(&value, &["base_url"])?)?;
    let loopback = url.host().is_some_and(|host| match host {
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
    });
    ensure(
        url.scheme() == "http"
            && loopback
            && value["local_development"] == true
            && value["redirect"] == "deny"
            && value["proxy"] == "disabled",
        "real local-model profile must use explicit loopback HTTP development policy with redirects and ambient proxies disabled",
    )
}

pub(super) fn profile_destination(origin: &str) -> EvidenceResult<String> {
    let url = url::Url::parse(origin)?;
    let host = match url.host() {
        Some(url::Host::Ipv6(address)) => format!("[{address}]"),
        Some(url::Host::Ipv4(address)) => address.to_string(),
        Some(url::Host::Domain(name)) => name.to_owned(),
        None => return Err("model endpoint origin has no host".into()),
    };
    let port = url
        .port_or_known_default()
        .ok_or("model endpoint origin has no port")?;
    Ok(format!("{host}:{port}"))
}
