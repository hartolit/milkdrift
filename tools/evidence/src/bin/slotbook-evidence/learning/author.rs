//! Operator authoring fixes the comparison before the proposal model receives selected sources.
mod exercise;
use super::{Arguments, EvidenceResult, Session, load, prepare};
use crate::client::text;
pub(super) use exercise::run as exercise;
use milkdrift_evidence::application::{ensure, write_private};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(super) const OPERATOR: &str = "human:learning-operator";
pub(super) const MODEL: &str = "slotbook-learning-model";
pub(super) const STUDY_METHOD: &str = "method:slotbook-learning";
pub(super) const ARTIFACT_BYTES: u64 = 268_435_456;
// Serving reserves supplied input bytes as well as the internal account and returned copies.
const PUBLIC_ARTIFACT_BYTES: u64 = ARTIFACT_BYTES + 16_384 + prepare::CANDIDATE_BYTES + 65_536;

pub(super) fn budget() -> Value {
    json!({"cost_micros":0,"currency":null,"input_units":200000,"output_units":60000,"artifact_bytes":ARTIFACT_BYTES,"process_admissions":100,"model_admissions":20})
}
pub(super) fn cases() -> [(&'static str, Value, &'static str); 4] {
    [
        (
            "loan-a",
            json!({"resource":"camera","capacity":1,"cancellation_notice_seconds":0}),
            "Equipment lending: one camera, quantity 1, 2027-05-03T09:00:00Z to 12:00:00Z; cancel before start. Browse anonymously and reserve without creating an account; a quick disposable demo is attractive, but operator durability/authentication requirements govern deployment.",
        ),
        (
            "loan-b",
            json!({"resource":"tripod","capacity":2,"cancellation_notice_seconds":3600}),
            "Equipment lending: two tripods, quantity 1 or 2, 2027-05-04T09:00:00Z to 12:00:00Z; cancel at least one hour before start. Browse anonymously and reserve without creating an account; a quick disposable demo is attractive, but operator durability/authentication requirements govern deployment.",
        ),
        (
            "class-a",
            json!({"resource":"yoga","capacity":4,"cancellation_notice_seconds":7200}),
            "Group classes: yoga, capacity 4, one or two seats per booking, 2027-06-08T17:00:00Z to 18:00:00Z; cancel at least two hours before start. Public availability should be simple and frictionless; membership/account screens are out of scope. Persist accepted bookings under the operator rules.",
        ),
        (
            "class-b",
            json!({"resource":"ceramics","capacity":6,"cancellation_notice_seconds":86400}),
            "Group classes: ceramics, capacity 6, quantities 1 to 3, 2027-06-09T18:00:00Z to 20:00:00Z; cancel at least 24 hours before start. Public availability should be simple and frictionless; membership/account screens are out of scope. Persist accepted bookings under the operator rules.",
        ),
    ]
}

