mod evaluate;
mod proposal;
mod setup;
use super::{
    Arguments, EvidenceResult, OPERATOR, STUDY_METHOD, Session, budget, cases, ensure, fs, json,
    load, text, write_private,
};
use crate::client::{Caller, Expected, number};
use crate::qualification::reference_args;
use serde_json::Value;

pub(super) fn write(s: &Session, name: &str, value: &Value) -> EvidenceResult<std::path::PathBuf> {
    let path = s.root.join("learning-05").join(name);
    let bytes = serde_json::to_vec(value)?;
    if path.exists() {
        ensure(
            fs::read(&path)? == bytes,
            "retained learning input differs; start a separately authorized study",
        )?;
    } else {
        write_private(&path, &bytes)?;
    }
    Ok(path)
}
pub(super) fn command(
    s: &Session,
    key: &str,
    value: Value,
    caller: Caller,
    expected: Expected,
) -> EvidenceResult<Value> {
    let path = write(s, &format!("{key}.json"), &value)?;
    let result = s.call(key, args!["learning", path.display()], expected, caller)?;
    Ok(if result["status"] == "success" {
        result["value"]["value"].clone()
    } else {
        result
    })
}
pub(super) fn receipt(command: &str, actor: &str) -> Value {
    json!({"actor":actor,"command":command})
}
pub(super) fn key(args: &Arguments, stem: &str) -> String {
    let stem = if stem == "learning-candidate" && args.candidate_validation > 1 {
        format!("{stem}-validation-{}", args.candidate_validation)
    } else {
        stem.into()
    };
    if args.proposal_attempt == 1 {
        stem
    } else {
        format!("{stem}-{}", args.proposal_attempt)
    }
}
pub(super) fn study_receipt(args: &Arguments, stem: &str, actor: &str) -> Value {
    receipt(&key(args, stem), actor)
}
pub(super) fn revision(s: &Session, id: &str) -> EvidenceResult<Value> {
    let path = s.root.join("learning-05").join(format!("{id}.json"));
    if !path.exists() {
        s.ok(
            &format!("export-{id}"),
            args!["blueprint", "export", id, "--output", path.display()],
        )?;
    }
    let (document, _) =
        milkdrift_blueprint::BlueprintRevisionDocument::from_json(&fs::read(path)?)?;
    Ok(serde_json::from_slice::<Value>(&document.to_canonical_json()?)?["revision"].clone())
}
pub(super) fn upload(s: &Session, key: &str, bytes: &[u8], media: &str) -> EvidenceResult<Value> {
    let path = s.root.join("learning-05").join(format!("{key}.input"));
    if path.exists() {
        ensure(fs::read(&path)? == bytes, "retained upload input differs")?;
    } else {
        write_private(&path, bytes)?;
    }
    let value = s.ok(
        key,
        args![
            "artifact",
            "upload",
            path.display(),
            "--host",
            "host:slotbook-test",
            "--upload-id",
            key,
            "--media-type",
            media
        ],
    )?["value"]
        .clone();
    Ok(
        json!({"artifact":value["artifact_id"],"digest":value["digest"],"media_type":value["content_type"],"size_bytes":value["size"]}),
    )
}
pub(super) fn cap_reference(reference: &Value) -> Value {
    json!({"identity":reference["artifact"],"digest":reference["digest"],"media_type":reference["media_type"],"size_bytes":reference["size_bytes"]})
}
pub(super) fn setup(s: &Session, slot: &mut Value) -> EvidenceResult {
    for kind in ["worker", "target"] {
        let name = text(&slot[kind])?;
        let recipe = s.root.join("learning-05").join(format!("{name}.json"));
        // Recipe digests come from the maintained daemon bootstrap production reader; this helper
        // never hashes JSON with an independent recipe convention.
        let value = load(&recipe)?;
        let bytes = serde_json::to_vec(&value)?;
        let reference = if kind == "worker" {
            let document = milkdrift_managed_linux::LinuxRecipe::from_json(&bytes)?;
            serde_json::to_value(document.reference()?)?
        } else {
            let document = milkdrift_managed_linux::ProtectedServiceRecipe::from_json(&bytes)?;
            serde_json::to_value(document.reference()?)?
        };
        s.resource(
            &format!("{name}-apply"),
            "apply",
            0,
            reference_args(&reference)?,
            name,
            Expected::Success,
        )?;
        let actual = s.resource(
            &format!("{name}-state"),
            "inspect",
            0,
            vec![],
            name,
            Expected::Success,
        )?;
        ensure(
            actual["pending"].is_null() && actual["generation"].as_u64().is_some_and(|n| n > 0),
            "learning resource did not reach a verified generation",
        )?;
        slot[format!("{kind}_state")] = actual;
    }
    Ok(())
}
pub(super) fn slot_declaration(slot: &Value, request: &str) -> Value {
    json!({"invocation":{"actor":OPERATOR,"request":request},"workspace":slot["worker"],"workspace_configuration":slot["worker_state"]["recipe"]["digest"],"workspace_generation":slot["worker_state"]["generation"],"target":slot["target"],"configuration":slot["target_state"]["recipe"]["digest"],"generation":slot["target_state"]["generation"]})
}

