use super::prepare::{self, INTERNAL_ARTIFACT_BYTES};
use milkdrift_evidence::{EvidenceResult, application::write_private};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) fn configure(config: &mut Value, root: &Path) -> EvidenceResult {
    let runtime = config["runtime"]
        .as_object_mut()
        .ok_or("runtime config absent")?;
    for key in [
        "effect_threads",
        "effect_queue",
        "global_concurrency",
        "per_run_concurrency",
        "per_capability_concurrency",
        "per_branch_concurrency",
        "maximum_effect_claim",
    ] {
        runtime.insert(key.into(), json!(1));
    }
    runtime.insert("controller_activation".into(), json!("enabled"));
    runtime.insert(
        "publication_services".into(),
        json!({"method:slotbook":"grant:operator"}),
    );
    config["serving"]["worker_threads"] = json!(1);
    config["serving"]["observation_hot_retention_ms"] = json!(100);
    config["serving"]["clients"]["execution_limits"]["nested_invocations"] =
        json!({"process":8,"model":0});
    config["serving"]["clients"]["execution_limits"]["artifact_bytes"] =
        json!(INTERNAL_ARTIFACT_BYTES);
    let operator = &mut config["actors"][0];
    operator["authority"]["resources"]["capability"]["identities"]["values"]
        .as_array_mut()
        .ok_or("capability identities absent")?
        .push(json!("method:slotbook"));
    operator["authority"]["resources"]["capability"]["operations"]["values"]
        .as_array_mut()
        .ok_or("capability operations absent")?
        .extend(
            [
                "method.publish",
                "method.inspect",
                "method.retire",
                "method.invoke",
            ]
            .map(|v| json!(v)),
        );
    let mut consumer = operator.clone();
    consumer["actor"] = json!("human:consumer");
    consumer["credential_ref"] = json!("credential:consumer");
    consumer["grant_id"] = json!("grant:consumer");
    consumer["preset"] = json!("invoker");
    let resources = &mut consumer["authority"]["resources"];
    resources["capability"]["identities"]["values"] = json!(["method:slotbook"]);
    resources["capability"]["operations"]["values"] = json!(["method.invoke"]);
    resources["artifacts"] = json!({"type":"deny_all"});
    resources["filesystem"] = json!([]);
    resources["workspace"] = json!({"allow_any_in_run":false,"scopes":[]});
    resources["secrets"] = json!([]);
    config["actors"]
        .as_array_mut()
        .ok_or("actors absent")?
        .push(consumer);
    let credential = root.join("consumer.token");
    prepare::credential(&credential)?;
    config["secret_sources"]["credential:consumer"] = json!({"type":"file","path":credential});
    Ok(())
}
pub(super) fn method(mut descriptor: Value, revision: &Value, authority: &Value) -> Value {
    descriptor["identity"] = json!("method:slotbook");
    descriptor["descriptor_revision"] = json!(1);
    descriptor["category"] = json!({"type":"tool"});
    descriptor["operations"] =
        json!({"method.invoke":descriptor["operations"]["workspace.execute"]});
    json!({"schema_version":1,"descriptor":descriptor,"documentation":"Verify and deploy the governed Slotbook candidate to the fixed test installation. A successful invocation confirms the internal agreement; application service state remains separately inspectable.","revision":revision["id"],"agreement":revision["semantic"]["agreement"]["digest"],"service":{"actor":authority["actor"],"grant":authority["grant_id"],"grant_revision":authority["grant_revision"],"grant_digest":authority["grant_digest"],"revocation_generation":0},"inputs":{},"outputs":{},"workspace_budget":{"max_value_versions":1024,"max_inline_bytes_per_value":65536,"max_total_inline_bytes":1048576,"max_artifacts":256,"max_bytes_per_artifact":33554432,"max_total_artifact_bytes":INTERNAL_ARTIFACT_BYTES},"allowance":{"cost_micros":0,"currency":null,"input_units":0,"output_units":0,"artifact_bytes":INTERNAL_ARTIFACT_BYTES,"process_admissions":8,"model_admissions":0},"maximum_outstanding":1,"maximum_depth":4,"maximum_duration_ms":240000})
}
pub(super) fn outer(
    root: &Path,
    cli: &Path,
    capability: &str,
    peer: bool,
) -> EvidenceResult<Value> {
    let document: Value = serde_json::from_slice(&std::fs::read(root.join("governed.json"))?)?;
    let internal = &document["revision"];
    let mut task = internal["semantic"]["nodes"]["repair.begin"].clone();
    task["id"] = json!("invoke-method");
    task["control_inputs"] = json!([]);
    task["data_inputs"] = json!({});
    task["data_outputs"] = json!({});
    let req = &mut task["kind"]["config"]["requirement"];
    req["categories"] = json!([{"type":"tool"}]);
    req["exact_capability"] = json!(capability);
    req["operation"] = json!("method.invoke");
    if peer {
        req["placement"] = json!({"localities":["peer"],"peers":["host:slotbook-test"]});
    }
    let mutations = json!([{"type":"add_node","node":task},{"type":"add_node","node":internal["semantic"]["nodes"]["done"]},{"type":"add_edge","edge":prepare::edge("finish","invoke-method","done","control","out","in")}]);
    let input = prepare::write(root, "outer-mutations.json", &mutations)?;
    prepare::local(
        cli,
        &[
            "blueprint".into(),
            "create".into(),
            prepare::argument(input),
            "--workflow".into(),
            "slotbook-caller".into(),
            "--author".into(),
            "agent:repair".into(),
            "--output".into(),
            prepare::argument(root.join("outer.json")),
        ],
    )?;
    Ok(
        serde_json::from_slice::<Value>(&std::fs::read(root.join("outer.json"))?)?["revision"]
            .clone(),
    )
}
pub(super) fn peer_configuration(
    config: &mut Value,
    root: &Path,
    provider_port: u16,
) -> EvidenceResult<(PathBuf, u16)> {
    let origin_port = provider_port.checked_add(1).ok_or("peer port overflows")?;
    let credential = root.join("peer.token");
    prepare::credential(&credential)?;
    config["secret_sources"]["credential:peer"] = json!({"type":"file","path":credential});
    let relationship = |peer: &str, port: u16| json!({"peer_id":peer,"endpoint":format!("http://127.0.0.1:{port}/"),"credential_ref":"credential:peer","insecure_loopback_development":true,"minimum_minor":5,"maximum_minor":5,"actions":["read_catalog","invoke","cancel","artifact_upload","artifact_download"],"capability_allow":["method:slotbook"],"capability_deny":[],"operation_allow":["method.invoke"],"maximum_side_effect":"non_idempotent_write","maximum_concurrent":1,"maximum_requests_per_minute":6000,"maximum_artifact_bytes":INTERNAL_ARTIFACT_BYTES,"artifact_sensitivities":["internal","restricted"],"maximum_duration_ms":240000,"maximum_cost_micros":0,"maximum_input_units":0,"maximum_output_units":0,"nested_invocations":{"process":8,"model":0},"maximum_observations":256,"catalog_ttl_ms":300000,"trust_zone":"slotbook-qualification","delegation_ref":"delegation:slotbook","expires_at_unix_ms":4102444800000_u64,"enabled":true});
    config["peers"] = json!({"mode":"enabled","relationships":[relationship("host:slotbook-caller",origin_port)]});
    let mut origin = config.clone();
    origin["host_id"] = json!("host:slotbook-caller");
    origin["data_root"] = json!(root.join("origin-data"));
    origin["bind"] = json!(format!("127.0.0.1:{origin_port}"));
    origin["runtime"]["publication_services"] = json!({});
    origin["adapters"] = json!({"process_profiles":[],"model_profiles":[]});
    origin["peers"]["relationships"] = json!([relationship("host:slotbook-test", provider_port)]);
    origin["actors"] = json!([origin["actors"][0]]);
    let resources = &mut origin["actors"][0]["authority"]["resources"];
    resources["capability"]["identities"] = json!({"type":"any"});
    resources["capability"]["operations"] = json!({"type":"only","values":["method.invoke"]});
    resources["filesystem"] = json!([]);
    resources["network"] = json!({"profiles":["peer:host:slotbook-test"],"destinations":[format!("127.0.0.1:{provider_port}")]});
    resources["peers"] = json!({"allow_any":false,"identities":["host:slotbook-test"]});
    let path = root.join("origin.toml");
    write_private(&path, toml::to_string_pretty(&origin)?.as_bytes())?;
    Ok((path, origin_port))
}