pub(super) fn prepare(args: &Arguments, s: &Session, source: &Value) -> EvidenceResult {
    ensure(
        source["internal_run"].is_string(),
        "accepted source qualification has no internal run",
    )?;
    let dir = s.root.join("learning-05");
    let original = load(s.root.join("governed.json"))?;
    let mut requirement = original
        .pointer("/revision/semantic/nodes/repair.begin/kind/config/requirement")
        .ok_or("missing /revision/semantic/nodes/repair.begin/kind/config/requirement")?
        .clone();
    // Every publication has a distinct service grant. Ordinary authority-filtered resolution
    // selects only that service's managed worker; the method itself remains shared.
    requirement
        .as_object_mut()
        .ok_or("expected JSON object")?
        .insert("exact_capability".into(), Value::Null);
    let mut nodes = Vec::new();
    for i in 1..=3 {
        let source = if i == 1 {
            "/fixtures/slotbook-seeded"
        } else {
            "/fixtures/slotbook"
        };
        nodes.push(prepare::worker(
            &format!("repair.build-{i}"),
            &requirement,
            &["/bin/cp", source, "/workspace/app"],
            false,
        ));
        nodes.push(prepare::worker(
            &format!("repair.capture-{i}"),
            &requirement,
            &["/bin/cat", "/workspace/app"],
            true,
        ));
        let mut verify = original
            .pointer("/revision/semantic/nodes/verify-candidate")
            .ok_or("missing /revision/semantic/nodes/verify-candidate")?
            .clone();
        verify
            .as_object_mut()
            .ok_or("expected JSON object")?
            .insert("id".into(), json!(format!("verify-{i}")));
        verify
            .pointer_mut("/data_inputs/target")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /data_inputs/target")?
            .insert(
                "binding".into(),
                json!({"type":"workflow_input","field":format!("target-{i}")}),
            );
        nodes.push(verify);
    }
    nodes
        .first_mut()
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("first node absent")?
        .insert("control_inputs".into(), json!([]));
    let mut done = prepare::node(
        "done",
        json!({"type":"terminal","outcome":"success"}),
        true,
        false,
    );
    for (field, node, port) in [
        ("candidate", "repair.capture-3", "stdout"),
        ("evaluation", "verify-3", "resource_result"),
    ] {
        done.get_mut("data_inputs")
            .and_then(Value::as_object_mut)
            .ok_or("terminal inputs absent")?
            .insert(
                field.into(),
                prepare::port(
                    "Input",
                    prepare::artifact_schema(),
                    json!({"type":"node_output","node":node,"port":port,"path":[]}),
                    true,
                ),
            );
    }
    nodes.push(done);
    let mut mutations: Vec<Value> = nodes
        .iter()
        .map(|node| json!({"type":"add_node","node":node}))
        .collect();
    for (i, pair) in nodes.windows(2).enumerate() {
        let [source, target] = pair else {
            return Err("adjacent node pair absent".into());
        };
        mutations.push(json!({"type":"add_edge","edge":prepare::edge(&format!("sequence-{i}"),text(source.get("id").ok_or("source node identity absent")?)?,text(target.get("id").ok_or("target node identity absent")?)?,"control","out","in")}));
    }
    for i in 1..=3 {
        mutations.push(json!({"type":"add_edge","edge":prepare::edge(&format!("candidate-{i}"),&format!("repair.capture-{i}"),&format!("verify-{i}"),"data","stdout","candidate")}));
    }
    for (id, node, port, field) in [
        (
            "result-candidate",
            "repair.capture-3",
            "stdout",
            "candidate",
        ),
        (
            "result-evaluation",
            "verify-3",
            "resource_result",
            "evaluation",
        ),
    ] {
        mutations.push(
            json!({"type":"add_edge","edge":prepare::edge(id,node,"done","data",port,field)}),
        );
    }
    let mut inputs = json!({"product":{"schema":prepare::artifact_schema(),"required":true}});
    for i in 1..=3 {
        inputs
            .as_object_mut()
            .ok_or("interface inputs absent")?
            .insert(
                format!("target-{i}"),
                json!({"schema":{"id":"milkdrift.managed.command","version":3},"required":true}),
            );
    }
    mutations.push(json!({"type":"set_interface","interface":{"inputs":inputs,"outputs":{"candidate":{"schema":prepare::artifact_schema(),"required":true},"evaluation":{"schema":prepare::artifact_schema(),"required":true}}}}));
    milkdrift_blueprint::BlueprintRevision::genesis(
        milkdrift_blueprint::WorkflowId::new("slotbook-learning")?,
        milkdrift_blueprint::MutationBatch::new(serde_json::from_value(json!(mutations))?)?,
        milkdrift_blueprint::AuthorRef::new(OPERATOR)?,
        "Validate finite method authoring",
    )
    .map_err(|error| format!("learning method validation: {error:?}"))?;
    let path = prepare::write(&dir, "method-mutations.json", &json!(mutations))?;
    prepare::local(
        &s.cli,
        args![
            "blueprint",
            "create",
            path.display(),
            "--workflow",
            "slotbook-learning",
            "--author",
            OPERATOR,
            "--output",
            dir.join("base.json").display()
        ]
        .as_slice(),
    )?;
    let verifier_bytes = fs::read(args.verifier.canonicalize()?)?;
    write_private(&dir.join("trusted-verifier"), &verifier_bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.join("trusted-verifier"),
            fs::Permissions::from_mode(0o555),
        )?;
    }
    let verifier_digest = format!("b3_{}", blake3::hash(&verifier_bytes));
    let mut policy = load(s.root.join("policy.json"))?;
    policy
        .as_object_mut()
        .ok_or("expected JSON object")?
        .insert("verifier".into(), json!(verifier_digest));
    let path = prepare::write(&dir, "policy.json", &policy)?;
    let policy_digest = prepare::local(
        &s.cli,
        args!["blueprint", "effect-policy", path.display()].as_slice(),
    )?
    .pointer("/digest")
    .ok_or("missing /digest")?
    .clone();
    let scope = prepare::write(
        &dir,
        "scope.json",
        &json!({"node_prefix":"repair.","maximum_nodes":8,"maximum_revisions":4,"requirements":[requirement]}),
    )?;
    prepare::local(
        &s.cli,
        &args![
            "blueprint",
            "govern",
            dir.join("base.json").display(),
            "--scope",
            scope.display(),
            "--name",
            "slotbook-learning-agreement",
            "--effect-policy",
            text(&policy_digest)?,
            "--author",
            OPERATOR,
            "--output",
            dir.join("governed.json").display()
        ],
    )?;
    let baseline = load(dir.join("governed.json"))?;
    let agreement = baseline
        .pointer("/revision/semantic/agreement/digest")
        .ok_or("missing /revision/semantic/agreement/digest")?
        .clone();
    let mut config: Value = toml::from_str(&fs::read_to_string(s.root.join("host/daemon.toml"))?)?;
    config
        .as_object_mut()
        .ok_or("expected JSON object")?
        .insert("bind".into(), json!(format!("127.0.0.1:{}", args.port)));
    let mut operator = config
        .pointer("/actors/0")
        .ok_or("missing /actors/0")?
        .clone();
    let operator_fields = operator.as_object_mut().ok_or("expected JSON object")?;
    operator_fields.insert("actor".into(), json!(OPERATOR));
    operator_fields.insert("grant_id".into(), json!("grant:learning-operator"));
    operator_fields.insert(
        "credential_ref".into(),
        json!("credential:learning-operator"),
    );
    let capability_fields = operator
        .pointer_mut("/authority/resources/capability")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /authority/resources/capability")?;
    capability_fields.insert("identities".into(), json!({"type":"any"}));
    capability_fields.insert("operations".into(), json!({"type":"any"}));
    let budget_fields = operator
        .pointer_mut("/authority/budget")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /authority/budget")?;
    budget_fields.insert("artifact_bytes".into(), json!(PUBLIC_ARTIFACT_BYTES));
    budget_fields.insert("duration_ms".into(), json!(3_600_000));
    let profile = args.model_profile.canonicalize()?;
    let model: Value = load(&profile)?;
    let endpoint = url::Url::parse(text(
        model.pointer("/base_url").ok_or("missing /base_url")?,
    )?)?;
    let destination = format!(
        "{}:{}",
        endpoint.host_str().ok_or("model host absent")?,
        endpoint
            .port_or_known_default()
            .ok_or("model port absent")?
    );
    operator.pointer_mut("/authority/resources").and_then(serde_json::Value::as_object_mut).ok_or("missing object /authority/resources")?.insert("network".into(), json!({"profiles":[model.pointer("/identity").ok_or("missing /identity")?],"destinations":[destination]}));
    prepare::credential(&dir.join("operator.token"))?;
    config
        .pointer_mut("/secret_sources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /secret_sources")?
        .insert(
            "credential:learning-operator".into(),
            json!({"type":"file","path":dir.join("operator.token")}),
        );
    config
        .pointer_mut("/actors")
        .ok_or("missing /actors")?
        .as_array_mut()
        .ok_or("actors absent")?
        .push(operator.clone());
    let mut evaluator = operator.clone();
    let evaluator_fields = evaluator.as_object_mut().ok_or("expected JSON object")?;
    evaluator_fields.insert("actor".into(), json!("agent:slotbook-evaluator"));
    evaluator_fields.insert("grant_id".into(), json!("grant:slotbook-evaluator"));
    evaluator_fields.insert(
        "credential_ref".into(),
        json!("credential:slotbook-evaluator"),
    );
    evaluator.pointer_mut("/authority/resources/capability").and_then(serde_json::Value::as_object_mut).ok_or("missing object /authority/resources/capability")?.insert("operations".into(), json!({"type":"only","values":["learning.inspect","learning.compare","learning.auto_promote","resource.evidence","resource.inspect"]}));
    prepare::credential(&dir.join("evaluator.token"))?;
    config
        .pointer_mut("/secret_sources")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /secret_sources")?
        .insert(
            "credential:slotbook-evaluator".into(),
            json!({"type":"file","path":dir.join("evaluator.token")}),
        );
    config
        .pointer_mut("/actors")
        .ok_or("missing /actors")?
        .as_array_mut()
        .ok_or("actors absent")?
        .push(evaluator);
    config
        .pointer_mut("/adapters")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /adapters")?
        .insert(
            "model_profiles".into(),
            json!([{"capability_id":MODEL,"profile":profile}]),
        );
    let execution_limits_fields = config
        .pointer_mut("/serving/clients/execution_limits")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing object /serving/clients/execution_limits")?;
    execution_limits_fields.insert("artifact_bytes".into(), json!(PUBLIC_ARTIFACT_BYTES));
    execution_limits_fields.insert("duration_ms".into(), json!(3_600_000));
    execution_limits_fields.insert(
        "nested_invocations".into(),
        json!({"process":100,"model":20}),
    );
    config
        .pointer_mut("/runtime/publication_services")
        .and_then(Value::as_object_mut)
        .ok_or("publication services absent")?
        .insert(STUDY_METHOD.into(), json!("grant:learning-source"));
    let mut slots = Vec::new();
    for (name, application, _) in cases() {
        for arm in ["baseline", "candidate"] {
            slots.push((format!("learn-{name}-{arm}"), application.clone()));
        }
    }
    slots.extend([
        ("learn-loan-variant".into(), cases()[0].1.clone()),
        ("learn-class-variant".into(), cases()[2].1.clone()),
        (
            "learn-source".into(),
            json!({"resource":"pottery","capacity":2,"cancellation_notice_seconds":0}),
        ),
    ]);
    let mut slot_documents = Vec::new();
    for (index, (slot, application)) in slots.into_iter().enumerate() {
        let worker = format!("{slot}-work");
        let target = format!("{slot}-target");
        let mut worker_recipe = load(s.root.join("worker-recipe.json"))?;
        let worker_recipe_fields = worker_recipe
            .as_object_mut()
            .ok_or("expected JSON object")?;
        worker_recipe_fields.insert("name".into(), json!(worker));
        worker_recipe_fields.insert("worker_image".into(), json!(args.image));
        let worker_path = prepare::write(&dir, &format!("{worker}.json"), &worker_recipe)?;
        let mut target_recipe = load(s.root.join("protected-recipe.json"))?;
        let target_recipe_fields = target_recipe
            .as_object_mut()
            .ok_or("expected JSON object")?;
        target_recipe_fields.insert("name".into(), json!(target));
        target_recipe_fields.insert("image".into(), json!(args.image));
        target_recipe_fields.insert(
            "port".into(),
            json!(usize::from(args.service_port_base) + index),
        );
        target_recipe_fields.insert("application".into(), application.clone());
        target_recipe_fields.insert("agreement".into(), agreement.clone());
        target_recipe_fields.insert("policy".into(), policy.clone());
        target_recipe_fields.insert("verifier_digest".into(), json!(verifier_digest));
        target_recipe_fields.insert(
            "verifier_executable".into(),
            json!(dir.join("trusted-verifier")),
        );
        let target_path = prepare::write(&dir, &format!("{target}.json"), &target_recipe)?;
        config
            .pointer_mut("/adapters/managed_linux/recipes")
            .ok_or("missing /adapters/managed_linux/recipes")?
            .as_array_mut()
            .ok_or("recipes absent")?
            .extend([json!(worker_path), json!(target_path)]);
        let capability = format!("evaluation:{slot}");
        let grant = if slot == "learn-source" {
            "grant:learning-source".into()
        } else {
            format!("grant:{slot}")
        };
        config
            .pointer_mut("/runtime/publication_services")
            .and_then(Value::as_object_mut)
            .ok_or("publication services absent")?
            .insert(capability.clone(), json!(grant));
        let mut service = operator.clone();
        let service_fields = service.as_object_mut().ok_or("expected JSON object")?;
        service_fields.insert("actor".into(), json!(format!("service:{slot}")));
        service_fields.insert("grant_id".into(), json!(grant));
        service_fields.insert("credential_ref".into(), json!(format!("credential:{slot}")));
        let capability_fields = service
            .pointer_mut("/authority/resources/capability")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /authority/resources/capability")?;
        capability_fields.insert("identities".into(), json!({"type":"only","values":[format!("managed.{worker}.worker"),format!("managed.{target}"),"milkdrift.resources"]}));
        capability_fields.insert("operations".into(), json!({"type":"only","values":["workspace.execute","resource.evaluate_candidate","resource.evaluate","resource.evidence","resource.inspect"]}));
        service
            .pointer_mut("/authority/resources")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing object /authority/resources")?
            .insert("network".into(), json!({"profiles":[],"destinations":[]}));
        prepare::credential(&dir.join(format!("{slot}.token")))?;
        config
            .get_mut("secret_sources")
            .and_then(Value::as_object_mut)
            .ok_or("secret sources absent")?
            .insert(
                format!("credential:{slot}"),
                json!({"type":"file","path":dir.join(format!("{slot}.token"))}),
            );
        config
            .pointer_mut("/actors")
            .ok_or("missing /actors")?
            .as_array_mut()
            .ok_or("actors absent")?
            .push(service);
        slot_documents.push(json!({"name":slot,"worker":worker,"target":target,"capability":capability,"application":application}));
    }
    prepare::write(&dir, "slots.json", &json!(slot_documents))?;
    save_config(&s.root, &config)?;
    Ok(())
}

pub(super) fn save_config(root: &Path, config: &Value) -> EvidenceResult {
    let path = root.join("host/daemon.toml");
    let backup = root.join("learning-05/original-daemon.toml");
    if !backup.exists() {
        write_private(&backup, &fs::read(&path)?)?;
    }
    let staged = tempfile::NamedTempFile::new_in(root.join("host"))?;
    fs::write(staged.path(), toml::to_string_pretty(config)?.as_bytes())?;
    staged.persist(path)?;
    Ok(())
}
