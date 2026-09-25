//! The endpoint receives only explicit immutable source documents, never evaluator files.
use super::super::{MODEL, prepare, save_config};
use super::{
    Arguments, Caller, EvidenceResult, Expected, OPERATOR, Session, Value, command, ensure, fs,
    json, key, load, study_receipt, text, write,
};
use milkdrift_control::WorkflowProposalDocument;

const LEARNER: &str = "agent:slotbook-learner";
fn metadata_reference(value: &Value) -> Value {
    json!({"identity":value["artifact_id"],"digest":value["digest"],"media_type":value["content_type"],"size_bytes":value["size"]})
}
pub(super) fn run(
    args: &Arguments,
    s: &mut Session,
    baseline: &Value,
    selection: &Value,
    declaration: &Value,
) -> EvidenceResult<Value> {
    let model_run = text(&declaration["proposal_run"])?;
    let file = |stem: &str| format!("{}.json", key(args, stem));
    let source = load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/operator/model.json"),
    )?;
    let mut model = source["revision"]["semantic"]["nodes"]["model"].clone();
    model["kind"]["config"]["requirement"]["exact_capability"] = json!(MODEL);
    model["kind"]["config"]["requirement"]["provider_profile"] =
        load(&args.model_profile)?["identity"].clone();
    model["kind"]["config"]["requirement"]["trust_zones"] = json!([]);
    model["kind"]["config"]["context_policy"]["include_direct_inputs"] = json!(true);
    model["kind"]["config"]["context_policy"]["include_categories"] = json!(["direct_input"]);
    model["kind"]["config"]["context_policy"]["fail_closed"] = json!(true);
    let mut prompt = format!(
        "Propose one evidence-supported reusable Slotbook method change. The problem is that unclear planning can produce contradictory implementation and avoidable repair. Read the explicitly selected source artifacts. The source failure and repair are labeled deterministic native fixtures, not your or another model's past choices. Missing model decisions remain unknown. You may change only the governed editable region; preserve its agreement, verifier nodes, interfaces, obligations and all fixed bounds. Do not prescribe or optimize toward unseen test cases. A planning responsibility is a hypothesis, not a required node, name or answer. Cite exact selected source artifact identities for the decisions, contradiction, failed checks and repair. State expected benefit in rationale, applicability/unchanged obligations in assumptions, and disconfirming evidence in risk_notes. Return the structured proposal_document_json string containing a schema_version:1 draft object with fields identity, proposer, provenance, workflow, run, base_revision, base_digest, observed_run_sequence, mutation, rationale, rationale_artifact, risk_notes, assumptions, evidence, artifacts, application_policy, requested_action, claimed_stop. Use identity learning-candidate, proposer {LEARNER}, provenance {{\"type\":\"direct\"}}, workflow slotbook-learning, run null, base_revision {}, base_digest {}, observed_run_sequence null, mutation an array of ordinary blueprint mutations, rationale_artifact null, evidence an array of {{\"kind\":\"worker_observation\",\"id\":\"EXACT_SELECTED_ARTIFACT_ID\"}}, artifacts [], application_policy propose_only, requested_action null, claimed_stop complete. Do not change the declaration or claim a test pass. The current complete reusable blueprint and actual source evidence follow as separately selected context. The metric is fewer failed verification submissions under identical obligations, with no regression and at least two fewer repairs in the fixed comparison. A new name or extra prose alone is insufficient.",
        text(&baseline["id"])?,
        text(&baseline["content_digest"])?
    );
    prompt.push_str(" Authoring contract: proposal_document_json must encode exactly {\"schema_version\":1,\"draft\":{...the fields above...}}. risk_notes and assumptions are arrays of strings. Mutation operations use a type tag: {\"type\":\"replace_node\",\"node\":COMPLETE_NODE}, {\"type\":\"add_node\",\"node\":COMPLETE_NODE}, {\"type\":\"remove_node\",\"node\":EXISTING_ID}, {\"type\":\"add_edge\",\"edge\":COMPLETE_EDGE}, {\"type\":\"remove_edge\",\"edge\":EXISTING_ID}, or replace_edge with a complete edge. These are not JSON Patch operations. Preserve the node and edge schemas visible in the complete baseline. New task nodes require an existing permitted capability requirement and executable configuration; an invented responsibility or node kind is not executable. Copy an existing node's full fields when proposing its replacement. The six immutable verify-related obligations and terminal interface must remain unchanged. This schema guidance does not prescribe a mutation or hypothesis.");
    let editable = baseline["semantic"]["nodes"]
        .as_object()
        .ok_or("baseline nodes absent")?
        .keys()
        .filter(|id| id.starts_with("repair."))
        .cloned()
        .collect::<Vec<_>>();
    let citations = selection["selection"]["artifacts"]
        .as_array()
        .ok_or("selected artifacts absent")?
        .iter()
        .map(|a| a["artifact"].clone())
        .collect::<Vec<_>>();
    prompt.push_str(&format!(" Distinguish historical source methods from the current reusable baseline. Existing editable nodes in the current baseline are exactly {}. A replace_node operation must name one of these existing nodes; historical repair.begin/repair.end nodes are not in this baseline. The permitted artifact citation IDs are {}. Cite these IDs, not a verifier identity or digest mentioned inside their content. Keep rationale below 2048 bytes. All argv values must be strings in a closed JSON array; stdout_artifact is a sibling field after that array. Return syntactically valid JSON inside proposal_document_json. These structural constraints do not select a method change.",serde_json::to_string(&editable)?,serde_json::to_string(&citations)?));
    let request = &mut model["data_inputs"]["milkdrift.model_task"]["binding"]["value"]["request"];
    request["messages"] =
        json!([{"role":"user","parts":[{"type":"text","text":prompt}],"tool_call_id":null}]);
    request["maximum_output_units"] = json!(4096);
    request["streaming"] = json!(false);
    request["extensions"] = json!({"org.milkdrift.openai/request":{"reasoning_effort":"none"}});
    request["structured_output"] =
        serde_json::to_value(milkdrift_control::workflow_proposal_structured_output()?)?;
    model["data_outputs"]["structured_output"] =
        prepare::port("Output", prepare::artifact_schema(), Value::Null, false);
    let mut selected = vec![
        selection["artifact"].clone(),
        selection["selection"]["guidance"].clone(),
    ];
    selected.extend(
        selection["selection"]["artifacts"]
            .as_array()
            .ok_or("source artifacts absent")?
            .iter()
            .cloned(),
    );
    model["kind"]["config"]["context_policy"]["explicit_evidence"] = json!(
        selected
            .iter()
            .map(|reference| serde_json::to_string(
                &json!({"type":"artifact","reference":reference})
            ))
            .collect::<Result<Vec<_>, _>>()?
    );
    for (i, reference) in selected.iter().enumerate() {
        model["data_inputs"][format!("source-{i}")] = prepare::port(
            "Input",
            prepare::artifact_schema(),
            json!({"type":"artifact","reference":serde_json::to_string(reference)?,"contract":prepare::artifact_schema()}),
            true,
        );
    }
    // Resume the exact authored model invocation, including any retained local reporting failure.
    // A source-code correction applies to newly authored studies, never to an entered invocation.
    let document = s
        .root
        .join("learning-05")
        .join(file("proposal-workflow-v1"));
    if document.exists() {
        model = load(&document)?["revision"]["semantic"]["nodes"]["model"].clone();
    }
    let done = prepare::node(
        "done",
        json!({"type":"terminal","outcome":"success"}),
        true,
        false,
    );
    let path = if document.exists() {
        s.root
            .join("learning-05")
            .join(file("proposal-mutations-v1"))
    } else {
        write(
            s,
            &file("proposal-mutations-v1"),
            &json!([{"type":"add_node","node":model},{"type":"add_node","node":done},{"type":"add_edge","edge":prepare::edge("finish","model","done","control","next","in")}]),
        )?
    };
    if !document.exists() {
        prepare::local(
            &s.cli,
            args![
                "blueprint",
                "create",
                path.display(),
                "--workflow",
                model_run,
                "--author",
                OPERATOR,
                "--output",
                document.display()
            ]
            .as_slice(),
        )?;
    }
    let workflow = load(&document)?["revision"].clone();
    s.ok(
        &key(args, "learning-proposal-import-v1"),
        args!["blueprint", "import", document.display()],
    )?;
    s.ok(
        &key(args, "learning-proposal-start"),
        args!["run", "start", model_run, model_run, text(&workflow["id"])?],
    )?;
    // Uncertainty needs inspection, not ten minutes of waiting for a terminal event that cannot
    // arrive without reconciliation. Bound both observation and the external provider separately.
    for poll in 0..=360 {
        let state = s.ok(
            &key(args, "learning-proposal-observation"),
            args!["run", "show", model_run],
        )?["value"]
            .clone();
        if !state["terminal"].is_null() || state["uncertainty_count"] != 0 {
            break;
        }
        ensure(
            poll < 360,
            "model proposal exceeded the bounded observation window",
        )?;
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    let run = s.ok(
        &key(args, "learning-proposal-run"),
        args!["run", "show", model_run],
    )?["value"]
        .clone();
    let attempt = run["nodes"]
        .as_array()
        .ok_or("proposal nodes absent")?
        .iter()
        .find(|n| n["node_id"] == "model")
        .ok_or("model node absent")?["latest_attempt_id"]
        .clone();
    let attempt = s.ok(
        &key(args, "learning-proposal-attempt"),
        args!["attempt", "inspect", model_run, text(&attempt)?],
    )?["value"]
        .clone();
    write(
        s,
        &file("proposal-observation"),
        &json!({"run":run,"attempt":attempt,"limit":"A retained model response may be analyzed even when local reporting failed. This does not settle unknown provider usage or claim the proposal run succeeded."}),
    )?;
    let response = attempt["outputs"]
        .as_array()
        .ok_or("model outputs absent")?
        .iter()
        .find(|o| o["name"] == "model_response")
        .ok_or("structured response absent")?["artifact"]
        .clone();
    let output = s.root.join("learning-05").join(file("model-response"));
    if !output.exists() {
        s.ok(
            &key(args, "learning-response-download"),
            args![
                "artifact",
                "get",
                text(&response["artifact_id"])?,
                "--output",
                output.display()
            ],
        )?;
    }
    let response_document = milkdrift_model::ModelResponseDocument::from_json(&fs::read(&output)?)?;
    let original = match WorkflowProposalDocument::from_model_response(response_document.body()) {
        Ok(proposal) => proposal,
        Err(error) => {
            let refusal = json!({"run":model_run,"response":response,"outcome":"invalid_proposal","reason":error.to_string(),"candidate_substituted":false});
            write(s, &file("proposal-refusal"), &refusal)?;
            super::upload(
                s,
                &key(args, "learning-proposal-refusal"),
                &serde_json::to_vec(&refusal)?,
                "application/json",
            )?;
            return Err(error.into());
        }
    };
    let provenance = json!({"type":"model","capability":MODEL,"invocation":attempt["invocation_id"],"model_profile":attempt["provider_profile"],"context_manifest":metadata_reference(&attempt["context_manifest"]),"response_artifact":metadata_reference(&response)});
    let mut draft: Value = serde_json::to_value(original.proposal())?;
    draft
        .as_object_mut()
        .ok_or("proposal absent")?
        .remove("digest");
    draft["proposer"] = json!(LEARNER);
    draft["identity"] = json!("learning-candidate");
    draft["provenance"] = provenance;
    draft["mutation"] = serde_json::to_value(original.proposal().mutation().operations())?;
    let proposal = json!({"schema_version":1,"draft":draft});
    let mut authorized_artifacts: Vec<Value> =
        selected.iter().map(|a| a["artifact"].clone()).collect();
    authorized_artifacts.extend([
        response["artifact_id"].clone(),
        attempt["context_manifest"]["artifact_id"].clone(),
    ]);
    learner(s, &authorized_artifacts)?;
    for pair in declaration["pairs"].as_array().ok_or("pairs absent")? {
        s.call(
            &format!("learner-denied-{}", text(&pair["name"])?),
            args![
                "artifact",
                "get",
                text(&pair["input"]["artifact"])?,
                "--output",
                s.root.join("learning-05/forbidden-input").display()
            ],
            Expected::Refused,
            Caller::Learner,
        )?;
    }
    command(
        s,
        &key(args, "learner-cannot-declare"),
        json!({"type":"declare","declaration":declaration}),
        Caller::Learner,
        Expected::Refused,
    )?;
    command(
        s,
        &key(args, "learner-cannot-inspect-held-out-declaration"),
        json!({"type":"inspect","receipt":study_receipt(args,"learning-declare",OPERATOR)}),
        Caller::Learner,
        Expected::Refused,
    )?;
    for (label, mutation) in [
        ("invented-evidence", false),
        ("forbidden-verifier-edit", true),
    ] {
        let mut forged = proposal.clone();
        if mutation {
            forged["draft"]["mutation"] = json!([{"type":"remove_node","node":"verify-1"}]);
        } else {
            forged["draft"]["evidence"] =
                json!([{"kind":"worker_observation","id":"invented-source"}]);
        }
        command(
            s,
            &key(args, label),
            json!({"type":"candidate","declaration":study_receipt(args,"learning-declare",OPERATOR),"proposal":forged,"expected_benefit":original.proposal().rationale(),"applicability":original.proposal().assumptions().join("; "),"counterevidence":original.proposal().risk_notes().join("; ")}),
            Caller::Learner,
            Expected::Refused,
        )?;
    }
    let result = command(
        s,
        &key(args, "learning-candidate"),
        json!({"type":"candidate","declaration":study_receipt(args,"learning-declare",OPERATOR),"proposal":proposal,"expected_benefit":original.proposal().rationale(),"applicability":original.proposal().assumptions().join("; "),"counterevidence":original.proposal().risk_notes().join("; ")}),
        Caller::Learner,
        Expected::Success,
    )?;
    s.stop()?;
    s.start()?;
    command(
        s,
        &key(args, "learning-candidate"),
        load(s.root.join("learning-05").join(file("learning-candidate")))?,
        Caller::Learner,
        Expected::Success,
    )?;
    Ok(result)
}

