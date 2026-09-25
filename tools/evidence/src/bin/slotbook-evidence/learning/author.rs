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
    let mut requirement =
        original["revision"]["semantic"]["nodes"]["repair.begin"]["kind"]["config"]["requirement"]
            .clone();
    // Every publication has a distinct service grant. Ordinary authority-filtered resolution
    // selects only that service's managed worker; the method itself remains shared.
    requirement["exact_capability"] = Value::Null;
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
        let mut verify = original["revision"]["semantic"]["nodes"]["verify-candidate"].clone();
        verify["id"] = json!(format!("verify-{i}"));
        verify["data_inputs"]["target"]["binding"] =
            json!({"type":"workflow_input","field":format!("target-{i}")});
        nodes.push(verify);
    }
    nodes[0]["control_inputs"] = json!([]);
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
        done["data_inputs"][field] = prepare::port(
            "Input",
            prepare::artifact_schema(),
            json!({"type":"node_output","node":node,"port":port,"path":[]}),
            true,
        );
    }
    nodes.push(done);
    let mut mutations: Vec<Value> = nodes
        .iter()
        .map(|node| json!({"type":"add_node","node":node}))
        .collect();
    for (i, pair) in nodes.windows(2).enumerate() {
        mutations.push(json!({"type":"add_edge","edge":prepare::edge(&format!("sequence-{i}"),text(&pair[0]["id"])?,text(&pair[1]["id"])?,"control","out","in")}));
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
        inputs[format!("target-{i}")] =
            json!({"schema":{"id":"milkdrift.managed.command","version":3},"required":true});
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
    policy["verifier"] = json!(verifier_digest);
    let path = prepare::write(&dir, "policy.json", &policy)?;
    let policy_digest = prepare::local(
        &s.cli,
        args!["blueprint", "effect-policy", path.display()].as_slice(),
    )?["digest"]
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
    let agreement = baseline["revision"]["semantic"]["agreement"]["digest"].clone();
    let mut config: Value = toml::from_str(&fs::read_to_string(s.root.join("host/daemon.toml"))?)?;
    config["bind"] = json!(format!("127.0.0.1:{}", args.port));
    let mut operator = config["actors"][0].clone();
    operator["actor"] = json!(OPERATOR);
    operator["grant_id"] = json!("grant:learning-operator");
    operator["credential_ref"] = json!("credential:learning-operator");
    operator["authority"]["resources"]["capability"]["identities"] = json!({"type":"any"});
    operator["authority"]["resources"]["capability"]["operations"] = json!({"type":"any"});
    operator["authority"]["budget"]["artifact_bytes"] = json!(PUBLIC_ARTIFACT_BYTES);
    operator["authority"]["budget"]["duration_ms"] = json!(3_600_000);
    let profile = args.model_profile.canonicalize()?;
    let model: Value = load(&profile)?;
    let endpoint = url::Url::parse(text(&model["base_url"])?)?;
    let destination = format!(
        "{}:{}",
        endpoint.host_str().ok_or("model host absent")?,
        endpoint
            .port_or_known_default()
            .ok_or("model port absent")?
    );
    operator["authority"]["resources"]["network"] =
        json!({"profiles":[model["identity"]],"destinations":[destination]});
    prepare::credential(&dir.join("operator.token"))?;
    config["secret_sources"]["credential:learning-operator"] =
        json!({"type":"file","path":dir.join("operator.token")});
    config["actors"]
        .as_array_mut()
        .ok_or("actors absent")?
        .push(operator.clone());
    let mut evaluator = operator.clone();
    evaluator["actor"] = json!("agent:slotbook-evaluator");
    evaluator["grant_id"] = json!("grant:slotbook-evaluator");
    evaluator["credential_ref"] = json!("credential:slotbook-evaluator");
    evaluator["authority"]["resources"]["capability"]["operations"] = json!({"type":"only","values":["learning.inspect","learning.compare","learning.auto_promote","resource.evidence","resource.inspect"]});
    prepare::credential(&dir.join("evaluator.token"))?;
    config["secret_sources"]["credential:slotbook-evaluator"] =
        json!({"type":"file","path":dir.join("evaluator.token")});
    config["actors"]
        .as_array_mut()
        .ok_or("actors absent")?
        .push(evaluator);
    config["adapters"]["model_profiles"] = json!([{"capability_id":MODEL,"profile":profile}]);
    config["serving"]["clients"]["execution_limits"]["artifact_bytes"] =
        json!(PUBLIC_ARTIFACT_BYTES);
    config["serving"]["clients"]["execution_limits"]["duration_ms"] = json!(3_600_000);
    config["serving"]["clients"]["execution_limits"]["nested_invocations"] =
        json!({"process":100,"model":20});
    config["runtime"]["publication_services"][STUDY_METHOD] = json!("grant:learning-source");
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
        worker_recipe["name"] = json!(worker);
        worker_recipe["worker_image"] = json!(args.image);
        let worker_path = prepare::write(&dir, &format!("{worker}.json"), &worker_recipe)?;
        let mut target_recipe = load(s.root.join("protected-recipe.json"))?;
        target_recipe["name"] = json!(target);
        target_recipe["image"] = json!(args.image);
        target_recipe["port"] = json!(usize::from(args.service_port_base) + index);
        target_recipe["application"] = application.clone();
        target_recipe["agreement"] = agreement.clone();
        target_recipe["policy"] = policy.clone();
        target_recipe["verifier_digest"] = json!(verifier_digest);
        target_recipe["verifier_executable"] = json!(dir.join("trusted-verifier"));
        let target_path = prepare::write(&dir, &format!("{target}.json"), &target_recipe)?;
        config["adapters"]["managed_linux"]["recipes"]
            .as_array_mut()
            .ok_or("recipes absent")?
            .extend([json!(worker_path), json!(target_path)]);
        let capability = format!("evaluation:{slot}");
        let grant = if slot == "learn-source" {
            "grant:learning-source".into()
        } else {
            format!("grant:{slot}")
        };
        config["runtime"]["publication_services"][&capability] = json!(grant);
        let mut service = operator.clone();
        service["actor"] = json!(format!("service:{slot}"));
        service["grant_id"] = json!(grant);
        service["credential_ref"] = json!(format!("credential:{slot}"));
        service["authority"]["resources"]["capability"]["identities"] = json!({"type":"only","values":[format!("managed.{worker}.worker"),format!("managed.{target}"),"milkdrift.resources"]});
        service["authority"]["resources"]["capability"]["operations"] = json!({"type":"only","values":["workspace.execute","resource.evaluate_candidate","resource.evaluate","resource.evidence","resource.inspect"]});
        service["authority"]["resources"]["network"] = json!({"profiles":[],"destinations":[]});
        prepare::credential(&dir.join(format!("{slot}.token")))?;
        config["secret_sources"][format!("credential:{slot}")] =
            json!({"type":"file","path":dir.join(format!("{slot}.token"))});
        config["actors"]
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