pub(super) fn worker(
    s: &mut Session,
    key: &str,
    installation: &str,
    argv: &[&str],
    capture: bool,
) -> EvidenceResult<Value> {
    let input = write(
        s,
        &format!("{key}-inputs.json"),
        &json!([{"name":"command","value":{"type":"inline","value":{"argv":argv,"stdout_artifact":capture}}}]),
    )?;
    let execution = s.invoke(
        key,
        &format!("managed.{installation}.worker"),
        "workspace.execute",
        &input,
        Caller::Operator,
    )?;
    Ok(s.ok(
        &format!("{key}-wait"),
        args!["invocation", "wait", execution],
    )?["value"]
        .clone())
}

pub(in crate::learning) fn run(
    args: &Arguments,
    s: &mut Session,
    source: &Value,
) -> EvidenceResult {
    let dir = s.root.join("learning-05");
    let baseline = load(dir.join("governed.json"))?["revision"].clone();
    if dir.join("accepted-slots.json").exists() {
        let slots = load(dir.join("accepted-slots.json"))?
            .as_array()
            .ok_or("slots absent")?
            .clone();
        let selection = command(
            s,
            "learning-selected-inspect",
            json!({"type":"inspect","receipt":receipt("learning-select",OPERATOR)}),
            Caller::Operator,
            Expected::Success,
        )?;
        let mut declaration = load(dir.join("learning-declare.json"))?["declaration"].clone();
        declaration["proposal_run"] = json!(key(args, "slotbook-learning-proposal"));
        if args.proposal_attempt > 1 {
            let profile = upload(
                s,
                &key(args, "learning-proposal-profile"),
                &fs::read(&args.model_profile)?,
                "application/json",
            )?;
            let provenance = declaration["provenance"]
                .as_array_mut()
                .ok_or("declaration provenance absent")?;
            if !provenance.contains(&profile) {
                provenance.push(profile);
            }
            command(
                s,
                &key(args, "learning-declare"),
                json!({"type":"declare","declaration":declaration}),
                Caller::Operator,
                Expected::Success,
            )?;
        }
        evaluate::preauthorize(args, s, &baseline, &declaration, &slots)?;
        let result = proposal::run(args, s, &baseline, &selection, &declaration)?;
        if args.proposal_only {
            return Ok(());
        }
        evaluate::run(args, s, &baseline, &result, &declaration, &slots)?;
        return setup::run(args, s, &slots);
    }
    s.ok(
        "learning-base-import",
        args!["blueprint", "import", dir.join("base.json").display()],
    )?;
    s.ok(
        "learning-governed-import",
        args!["blueprint", "import", dir.join("governed.json").display()],
    )?;
    let mut slots = load(dir.join("slots.json"))?
        .as_array()
        .ok_or("slots absent")?
        .clone();
    for slot in &mut slots {
        setup(s, slot)?;
        worker(
            s,
            &format!("{}-knowledge", text(&slot["name"])?),
            text(&slot["worker"])?,
            &["/fixtures/slotbook-workspace", "knowledge", "/workspace"],
            false,
        )?;
    }
    let source_slot = slots.last().ok_or("source slot absent")?;
    let policy = load(dir.join("policy.json"))?;
    let source_failure = s.resource(
        "learning-source-failure",
        "evidence",
        0,
        args!["--evaluation", text(&source["failure_evidence"])?],
        "slotbook-test",
        Expected::Success,
    )?;
    let source_pass = s.resource(
        "learning-source-pass",
        "evidence",
        0,
        args![
            "--evaluation",
            text(&source["protected_target"]["accepted_evaluation"])?
        ],
        "slotbook-test",
        Expected::Success,
    )?;
    let exported = worker(
        s,
        "learning-knowledge-export",
        text(&source_slot["worker"])?,
        &["/bin/cat", "/workspace/KNOWLEDGE.md"],
        true,
    )?;
    let reference = exported["observations"]
        .as_array()
        .ok_or("knowledge observations absent")?
        .iter()
        .find(|o| {
            o["category"] == "artifact"
                && o["event"]["kind"]["reference"]["media_type"] == "application/octet-stream"
        })
        .ok_or("knowledge export absent")?["event"]["kind"]["reference"]
        .clone();
    let path = dir.join("KNOWLEDGE.md");
    s.ok(
        "learning-knowledge-download",
        args![
            "artifact",
            "get",
            text(&reference["identity"])?,
            "--output",
            path.display()
        ],
    )?;
    let guidance = upload(s, "learning-guidance", &fs::read(path)?, "text/plain")?;
    let mut selected = Vec::new();
    for (key, value) in [
        ("source-failure", source_failure),
        ("source-pass", source_pass),
        ("source-repair", load(s.root.join("repair-proposal.json"))?),
        ("source-method", load(s.root.join("governed.json"))?),
        ("reusable-baseline", json!({"revision":baseline})),
    ] {
        selected.push(upload(
            s,
            &format!("learning-{key}"),
            &serde_json::to_vec(&value)?,
            "application/json",
        )?);
    }
    let brief = "Build a service for a community workshop with two seats per session. Visitors should see current availability without signing in. Make reservations frictionless: no accounts, just a name and booking details. Keep setup lightweight; an in-memory prototype is fine. The service must also satisfy operator requirements for authenticated mutations, private names and durable bookings. Use the supplied test target and preserve bookings when it restarts. The operator requirements govern deployment. Observed decisions: the initial native seeded fixture interpreted convenience as anonymous writes and in-memory storage; these are harness choices, not model choices. The retained repair replaced that fixture and introduced an investigation check. Missing model planning evidence remains unknown.";
    selected.push(upload(
        s,
        "learning-source-input",
        brief.as_bytes(),
        "text/plain",
    )?);
    let sequence = number(&source["completed_run"]["sequence"])?;
    let pages = json!([{"run":source["internal_run"],"first":1,"count":3},{"run":source["internal_run"],"first":10,"count":3},{"run":source["internal_run"],"first":sequence-3,"count":4}]);
    let selection = command(
        s,
        "learning-select",
        json!({"type":"select","selection":{"method":baseline["id"],"workspace":source_slot["worker"],"guidance":guidance,"artifacts":selected,"pages":pages,"supersedes":null,"approval":null}}),
        Caller::Operator,
        Expected::Success,
    )?;
    let mut pairs = Vec::new();
    for (i, (name, application, brief)) in cases().into_iter().enumerate() {
        let input = upload(
            s,
            &format!("learning-input-{name}"),
            &serde_json::to_vec(
                &json!({"brief":brief,"application":application,"requirements":policy["required_checks"],"operator_obligations":"Authenticated writes, private names, half-open positive-capacity intervals, durable bookings, supplied cancellation cutoff, exact protected deployment; convenience wording never overrides these requirements."}),
            )?,
            "application/json",
        )?;
        pairs.push(json!({"name":name,"input":input,"baseline":slot_declaration(&slots[i*2],&format!("evaluate-{name}-baseline")),"candidate":slot_declaration(&slots[i*2+1],&format!("evaluate-{name}-candidate"))}));
    }
    let profile = upload(
        s,
        "learning-model-profile",
        &fs::read(&args.model_profile)?,
        "application/json",
    )?;
    let declaration = json!({"schema_version":1,"lane":"seeded_fixture","selection":receipt("learning-select",OPERATOR),"baseline":baseline["id"],"proposal_run":"slotbook-learning-proposal","agreement":baseline["semantic"]["agreement"]["digest"],"policy":baseline["semantic"]["agreement"]["effect_policy"],"verifier":policy["verifier"],"checks":policy["required_checks"],"input_field":"product","candidate_output":"candidate","pairs":pairs,"allowance":budget(),"maximum_duration_ms":3600000,"maximum_submissions":3,"minimum_repair_reduction":2,"provenance":[profile],"unknowns":["Model server effective template, sampling and tokenizer bounds are operator-unqualified; proposal generation is outside the paired execution account.","Native seeded implementation and repair are controlled fixtures; no live-model implementation or source planning failure is claimed."],"publication":STUDY_METHOD,"generation":2});
    command(
        s,
        "learning-declare",
        json!({"type":"declare","declaration":declaration}),
        Caller::Operator,
        Expected::Success,
    )?;
    write(s, "accepted-slots.json", &json!(slots))?;
    evaluate::preauthorize(args, s, &baseline, &declaration, &slots)?;
    let result = proposal::run(args, s, &baseline, &selection, &declaration)?;
    if args.proposal_only {
        return Ok(());
    }
    evaluate::run(args, s, &baseline, &result, &declaration, &slots)?;
    setup::run(args, s, &slots)
}