fn learner(s: &mut Session, artifacts: &[Value]) -> EvidenceResult {
    s.stop()?;
    let mut config: Value = toml::from_str(&fs::read_to_string(s.root.join("host/daemon.toml"))?)?;
    let mut actor = config["actors"]
        .as_array()
        .ok_or("actors absent")?
        .iter()
        .find(|a| a["actor"] == OPERATOR)
        .ok_or("operator absent")?
        .clone();
    actor["actor"] = json!(LEARNER);
    actor["preset"] = json!("advisor");
    actor["grant_id"] = json!("grant:slotbook-learner");
    actor["credential_ref"] = json!("credential:slotbook-learner");
    let resources = &mut actor["authority"]["resources"];
    resources["artifacts"]["identities"] = json!({"type":"only","values":artifacts});
    resources["capability"]["identities"] = json!({"type":"any"});
    resources["capability"]["operations"] = json!({"type":"only","values":["learning.candidate","learning.inspect","workflow.propose","workspace.execute","resource.evaluate_candidate"]});
    resources["filesystem"] = json!([]);
    resources["network"] = json!({"profiles":[],"destinations":[]});
    if !s.root.join("learning-05/learner.token").exists() {
        prepare::credential(&s.root.join("learning-05/learner.token"))?;
    }
    config["secret_sources"]["credential:slotbook-learner"] =
        json!({"type":"file","path":s.root.join("learning-05/learner.token")});
    let actors = config["actors"].as_array_mut().ok_or("actors absent")?;
    if let Some(existing) = actors.iter().find(|a| a["actor"] == LEARNER) {
        ensure(
            existing == &actor,
            "retained learner grant differs; explicit new grant revision required",
        )?;
    } else {
        actors.push(actor);
    }
    save_config(&s.root, &config)?;
    s.start()
}
