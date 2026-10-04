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
    let serving_fields = config
        .pointer_mut("/serving")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /serving")?;
    serving_fields.insert("worker_threads".into(), json!(1));
    serving_fields.insert("observation_hot_retention_ms".into(), json!(100));
    let execution_limits_fields = config
        .pointer_mut("/serving/clients/execution_limits")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /serving/clients/execution_limits")?;
    execution_limits_fields.insert("nested_invocations".into(), json!({"process":8,"model":0}));
    execution_limits_fields.insert("artifact_bytes".into(), json!(INTERNAL_ARTIFACT_BYTES));
    let operator = config
        .pointer_mut("/actors/0")
        .ok_or("operator actor absent")?;
    operator
        .pointer_mut("/authority/resources/capability/identities/values")
        .ok_or("missing /authority/resources/capability/identities/values")?
        .as_array_mut()
        .ok_or("capability identities absent")?
        .push(json!("method:slotbook"));
    operator
        .pointer_mut("/authority/resources/capability/operations/values")
        .ok_or("missing /authority/resources/capability/operations/values")?
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
    let consumer_fields = consumer.as_object_mut().ok_or("expected JSON object")?;
    consumer_fields.insert("actor".into(), json!("human:consumer"));
    consumer_fields.insert("credential_ref".into(), json!("credential:consumer"));
    consumer_fields.insert("grant_id".into(), json!("grant:consumer"));
    consumer_fields.insert("preset".into(), json!("invoker"));
    let resources = consumer
        .pointer_mut("/authority/resources")
        .ok_or("consumer resource grant absent")?;
    resources
        .pointer_mut("/capability/identities")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /capability/identities")?
        .insert("values".into(), json!(["method:slotbook"]));
    resources
        .pointer_mut("/capability/operations")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /capability/operations")?
        .insert("values".into(), json!(["method.invoke"]));
    let resource_fields = resources
        .as_object_mut()
        .ok_or("consumer resource grant is not an object")?;
    resource_fields.insert("artifacts".into(), json!({"type":"deny_all"}));
    resource_fields.insert("filesystem".into(), json!([]));
    resource_fields.insert(
        "workspace".into(),
        json!({"allow_any_in_run":false,"scopes":[]}),
    );
    resource_fields.insert("secrets".into(), json!([]));
    config
        .get_mut("actors")
        .ok_or("actors absent")?
        .as_array_mut()
        .ok_or("actors absent")?
        .push(consumer);
    let credential = root.join("consumer.token");
    prepare::credential(&credential)?;
    config
        .pointer_mut("/secret_sources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /secret_sources")?
        .insert(
            "credential:consumer".into(),
            json!({"type":"file","path":credential}),
        );
    Ok(())
}
pub(super) fn method(
    mut descriptor: Value,
    revision: &Value,
    authority: &Value,
) -> EvidenceResult<Value> {
    let operation = descriptor
        .pointer("/operations/workspace.execute")
        .ok_or("workspace operation absent")?
        .clone();
    let descriptor_fields = descriptor.as_object_mut().ok_or("expected JSON object")?;
    descriptor_fields.insert("identity".into(), json!("method:slotbook"));
    descriptor_fields.insert("descriptor_revision".into(), json!(1));
    descriptor_fields.insert("category".into(), json!({"type":"tool"}));
    descriptor_fields.insert("operations".into(), json!({"method.invoke":operation}));
    Ok(
        json!({"schema_version":1,"descriptor":descriptor,"documentation":"Verify and deploy the governed Slotbook candidate to the fixed test installation. A successful invocation confirms the internal agreement; application service state remains separately inspectable.","revision":revision["id"],"agreement":revision.pointer("/semantic/agreement/digest").ok_or("missing /semantic/agreement/digest")?,"service":{"actor":authority["actor"],"grant":authority["grant_id"],"grant_revision":authority["grant_revision"],"grant_digest":authority["grant_digest"],"revocation_generation":0},"inputs":{},"outputs":{},"workspace_budget":{"max_value_versions":1024,"max_inline_bytes_per_value":65536,"max_total_inline_bytes":1048576,"max_artifacts":256,"max_bytes_per_artifact":33554432,"max_total_artifact_bytes":INTERNAL_ARTIFACT_BYTES},"allowance":{"cost_micros":0,"currency":null,"input_units":0,"output_units":0,"artifact_bytes":INTERNAL_ARTIFACT_BYTES,"process_admissions":8,"model_admissions":0},"maximum_outstanding":1,"maximum_depth":4,"maximum_duration_ms":240000}),
    )
}
pub(super) fn outer(
    root: &Path,
    cli: &Path,
    capability: &str,
    peer: bool,
) -> EvidenceResult<Value> {
    let document: Value = serde_json::from_slice(&std::fs::read(root.join("governed.json"))?)?;
    let internal = document.pointer("/revision").ok_or("missing /revision")?;
    let mut task = internal
        .pointer("/semantic/nodes/repair.begin")
        .ok_or("missing /semantic/nodes/repair.begin")?
        .clone();
    let task_fields = task.as_object_mut().ok_or("expected JSON object")?;
    task_fields.insert("id".into(), json!("invoke-method"));
    task_fields.insert("control_inputs".into(), json!([]));
    task_fields.insert("data_inputs".into(), json!({}));
    task_fields.insert("data_outputs".into(), json!({}));
    let req = task
        .pointer_mut("/kind/config/requirement")
        .and_then(Value::as_object_mut)
        .ok_or("method requirement absent")?;
    req.insert("categories".into(), json!([{"type":"tool"}]));
    req.insert("exact_capability".into(), json!(capability));
    req.insert("operation".into(), json!("method.invoke"));
    if peer {
        req.insert(
            "placement".into(),
            json!({"localities":["peer"],"peers":["host:slotbook-test"]}),
        );
    }
    let mutations = json!([{"type":"add_node","node":task},{"type":"add_node","node":internal.pointer("/semantic/nodes/done").ok_or("missing /semantic/nodes/done")?},{"type":"add_edge","edge":prepare::edge("finish","invoke-method","done","control","out","in")}]);
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
        serde_json::from_slice::<Value>(&std::fs::read(root.join("outer.json"))?)?
            .pointer("/revision")
            .ok_or("missing /revision")?
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
    config
        .pointer_mut("/secret_sources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /secret_sources")?
        .insert(
            "credential:peer".into(),
            json!({"type":"file","path":credential}),
        );
    let relationship = |peer: &str, port: u16| json!({"peer_id":peer,"endpoint":format!("http://127.0.0.1:{port}/"),"credential_ref":"credential:peer","insecure_loopback_development":true,"minimum_minor":5,"maximum_minor":5,"actions":["read_catalog","invoke","cancel","artifact_upload","artifact_download"],"capability_allow":["method:slotbook"],"capability_deny":[],"operation_allow":["method.invoke"],"maximum_side_effect":"non_idempotent_write","maximum_concurrent":1,"maximum_requests_per_minute":6000,"maximum_artifact_bytes":INTERNAL_ARTIFACT_BYTES,"artifact_sensitivities":["internal","restricted"],"maximum_duration_ms":240000,"maximum_cost_micros":0,"maximum_input_units":0,"maximum_output_units":0,"nested_invocations":{"process":8,"model":0},"maximum_observations":256,"catalog_ttl_ms":300000,"trust_zone":"slotbook-qualification","delegation_ref":"delegation:slotbook","expires_at_unix_ms":4102444800000_u64,"enabled":true});
    config.as_object_mut().ok_or("daemon configuration is not an object")?.insert("peers".into(), json!({"mode":"enabled","relationships":[relationship("host:slotbook-caller",origin_port)]}));
    let mut origin = config.clone();
    let origin_fields = origin.as_object_mut().ok_or("expected JSON object")?;
    origin_fields.insert("host_id".into(), json!("host:slotbook-caller"));
    origin_fields.insert("data_root".into(), json!(root.join("origin-data")));
    origin_fields.insert("bind".into(), json!(format!("127.0.0.1:{origin_port}")));
    origin
        .pointer_mut("/runtime")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /runtime")?
        .insert("publication_services".into(), json!({}));
    origin
        .as_object_mut()
        .ok_or("expected JSON object")?
        .insert(
            "adapters".into(),
            json!({"process_profiles":[],"model_profiles":[]}),
        );
    origin
        .pointer_mut("/peers")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /peers")?
        .insert(
            "relationships".into(),
            json!([relationship("host:slotbook-test", provider_port)]),
        );
    let operator = origin
        .pointer("/actors/0")
        .ok_or("origin actor absent")?
        .clone();
    origin
        .as_object_mut()
        .ok_or("expected JSON object")?
        .insert("actors".into(), json!([operator]));
    let resources = origin
        .pointer_mut("/actors/0/authority/resources")
        .ok_or("origin resource grant absent")?;
    let capability_fields = resources
        .pointer_mut("/capability")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /capability")?;
    capability_fields.insert("identities".into(), json!({"type":"any"}));
    capability_fields.insert(
        "operations".into(),
        json!({"type":"only","values":["method.invoke"]}),
    );
    let resource_fields = resources
        .as_object_mut()
        .ok_or("origin resource grant is not an object")?;
    resource_fields.insert("filesystem".into(), json!([]));
    resource_fields.insert("network".into(), json!({"profiles":["peer:host:slotbook-test"],"destinations":[format!("127.0.0.1:{provider_port}")]}));
    resource_fields.insert(
        "peers".into(),
        json!({"allow_any":false,"identities":["host:slotbook-test"]}),
    );
    let path = root.join("origin.toml");
    write_private(&path, toml::to_string_pretty(&origin)?.as_bytes())?;
    Ok((path, origin_port))
}
