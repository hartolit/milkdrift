//! Selected source and failure evidence for the next ordinary model proposal.
use super::{Session, Value, ensure, fs, json, text};
use crate::{client::Caller, qualification};
use milkdrift_evidence::EvidenceResult;
use milkdrift_peer_protocol::{ObservationHistory, ObservationPage};
use std::{io::Read as _, path::Path, thread, time::Duration};

// The configured worker captures up to 4 MiB; JSON escaping can expand those bytes
// sixfold. This separate read allowance does not enlarge the selected model context.
const WORKER_REPORT_BYTES: u64 = 32 * 1024 * 1024;
const SELECTED_REPORT_BYTES: u64 = 65536;

pub(super) fn read_bounded_file(path: &Path, limit: u64) -> EvidenceResult<Vec<u8>> {
    // Check before opening: opening a FIFO itself can wait for an unrelated writer.
    ensure(
        fs::metadata(path)?.is_file(),
        "selected input is not a regular file",
    )?;
    let file = fs::File::open(path)?;
    ensure(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "selected input exceeds its read allowance or is not a regular file",
    )?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure(
        bytes.len() as u64 <= limit,
        "selected input grew beyond its read allowance",
    )?;
    Ok(bytes)
}

pub(super) fn read_artifact(
    path: &Path,
    size: u64,
    digest: &str,
    limit: u64,
) -> EvidenceResult<Vec<u8>> {
    ensure(size <= limit, "artifact exceeds its read allowance")?;
    let bytes = read_bounded_file(path, size)?;
    ensure(
        bytes.len() as u64 == size && blake3::hash(&bytes).to_hex().as_str() == digest,
        "downloaded artifact differs from its immutable reference",
    )?;
    Ok(bytes)
}

pub(super) fn source_snapshot(s: &mut Session, index: u8) -> EvidenceResult<Value> {
    let name = format!("source-{index}-snapshot");
    let input = s.root.join(format!("{name}-inputs.json"));
    if !input.exists() {
        s.write(&format!("{name}-inputs.json"), &json!([{"name":"command","value":{"type":"inline","value":{"argv":["/bin/sh","-c","test -f /workspace/source/slotbook.rs && cat /workspace/source/slotbook.rs"],"stdout_artifact":true}}}]))?;
    }
    let execution = s.invoke(
        &name,
        "managed.slotbook-build.worker",
        "workspace.execute",
        &input,
        Caller::Operator,
    )?;
    let observed = s.ok(
        &format!("{name}-wait"),
        args!["invocation", "wait", execution],
    )?;
    let artifact = snapshot_artifact(serde_json::from_value(observed["value"].clone())?)?;
    let size = super::number(&artifact["size_bytes"])?;
    ensure(size <= 32768, "source exceeds the bounded repair context")?;
    let path = s.root.join(format!("{name}.rs"));
    if !path.exists() {
        s.ok(
            &format!("{name}-download"),
            args![
                "artifact",
                "get",
                text(&artifact["identity"])?,
                "--output",
                path.display()
            ],
        )?;
    }
    let source = String::from_utf8(read_artifact(
        &path,
        size,
        text(&artifact["digest"])?,
        32768,
    )?)?;
    Ok(
        json!({"artifact":artifact,"digest":format!("b3_{}",blake3::hash(source.as_bytes())),"text":source}),
    )
}

fn snapshot_artifact(page: ObservationPage) -> EvidenceResult<Value> {
    let (observations, terminal) = match &page.history {
        ObservationHistory::Hot => (&page.observations, page.observations.last()),
        ObservationHistory::Archived { summary } => (
            &summary.output_observations,
            summary.final_observation.as_ref(),
        ),
    };
    ensure(
        page.closed
            && terminal
                .and_then(|o| o.event.kind().terminal())
                .is_some_and(|t| t.status() == milkdrift_capability::TerminalStatus::Success),
        "source snapshot did not finish successfully; inspect before another proposal",
    )?;
    let (_, artifact) = observations
        .iter()
        .filter_map(|o| o.event.kind().output())
        .find(|(name, _)| *name == "stdout")
        .ok_or("source snapshot absent; inspect worker before another proposal")?;
    Ok(serde_json::to_value(artifact)?)
}

