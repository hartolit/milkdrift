mod invocation;
mod results;
use super::{
    InvocationMode, Qualify,
    client::{Caller, Expected, Session, load, number, text},
};
use milkdrift_evidence::{EvidenceResult, application::ensure};
use serde_json::{Value, json};
use std::{thread, time::Duration};

pub(super) fn reference_args(reference: &Value) -> EvidenceResult<Vec<String>> {
    Ok(args![
        "--recipe",
        text(&reference["name"])?,
        "--digest",
        text(&reference["digest"])?
    ])
}
pub(super) fn candidate_args(candidate: &Value, key: &str) -> EvidenceResult<Vec<String>> {
    Ok(args![
        "--artifact",
        text(&candidate[key])?,
        "--digest",
        text(&candidate["digest"])?,
        "--media-type",
        text(&candidate["media_type"])?,
        "--size-bytes",
        number(&candidate["size_bytes"])?
    ])
}
pub(super) fn target(
    s: &Session,
    label: &str,
    action: &str,
    version: u64,
    extra: Vec<String>,
    expected: Expected,
) -> EvidenceResult<Value> {
    s.resource(label, action, version, extra, "slotbook-test", expected)
}
pub(super) fn inspect(s: &Session, label: &str) -> EvidenceResult<Value> {
    target(s, label, "inspect", 0, vec![], Expected::Success)
}
pub(super) fn evidence(s: &Session, label: &str, id: &str) -> EvidenceResult<Value> {
    Ok(target(
        s,
        label,
        "evidence",
        0,
        args!["--evaluation", id],
        Expected::Success,
    )?["evaluation"]
        .clone())
}
pub(super) fn passed(evaluation: &Value) -> bool {
    evaluation["complete"] == true
        && evaluation["checks"]
            .as_array()
            .is_some_and(|checks| !checks.is_empty() && checks.iter().all(|c| c["passed"] == true))
}
pub(super) fn run(args: Qualify) -> EvidenceResult {
    ensure(
        args.published || args.invocation_mode == InvocationMode::Direct,
        "invocation mode requires --published",
    )?;
    let candidate = std::fs::read(&args.candidate)?;
    ensure(
        candidate.len() as u64 <= super::prepare::CANDIDATE_BYTES,
        "candidate exceeds example allocation",
    )?;
    let mut session = Session::prepare(&args)?;
    session.start()?;
    let result = qualify(&mut session, &args, &candidate);
    let stop = session.stop();
    result?;
    stop?;
    println!("finite Rust binary qualification passed");
    Ok(())
}
fn qualify(s: &mut Session, args: &Qualify, candidate_bytes: &[u8]) -> EvidenceResult {
    let prepared = target(
        s,
        "protected-prepare",
        "prepare",
        0,
        reference_args(&s.protected)?,
        Expected::Success,
    )?;
    ensure(
        prepared["state"] == "prepared" && prepared["version"] == 0,
        "protected preparation differs",
    )?;
    target(
        s,
        "protected-apply",
        "apply",
        0,
        reference_args(&s.protected)?,
        Expected::Success,
    )?;
    let state = inspect(s, "protected-before")?;
    ensure(
        state["pending"].is_null() && state["desired_running"] == false,
        "protected target already running",
    )?;
    let version = number(&state["version"])?;
    let document = load(s.root.join("governed.json"))?;
    let revision = &document["revision"];
    ensure(
        revision["semantic"]["nodes"]["verify-candidate"]["data_inputs"]["target"]["binding"]["value"]
            ["expected_version"]
            == version,
        "re-author with the observed --target-version",
    )?;
    target(
        s,
        "raw-start-refused",
        "start",
        version,
        vec![],
        Expected::Refused,
    )?;
    s.resource(
        "worker-apply",
        "apply",
        0,
        reference_args(&s.worker)?,
        "slotbook-build",
        Expected::Success,
    )?;
    let worker = s.resource(
        "worker-inspect",
        "inspect",
        0,
        vec![],
        "slotbook-build",
        Expected::Success,
    )?;
    ensure(
        worker["pending"].is_null()
            && worker["capabilities"]
                .as_array()
                .is_some_and(|a| a.contains(&json!("managed.slotbook-build.worker"))),
        "managed worker not registered",
    )?;
    let input=s.write("seed-inputs.json",&json!([{"name":"command","value":{"type":"inline","value":{"argv":["/bin/sh","-c","cp /fixtures/slotbook-seeded /workspace/app && cat /workspace/app"],"stdout_artifact":true}}}]))?;
    let execution = s.invoke(
        "seed",
        "managed.slotbook-build.worker",
        "workspace.execute",
        &input,
        Caller::Operator,
    )?;
    let observation = s.ok("seed-wait", args!["invocation", "wait", execution])?;
    let candidate = observation["value"]["observations"]
        .as_array()
        .ok_or("seed observations absent")?
        .iter()
        .filter(|o| o["category"] == "artifact")
        .map(|o| &o["event"]["kind"]["reference"])
        .find(|r| r["media_type"] == "application/octet-stream")
        .cloned()
        .ok_or("seed artifact absent")?;
    let evaluated = target(
        s,
        "seed-evaluate",
        "evaluate",
        version,
        candidate_args(&candidate, "identity")?,
        Expected::Success,
    )?;
    let evaluation = text(&evaluated["evaluation"]["identity"])?.to_owned();
    let failure = evidence(s, "seed-evidence", &evaluation)?;
    let failed = failure["checks"]
        .as_array()
        .ok_or("checks absent")?
        .iter()
        .filter(|c| c["passed"] == false)
        .map(|c| text(&c["name"]))
        .collect::<EvidenceResult<Vec<_>>>()?;
    ensure(
        failure["complete"] == true && failed == ["authenticated-mutation", "durable-bookings"],
        "seed did not produce exactly the two declared failures",
    )?;
    target(
        s,
        "failed-publish-refused",
        "publish",
        version,
        args!["--evaluation", evaluation],
        Expected::Refused,
    )?;
    target(
        s,
        "forged-publish-refused",
        "publish",
        version,
        args!["--evaluation", format!("b3_{}", "a".repeat(64))],
        Expected::Refused,
    )?;
    forged_report(s, version, &evaluation)?;
    s.ok(
        "base-import",
        args!["blueprint", "import", s.root.join("base.json").display()],
    )?;
    s.ok(
        "governed-import",
        args![
            "blueprint",
            "import",
            s.root.join("governed.json").display()
        ],
    )?;
    let link = invocation::start(s, args, revision, version)?;
    let run = s.ok("run-before", args!["run", "show", link.internal])?["value"].clone();
    let proposal = json!({"schema_version":1,"draft":{"identity":"repair-slotbook","proposer":"agent:repair","provenance":{"type":"direct"},"workflow":"slotbook","run":link.internal,"base_revision":revision["id"],"base_digest":revision["content_digest"],"observed_run_sequence":run["sequence"],"mutation":load(s.root.join("repair-mutations.json"))?,"rationale":"Retain observed authentication and restart failures, replace the candidate with the compiled repair fixture and obtain fresh trusted evidence.","rationale_artifact":null,"risk_notes":[],"assumptions":["Finite compiled fixture, not model-generated repair"],"evidence":[{"kind":"worker_observation","id":evaluation}],"artifacts":[candidate],"application_policy":"auto_apply_low_risk","requested_action":null,"claimed_stop":"complete"}});
    let proposal = s.write("repair-proposal.json", &proposal)?;
    s.ok(
        "proposal-submit",
        args![
            "--expected-sequence",
            number(&run["sequence"])?,
            "--expected-revision",
            text(&revision["id"])?,
            "proposal",
            "submit",
            proposal.display()
        ],
    )?;
    if args.published {
        let mut observed = false;
        for index in 0..600 {
            let held = s.resource(
                &format!("published-holds-{index}"),
                "inspect",
                0,
                vec![],
                "slotbook-build",
                Expected::Success,
            )?;
            let held: milkdrift_capability::managed::ManagedResponse =
                serde_json::from_value(held)?;
            let blockers = &held.blockers;
            if blockers.iter().any(|b| b.state == "suspended") {
                ensure(
                    blockers.len() == 2
                        && blockers.iter().filter(|b| !b.editing.is_empty()).count() == 1,
                    "published handoff does not have exactly one child writer",
                )?;
                observed = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        ensure(
            observed,
            "published parent never transferred editing to its child",
        )?;
    }
    let completed = s.ok("run-wait", args!["run", "wait", link.internal])?["value"].clone();
    ensure(
        completed["terminal"] == "succeeded" && completed["agreement_adoptions"] == 1,
        "method did not satisfy its agreement",
    )?;
    invocation::complete(s, args, &link)?;
    results::finish(
        s,
        args,
        &link,
        version,
        &evaluation,
        &failure,
        &completed,
        candidate_bytes,
    )
}
fn forged_report(s: &mut Session, version: u64, evaluation: &str) -> EvidenceResult {
    let mut report = target(
        s,
        "failed-report",
        "evidence",
        0,
        args!["--evaluation", evaluation],
        Expected::Success,
    )?;
    for check in report["evaluation"]["checks"]
        .as_array_mut()
        .ok_or("checks absent")?
    {
        check["passed"] = json!(true);
        check["diagnostic"] = json!("untrusted fabricated pass");
    }
    let file = s.write("forged-report.json", &report)?;
    let uploaded = s.ok(
        "forged-upload",
        args![
            "artifact",
            "upload",
            file.display(),
            "--host",
            "host:slotbook-test",
            "--upload-id",
            "forged-positive",
            "--media-type",
            "application/vnd.milkdrift.managed+json"
        ],
    )?;
    let uploaded = &uploaded["value"];
    let input=s.write("forged-inputs.json",&json!([{"name":"target","value":{"type":"inline","value":{"schema_version":3,"command":"forged-serving","installation":"slotbook-test","expected_version":version}}},{"name":"evaluation","value":{"type":"artifact","reference":{"identity":uploaded["artifact_id"],"digest":uploaded["digest"],"media_type":uploaded["content_type"],"size_bytes":uploaded["size"]}}}]))?;
    let execution = s.invoke(
        "forged",
        "milkdrift.resources",
        "resource.publish_candidate",
        &input,
        Caller::Operator,
    )?;
    let observation = s.call(
        "forged-wait",
        args!["invocation", "wait", execution],
        Expected::Refused,
        Caller::Operator,
    )?;
    let terminals = observation["value"]["observations"]
        .as_array()
        .ok_or("observations absent")?
        .iter()
        .filter(|o| o["category"] == "terminal" || o["category"] == "uncertainty")
        .map(|o| &o["event"]["kind"]["terminal"])
        .collect::<Vec<_>>();
    ensure(
        terminals.len() == 1
            && terminals[0]["status"] != "success"
            && serde_json::to_string(&terminals)?.contains("verification is failed"),
        "fabricated verification report was not refused",
    )?;
    let unchanged = inspect(s, "after-forged-report")?;
    ensure(
        unchanged["generation"] == 1
            && unchanged["observed_running"] == false
            && unchanged["pending"].is_null(),
        "forged evidence changed target",
    )
}
