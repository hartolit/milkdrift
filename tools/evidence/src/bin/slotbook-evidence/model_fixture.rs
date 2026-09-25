//! Deliberate proposal fixtures exercise the same HTTP/structured-response path as real models.
//! They read only selected source documents, never the comparison inputs or evaluator state.
use crate::client::{load, text};
use clap::ValueEnum;
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, write_private},
    http_fixture::read_request,
};
use serde_json::{Value, json};
use std::{
    io::Write as _,
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Useful,
    NoImprovement,
    Malformed,
    InventedEvidence,
    ForbiddenEdit,
}

#[derive(clap::Args)]
pub(super) struct Arguments {
    /// Private qualification directory; source selection must exist before the request arrives.
    #[arg(long)]
    root: PathBuf,
    #[arg(long, default_value_t = 18082)]
    port: u16,
    #[arg(long, value_enum, default_value_t=Mode::Useful)]
    mode: Mode,
    /// This endpoint exits after its finite request count or ten idle minutes.
    #[arg(long, default_value_t=1, value_parser=clap::value_parser!(u8).range(1..=8))]
    requests: u8,
}

fn draft(base: &Value, selection: &Value, mode: Mode) -> EvidenceResult<Value> {
    if matches!(mode, Mode::Malformed) {
        return Ok(json!({"schema_version":1,"draft":{}}));
    }
    let mut node = base["semantic"]["nodes"]["repair.build-1"].clone();
    let argv = &mut node["data_inputs"]["command"]["binding"]["value"]["argv"];
    ensure(
        argv.is_array(),
        "fixture baseline has no native worker command",
    )?;
    if matches!(mode, Mode::NoImprovement) {
        // A different command with the same seeded defect supplies a valid unhelpful candidate.
        *argv = json!([
            "/bin/cp",
            "-f",
            "/fixtures/slotbook-seeded",
            "/workspace/app"
        ]);
    } else {
        *argv = json!(["/bin/cp", "/fixtures/slotbook", "/workspace/app"]);
    }
    let mutation = if matches!(mode, Mode::ForbiddenEdit) {
        json!([{"type":"remove_node","node":"verify-1"}])
    } else {
        json!([{"type":"replace_node","node":node}])
    };
    let evidence = if matches!(mode, Mode::InventedEvidence) {
        json!([{"kind":"worker_observation","id":"invented-source"}])
    } else {
        json!(
            selection["artifacts"]
                .as_array()
                .ok_or("fixture source artifacts absent")?
                .iter()
                .map(|source| json!({"kind":"worker_observation","id":source["artifact"]}))
                .collect::<Vec<_>>()
        )
    };
    Ok(json!({"schema_version":1,"draft":{
        "identity":"learning-candidate","proposer":"agent:slotbook-learner","provenance":{"type":"direct"},
        "workflow":"slotbook-learning","run":null,"base_revision":base["id"],"base_digest":base["content_digest"],"observed_run_sequence":null,
        "mutation":mutation,"rationale":"Deterministic fixture hypothesis: applying the documented authenticated durable implementation before the first verifier avoids the seeded contradiction and repair.",
        "rationale_artifact":null,"risk_notes":["This controlled response is not a model inference. Any failed check, paired regression or insufficient repair reduction counts against the hypothesis."],
        "assumptions":["Applicability is the native Slotbook template with its existing operator obligations; all fixed verifier nodes, interfaces, account bounds and target policies remain unchanged."],
        "evidence":evidence,"artifacts":[],"application_policy":"propose_only","requested_action":null,"claimed_stop":"complete"
    }}))
}

pub(super) fn run(args: Arguments) -> EvidenceResult {
    let root = args.root.canonicalize()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, args.port))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(600);
    for index in 0..args.requests {
        let mut stream = loop {
            ensure(
                Instant::now() < deadline,
                "fixture endpoint idle bound reached",
            )?;
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => return Err(e.into()),
            }
        };
        let request = read_request(&mut stream)?;
        ensure(
            request.starts_with("POST /v1/chat/completions "),
            "fixture endpoint only accepts chat completions",
        )?;
        let dir = root.join("learning-05");
        let baseline = load(dir.join("governed.json"))?["revision"].clone();
        let selection = load(dir.join("learning-select.json"))?["selection"].clone();
        let proposal = draft(&baseline, &selection, args.mode)?;
        let document = serde_json::to_string(&proposal)?;
        if !matches!(args.mode, Mode::Malformed) {
            milkdrift_control::WorkflowProposalDocument::from_json(document.as_bytes())?;
        }
        let body = serde_json::to_string(
            &json!({"id":format!("slotbook-fixture-{index}"),"model":"learning-fixture","choices":[{"index":0,"message":{"role":"assistant","content":serde_json::to_string(&json!({"proposal_document_json":document}))?},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
        )?;
        let id = format!("fixture-{}-{index}", text(&baseline["id"])?);
        write_private(&dir.join(format!("{id}.request")), request.as_bytes())?;
        write_private(&dir.join(format!("{id}.response")), body.as_bytes())?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_responses_use_the_ordinary_closed_proposal_reader() -> EvidenceResult {
        let example: Value = serde_json::from_str(include_str!(
            "../../../../../examples/operator/process.json"
        ))?;
        let requirement =
            &example["revision"]["semantic"]["nodes"]["process"]["kind"]["config"]["requirement"];
        let node = crate::prepare::worker(
            "repair.build-1",
            requirement,
            &["/bin/cp", "/fixtures/slotbook-seeded", "/workspace/app"],
            false,
        );
        let mut base = example["revision"].clone();
        base["semantic"]["nodes"]["repair.build-1"] = node;
        let selection = json!({"artifacts":[{"artifact":"selected-source"}]});
        for mode in [
            Mode::Useful,
            Mode::NoImprovement,
            Mode::InventedEvidence,
            Mode::ForbiddenEdit,
        ] {
            let proposal = draft(&base, &selection, mode)?;
            let parsed = milkdrift_control::WorkflowProposalDocument::from_json(
                &serde_json::to_vec(&proposal)?,
            )?;
            assert_eq!(
                parsed.proposal().base_revision().as_str(),
                text(&base["id"])?
            );
        }
        assert!(
            milkdrift_control::WorkflowProposalDocument::from_json(&serde_json::to_vec(&draft(
                &base,
                &selection,
                Mode::Malformed
            )?)?)
            .is_err()
        );
        Ok(())
    }
}