pub(super) fn observe(s: &Session, run: &str, index: u8) -> EvidenceResult<Value> {
    let mut state = Value::Null;
    for poll in 0..=240 {
        state = s.ok(
            &format!("source-{index}-observe-{poll}"),
            args!["run", "show", run],
        )?["value"]
            .clone();
        if !state["terminal"].is_null() || state["uncertainty_count"] != 0 {
            break;
        }
        ensure(
            poll < 240,
            "source execution exhausted its 241 bounded observation requests",
        )?;
        thread::sleep(Duration::from_secs(5));
    }
    ensure(
        state["uncertainty_count"] == 0,
        "source execution uncertain; inspect before any further effect",
    )?;
    let mut attempts = Vec::new();
    for node in state["nodes"].as_array().ok_or("source nodes absent")? {
        if !node["latest_attempt_id"].is_string() {
            continue;
        }
        let id = text(&node["latest_attempt_id"])?;
        let attempt = s.ok(
            &format!("source-{index}-attempt-{id}"),
            args!["attempt", "inspect", run, id],
        )?["value"]
            .clone();
        let mut outputs = Vec::new();
        for output in attempt["outputs"]
            .as_array()
            .ok_or("attempt outputs absent")?
        {
            // Worker reports and verifier reports are bounded selected diagnostics. Native
            // executable stdout is retained by its artifact identity, never fed as model text.
            if (output["name"] == "worker_result" && node["node_id"] == "repair.begin")
                || output["name"] == "resource_result"
            {
                let worker = output["name"] == "worker_result";
                let limit = if worker {
                    WORKER_REPORT_BYTES
                } else {
                    SELECTED_REPORT_BYTES
                };
                ensure(
                    output["artifact"]["size"]
                        .as_u64()
                        .is_some_and(|size| size <= limit),
                    "diagnostic artifact exceeds its read allowance",
                )?;
                let path = s.logs.join(format!(
                    "source-{index}-{id}-{}.json",
                    text(&output["name"])?
                ));
                s.ok(
                    &format!("source-{index}-{id}-download-{}", text(&output["name"])?),
                    args![
                        "artifact",
                        "get",
                        text(&output["artifact"]["artifact_id"])?,
                        "--output",
                        path.display()
                    ],
                )?;
                let content = selected_report(&path, worker)?;
                let evaluation = if output["name"] == "resource_result"
                    && content["evaluation"]["identity"].is_string()
                {
                    qualification::evidence(
                        s,
                        &format!("source-{index}-{id}-completed-evaluation"),
                        text(&content["evaluation"]["identity"])?,
                    )?
                } else {
                    Value::Null
                };
                outputs.push(json!({"reference":output["artifact"],"content":content,"completed_evaluation":evaluation}));
            }
        }
        attempts.push(json!({"attempt_id":id,"node":node["node_id"],"state":attempt["state"],"terminal":attempt["terminal"],"terminal_detail":attempt["terminal_detail"],"uncertain":attempt["uncertain"],"selected_outputs":outputs}));
    }
    Ok(json!({"run":state,"attempts":attempts}))
}

fn selected_report(path: &Path, worker: bool) -> EvidenceResult<Value> {
    let limit = if worker {
        WORKER_REPORT_BYTES
    } else {
        SELECTED_REPORT_BYTES
    };
    let bytes = read_bounded_file(path, limit)?;
    let mut content: Value = serde_json::from_slice(&bytes)?;
    if worker {
        // Keep the full artifact; disclose every omitted byte in the selected excerpts.
        content = json!({"exit_code":content["exit_code"],"stderr":excerpt(text(&content["stderr"])?,12288),"stdout":excerpt(text(&content["stdout"])?,1024)});
    }
    ensure(
        serde_json::to_vec(&content)?.len() as u64 <= SELECTED_REPORT_BYTES,
        "selected diagnostics exceed model context allocation",
    )?;
    Ok(content)
}

