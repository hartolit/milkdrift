use super::{
    EvidenceResult, Expected, Qualify, Session, Value, candidate_args, ensure, evidence, inspect,
    invocation::{self, Link},
    json, number, passed, target, text,
};
use milkdrift_evidence::application::run_command;
use std::{fs, io::Read, process::Command, time::Duration};

fn container(name: &str) -> EvidenceResult<Value> {
    let output = run_command(
        Command::new("/usr/bin/podman").args(["inspect", name]),
        None,
        Duration::from_secs(30),
    )?;
    ensure(output.status.success(), "served container not inspectable")?;
    let value: Value = serde_json::from_str(&output.stdout)?;
    value
        .get(0)
        .cloned()
        .ok_or_else(|| "served container absent".into())
}
#[allow(clippy::too_many_arguments)] // Each input is retained evidence for this single qualification scenario.
pub(super) fn finish(
    s: &mut Session,
    args: &Qualify,
    link: &Link,
    version: u64,
    failed_id: &str,
    failure: &Value,
    completed: &Value,
    candidate_bytes: &[u8],
) -> EvidenceResult {
    let after = inspect(s, "published-inspect")?;
    ensure(
        after["pending"].is_null() && after["observed_running"] == true,
        "protected publication did not start",
    )?;
    let accepted_id = text(&after["accepted_evaluation"])?;
    let accepted = evidence(s, "accepted-evidence", accepted_id)?;
    ensure(
        passed(&accepted),
        "accepted evaluation does not pass every check",
    )?;
    let verified = &accepted["subject"]["artifact"];
    let exported = s.logs.join("deployed-candidate");
    s.ok(
        "candidate-download",
        args![
            "artifact",
            "get",
            text(&verified["artifact"])?,
            "--output",
            exported.display()
        ],
    )?;
    ensure(
        fs::read(&exported)? == candidate_bytes,
        "deployed artifact differs from corrected binary",
    )?;
    let service = text(
        &after["resources"]
            .as_array()
            .ok_or("resources absent")?
            .iter()
            .find(|r| r["kind"] == "service")
            .ok_or("service absent")?["identity"],
    )?;
    let deployed = container(service)?;
    let mounted = deployed["Mounts"]
        .as_array()
        .ok_or("mounts absent")?
        .iter()
        .find(|m| m["Destination"] == "/candidate/app")
        .ok_or("candidate mount absent")?;
    ensure(
        mounted["RW"] == false && fs::read(text(&mounted["Source"])?)? == candidate_bytes,
        "served mount differs from immutable candidate",
    )?;
    super::super::prepare::write(&s.logs, "served-container.json", &deployed)?;
    target(
        s,
        "wrong-generation-refused",
        "publish",
        number(&after["version"])?,
        args!["--evaluation", accepted_id],
        Expected::Refused,
    )?;
    s.ok(
        "timeline",
        args!["run", "timeline", link.internal, "--limit", 256],
    )?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()?
        .get("http://127.0.0.1:19848/health")
        .send()?;
    ensure(
        response.status().is_success(),
        "protected service health refused",
    )?;
    let mut bytes = Vec::new();
    response.take(16_385).read_to_end(&mut bytes)?;
    ensure(
        bytes.len() <= 16_384,
        "protected service health exceeds bound",
    )?;
    let health: Value = serde_json::from_slice(&bytes)?;
    ensure(
        health["status"] == "ready",
        "protected service health differs",
    )?;
    s.stop()?;
    s.start()?;
    let reopened = inspect(s, "reopened-inspect")?;
    ensure(
        reopened["generation"] == after["generation"] && reopened["observed_running"] == true,
        "restart lost protected generation",
    )?;
    invocation::replay(s, link)?;
    if args.published {
        let same = inspect(s, "wrapper-replay-no-deployment")?;
        ensure(
            same["generation"] == after["generation"] && same["version"] == after["version"],
            "wrapper replay deployed again",
        )?;
    }
    target(
        s,
        "publish-candidate",
        "publish",
        version,
        args!["--evaluation", accepted_id],
        Expected::Refused,
    )?;
    let direct_version = number(&reopened["version"])?;
    let direct = target(
        s,
        "direct-evaluate",
        "evaluate",
        direct_version,
        candidate_args(verified, "artifact")?,
        Expected::Success,
    )?;
    let direct_id = text(&direct["evaluation"]["identity"])?;
    let direct_checks = evidence(s, "direct-evidence", direct_id)?;
    ensure(passed(&direct_checks), "direct verification did not pass")?;
    let renewed = target(
        s,
        "renew-evaluate",
        "evaluate",
        direct_version,
        candidate_args(verified, "artifact")?,
        Expected::Success,
    )?;
    let renewed_id = text(&renewed["evaluation"]["identity"])?;
    let renewed_checks = evidence(s, "renew-evidence", renewed_id)?;
    ensure(
        renewed_id != direct_id
            && renewed_checks["subject"] == direct_checks["subject"]
            && passed(&renewed_checks),
        "renewed verification changed subject or failed",
    )?;
    ensure(
        evidence(s, "prior-evidence-unchanged", direct_id)? == direct_checks,
        "renewal changed prior evidence",
    )?;
    target(
        s,
        "direct-publish",
        "publish",
        direct_version,
        args!["--evaluation", renewed_id],
        Expected::Success,
    )?;
    let published = inspect(s, "direct-published")?;
    ensure(
        published["pending"].is_null()
            && number(&published["generation"])? == number(&after["generation"])? + 1
            && published["observed_running"] == true,
        "renewed publication differs",
    )?;
    let service = text(
        &published["resources"]
            .as_array()
            .ok_or("resources absent")?
            .iter()
            .find(|r| r["kind"] == "service")
            .ok_or("service absent")?["identity"],
    )?
    .to_owned();
    let prior = container(&service)?;
    s.stop()?;
    s.start()?;
    target(
        s,
        "direct-publish",
        "publish",
        direct_version,
        args!["--evaluation", renewed_id],
        Expected::Success,
    )?;
    let replayed = inspect(s, "after-exact-replay")?;
    ensure(
        replayed["generation"] == published["generation"]
            && replayed["version"] == published["version"],
        "exact publication replay changed generation",
    )?;
    let again = container(&service)?;
    ensure(
        again["Id"] == prior["Id"] && again["State"]["StartedAt"] == prior["State"]["StartedAt"],
        "exact replay restarted service",
    )?;
    ensure(
        evidence(s, "failure-retained", failed_id)? == *failure,
        "failed evidence was overwritten",
    )?;
    s.ok("run-reopened", args!["run", "show", link.internal])?;
    for name in ["slotbook-test", "slotbook-build"] {
        let state = s.resource(
            &format!("cleanup-inspect-{name}"),
            "inspect",
            0,
            vec![],
            name,
            Expected::Success,
        )?;
        s.resource(
            &format!("cleanup-{name}"),
            "remove",
            number(&state["version"])?,
            vec![],
            name,
            Expected::Success,
        )?;
        let removed = s.resource(
            &format!("removed-{name}"),
            "inspect",
            0,
            vec![],
            name,
            Expected::Success,
        )?;
        ensure(
            removed["state"] == "removed",
            "qualification resource removal incomplete",
        )?;
    }
    s.write("qualification.json",&json!({"completed_run":completed,"protected_target":after,"failure_evidence":failed_id,"public_execution":link.public,"internal_run":link.internal,"invocation_mode":args.invocation_mode.name(),"lane":"compiled Rust seeded repair; rootless Linux; finite six-check HTTP verifier"}))?;
    Ok(())
}
