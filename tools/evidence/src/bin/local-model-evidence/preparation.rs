//! A local refusal observed through the actual daemon, CLI, and counting endpoint.
use super::{
    ACTOR, ActorGrantConfig, Arguments, BlueprintRevisionDocument, CliRunner, Duration,
    EndpointProfile, EvidenceConfig, EvidenceResult, ModelProfileConfig, NetworkProfileRef,
    NetworkScope, Path, TOKEN, Value, ensure, fs, inspect_profile, json, model_revision, node,
    path_text, profile_destination, required_text, reserve_endpoint, start_daemon,
    wait_for_readiness, wait_for_run, write_config, write_model_profile, write_private,
};

pub(super) fn scenario(arguments: &Arguments, output: &Path) -> EvidenceResult<Value> {
    let directory = output.join("preparation-session");
    fs::create_dir(&directory)?;
    let endpoint = std::net::TcpListener::bind("127.0.0.1:0")?;
    endpoint.set_nonblocking(true)?;
    let profile = write_model_profile(
        &directory,
        "local-refusal",
        endpoint.local_addr()?,
        "controlled-preparation",
    )?;
    let mut value: Value = serde_json::from_slice(&fs::read(&profile)?)?;
    value["limits"]["max_request_bytes"] = json!(1);
    let bytes = serde_json::to_vec(&value)?;
    EndpointProfile::from_json(&bytes)?;
    fs::write(&profile, bytes)?;
    let facts = inspect_profile(&profile)?;
    let mut authority = ActorGrantConfig::dangerous_administrator();
    authority.resources.network = NetworkScope::new(
        [NetworkProfileRef::new(&facts.profile_id)?].into(),
        [profile_destination(&facts.endpoint_origin)?].into(),
    )?;
    let token_file = write_private(&directory.join("controller.token"), TOKEN.as_bytes())?;
    let bind = reserve_endpoint()?;
    let config = write_config(
        &directory,
        bind,
        &token_file,
        ACTOR,
        EvidenceConfig {
            process_profiles: Vec::new(),
            model_profiles: vec![ModelProfileConfig {
                capability_id: "local-refusal".to_owned(),
                profile,
            }],
            secret_sources: std::collections::BTreeMap::new(),
            lease_duration_ms: 5000,
            authority,
        },
    )?;
    let runner = CliRunner {
        executable: arguments.cli.clone(),
        endpoint: format!("http://{bind}/"),
        token_file,
        forbidden_storage_path: directory.join("data"),
    };
    let revision = model_revision("local-refusal", "local-refusal", &facts, false, 64)?;
    let blueprint = directory.join("blueprint.json");
    fs::write(
        &blueprint,
        BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
    )?;
    let mut daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    runner.success(&[
        "--command-id",
        "refusal-import",
        "blueprint",
        "import",
        path_text(&blueprint)?,
    ])?;
    runner.success(&[
        "--command-id",
        "refusal-start",
        "run",
        "start",
        "run-local-refusal",
        "local-refusal",
        revision.id().as_str(),
    ])?;
    let state = wait_for_run(
        &runner,
        "run-local-refusal",
        Duration::from_secs(15),
        |run| run["value"]["lifecycle"] == "terminal" && node(run, "model").is_some(),
    )?;
    let model = node(&state, "model").ok_or("refused model absent")?;
    let attempt = required_text(model, &["latest_attempt_id"])?;
    let before = runner.success(&["attempt", "inspect", "run-local-refusal", &attempt])?;
    ensure(
        model["attempt_count"] == 1
            && state["value"]["uncertainty_count"] == 0
            && before["value"]["terminal"] == "rejected"
            && before["value"]["uncertain"] == false
            && before["value"]["entry_authorization"].is_null()
            && matches!(endpoint.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "local preparation refusal was retried, submitted, or classified as uncertain",
    )?;
    fs::write(
        directory.join("attempt-before.json"),
        serde_json::to_vec_pretty(&before)?,
    )?;
    daemon.terminate()?;
    daemon = start_daemon(&arguments.daemon, &config)?;
    wait_for_readiness(&runner, &mut daemon)?;
    let after = runner.success(&["attempt", "inspect", "run-local-refusal", &attempt])?;
    ensure(
        before["value"] == after["value"]
            && matches!(endpoint.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "restart changed the refused attempt or submitted its request",
    )?;
    daemon.terminate()?;

    Ok(
        json!({"attempt_id": attempt, "terminal": "rejected", "provider_requests": 0,
        "entry_intent": false, "uncertainty_count": 0, "restart_preserved": true}),
    )
}