fn excerpt(value: &str, limit: usize) -> Value {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    json!({"text":&value[..end],"omitted_bytes":value.len()-end,"selection":"leading UTF-8 excerpt; exact complete bytes remain in the referenced artifact"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_artifact_requires_exact_bytes_and_a_bounded_regular_file() -> EvidenceResult {
        let root = tempfile::tempdir()?;
        let path = root.path().join("response.json");
        let bytes = br#"{"source":"original"}"#;
        let digest = blake3::hash(bytes).to_hex().to_string();
        fs::write(&path, bytes)?;
        assert_eq!(
            read_artifact(&path, bytes.len() as u64, &digest, 1024)?,
            bytes
        );
        fs::write(&path, br#"{"source":"modified"}"#)?;
        assert!(read_artifact(&path, bytes.len() as u64, &digest, 1024).is_err());
        fs::write(&path, bytes)?;
        assert!(read_artifact(&path, bytes.len() as u64 - 1, &digest, 1024).is_err());
        assert!(read_artifact(&path, bytes.len() as u64, &digest, 1).is_err());
        assert!(read_bounded_file(root.path(), 1024).is_err());
        #[cfg(unix)]
        assert!(read_bounded_file(Path::new("/dev/zero"), 1024).is_err());
        Ok(())
    }

    #[test]
    fn snapshot_reads_hot_and_archived_outputs_and_refuses_incomplete_calls() -> EvidenceResult {
        let artifact = json!({"identity":"source:one","digest":"a".repeat(64),"media_type":"application/octet-stream","size_bytes":3});
        let output = json!({"category":"artifact","event":{"invocation":"snapshot:one","kind":{"name":"stdout","reference":artifact,"type":"output"},"sequence":1},"execution":"exec:one","observed_at_unix_ms":1,"sequence":1});
        let terminal = json!({"category":"terminal","event":{"invocation":"snapshot:one","kind":{"terminal":{"failure":null,"outputs":[],"side_effect":"none","status":"success","usage":null},"type":"terminal"},"sequence":2},"execution":"exec:one","observed_at_unix_ms":2,"sequence":2});
        let mut page = json!({"after_sequence":0,"closed":true,"execution":"exec:one","history":{"type":"hot"},"next_sequence":2,"observations":[output,terminal],"status":"terminal","terminal":true});
        assert_eq!(
            snapshot_artifact(serde_json::from_value(page.clone())?)?,
            artifact
        );
        page["closed"] = json!(false);
        assert!(snapshot_artifact(serde_json::from_value(page.clone())?).is_err());
        page["closed"] = json!(true);
        page["observations"] = json!([]);
        page["next_sequence"] = json!(0);
        page["history"] = json!({"type":"archived","summary":{"output_observations":[output],"status":"terminal","last_sequence":2,"observation_digest":format!("b3_{}","b".repeat(64)),"archived_at_unix_ms":3,"final_observation":terminal,"uncertainty_reason":null}});
        assert_eq!(
            snapshot_artifact(serde_json::from_value(page.clone())?)?,
            artifact
        );
        page["history"]["summary"]["final_observation"] = Value::Null;
        assert!(snapshot_artifact(serde_json::from_value(page)?).is_err());
        Ok(())
    }

    #[test]
    fn diagnostic_excerpt_discloses_omission_at_utf8_boundary() {
        let selected = excerpt("abéerror", 3);
        assert_eq!(selected["text"], "ab");
        assert_eq!(selected["omitted_bytes"], 7);
        assert_eq!(excerpt("short", 4096)["omitted_bytes"], 0);
    }

    #[test]
    fn large_worker_report_keeps_bounded_excerpts_and_refuses_oversized_artifacts() -> EvidenceResult
    {
        let root = tempfile::tempdir()?;
        let path = root.path().join("report.json");
        let stderr = "é: compiler error\n".repeat(8192);
        let bytes = serde_json::to_vec(&json!({"exit_code":1,"stderr":stderr,"stdout":"build"}))?;
        assert!(bytes.len() > usize::try_from(SELECTED_REPORT_BYTES)?);
        fs::write(&path, &bytes)?;
        let selected = selected_report(&path, true)?;
        let excerpt = text(&selected["stderr"]["text"])?;
        assert!(stderr.starts_with(excerpt));
        assert!(excerpt.len() <= 12288);
        assert_eq!(
            selected["stderr"]["omitted_bytes"],
            stderr.len() - excerpt.len()
        );
        assert_eq!(selected["stdout"]["text"], "build");
        assert_eq!(selected["exit_code"], 1);
        assert_eq!(fs::read(&path)?, bytes);
        assert!(selected_report(&path, false).is_err());
        fs::File::create(&path)?.set_len(WORKER_REPORT_BYTES + 1)?;
        assert!(selected_report(&path, true).is_err());
        Ok(())
    }
}
