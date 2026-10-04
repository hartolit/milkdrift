use super::{
    Journey,
    setup::{sequence, text},
};
use milkdrift_evidence::{EvidenceResult, application::ensure};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

pub(super) fn author(j: &Journey) -> EvidenceResult<String> {
    j.call(&["workflow", "models"])?;
    let file = j.file("release-notes.draft.json")?;
    j.call(&[
        "workflow",
        "new",
        "release-notes",
        "--name",
        "Release notes",
        "--file",
        &file,
    ])?;
    j.call(&["workflow", "input", &file, "brief"])?;
    for step in ["draft", "review"] {
        j.call(&[
            "workflow",
            "add",
            &file,
            step,
            "--model",
            "operator-model",
            "--prompt",
            &j.file(&format!("{step}.txt"))?,
            "--maximum-output-units",
            "1024",
        ])?;
        j.call(&[
            "workflow",
            "connect",
            &file,
            step,
            "brief",
            "--run-input",
            "brief",
        ])?;
    }
    let before = fs::read(&file)?;
    j.refuse(&["workflow", "model", &file, "review", "unavailable-model"])?;
    ensure(
        fs::read(&file)? == before,
        "refused model edit changed the draft",
    )?;
    j.call(&[
        "workflow",
        "connect",
        &file,
        "review",
        "draft",
        "--from-step",
        "draft",
    ])?;
    j.call(&["workflow", "output", &file, "review", "--name", "notes"])?;
    j.call(&["workflow", "inspect", &file])?;
    let saved = j.call(&["workflow", "save", &file])?;
    let revision = text(&saved, "/value/revision_id")?;
    j.retain("saved-workflow.json", &saved)?;
    // Every CLI command is a separate client process; reopening never depends on client memory.
    let reopened = j.file("reopened.draft.json")?;
    j.call(&["workflow", "open", &revision, "--file", &reopened])?;
    ensure(
        text(
            &j.call(&["workflow", "save", &reopened])?,
            "/value/revision_id",
        )? == revision,
        "unchanged reopen changed the revision",
    )?;
    let meeting = j.file("meeting.draft.json")?;
    j.call(&[
        "workflow",
        "new",
        "meeting-summary",
        "--name",
        "Meeting summary",
        "--file",
        &meeting,
    ])?;
    j.call(&["workflow", "input", &meeting, "brief"])?;
    j.call(&[
        "workflow",
        "add",
        &meeting,
        "summarize",
        "--model",
        "operator-model",
        "--prompt",
        &j.file("meeting.txt")?,
        "--maximum-output-units",
        "256",
    ])?;
    j.call(&[
        "workflow",
        "connect",
        &meeting,
        "summarize",
        "brief",
        "--run-input",
        "brief",
    ])?;
    j.call(&[
        "workflow",
        "output",
        &meeting,
        "summarize",
        "--name",
        "summary",
    ])?;
    j.call(&["workflow", "save", &meeting])?;
    Ok(revision)
}

fn start(j: &Journey, run: &str, revision: &str, brief: &str) -> EvidenceResult {
    j.call(&[
        "--command-id",
        &format!("start-{run}"),
        "run",
        "start",
        run,
        "release-notes",
        revision,
        "--input",
        &format!("brief={}", j.file(brief)?),
        "--request-file",
        &j.file(&format!("{run}.request.json"))?,
    ])?;
    Ok(())
}

fn completed(j: &Journey, run: &str) -> EvidenceResult<Value> {
    let outcome = j.call(&[
        "--timeout-secs",
        "390",
        "run",
        "wait",
        run,
        "--terminal",
        "succeeded",
    ]);
    let result = j.call(&["run", "result", run, "--details"])?;
    j.retain(&format!("{run}.result.json"), &result)?;
    outcome?;
    j.call(&[
        "run",
        "result",
        run,
        "--field",
        "notes",
        "--output",
        &j.file(&format!("{run}.txt"))?,
    ])?;
    ensure(
        !fs::read_to_string(j.root.join(format!("{run}.txt")))?
            .trim()
            .is_empty(),
        "downloaded empty final output",
    )?;
    Ok(result)
}

pub(super) fn briefs(j: &Journey, revision: &str) -> EvidenceResult {
    for (run, brief) in [
        ("harbor", "harbor-brief.txt"),
        ("lantern", "lantern-brief.txt"),
    ] {
        start(j, run, revision, brief)?;
        completed(j, run)?;
        let current = j.call(&["run", "show", run])?;
        ensure(
            text(&current, "/value/revision_id")? == revision,
            "brief used another revision",
        )?;
    }
    ensure(
        fs::read(j.root.join("harbor.txt"))? != fs::read(j.root.join("lantern.txt"))?,
        "different briefs returned identical notes",
    )?;
    Ok(())
}

pub(super) fn replay_and_copy(j: &Journey, revision: &str) -> EvidenceResult {
    for run in ["harbor", "lantern"] {
        let response = j.call(&["run", "reconnect", &j.file(&format!("{run}.request.json"))?])?;
        ensure(
            response.pointer("/value/replayed").and_then(Value::as_bool) == Some(true),
            "restart did not replay the original request",
        )?;
        j.call(&[
            "run",
            "result",
            run,
            "--field",
            "notes",
            "--output",
            &j.file(&format!("{run}.replayed.txt"))?,
        ])?;
        ensure(
            fs::read(j.root.join(format!("{run}.txt")))?
                == fs::read(j.root.join(format!("{run}.replayed.txt")))?,
            "replay changed result bytes",
        )?;
    }
    let run_sequence = sequence(j, "harbor")?;
    j.refuse(&[
        "--expected-sequence",
        &run_sequence,
        "run",
        "resume",
        "harbor",
    ])?;
    let source = j.call(&["workflow", "show", revision])?;
    j.call(&[
        "workflow",
        "list",
        "--workflow",
        "release-notes",
        "--limit",
        "32",
    ])?;
    let copied = j.file("copied.draft.json")?;
    j.call(&[
        "workflow",
        "copy",
        revision,
        "independent-notes",
        "--name",
        "Independent notes",
        "--file",
        &copied,
    ])?;
    j.call(&["workflow", "rename", &copied, "Edited independent notes"])?;
    let changed = j.call(&["workflow", "save", &copied])?;
    ensure(
        text(&changed, "/value/revision_id")? != revision,
        "copy did not get an independent revision",
    )?;
    let reopened = j.file("reopened.draft.json")?;
    j.call(&[
        "workflow",
        "rename",
        &reopened,
        "Release notes revised title",
    ])?;
    ensure(
        text(
            &j.call(&["workflow", "save", &reopened])?,
            "/value/revision_id",
        )? != revision,
        "later saved edit did not create a child",
    )?;
    ensure(
        j.call(&["workflow", "show", revision])?.get("value") == source.get("value"),
        "later edit changed accepted definition",
    )?;
    for run in ["harbor", "lantern"] {
        ensure(
            text(&j.call(&["run", "show", run])?, "/value/revision_id")? == revision,
            "later edit changed an accepted run pin",
        )?;
    }
    Ok(())
}

pub(super) fn repair(j: &Journey, revision: &str) -> EvidenceResult {
    start(j, "repair-run", revision, "harbor-brief.txt")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let current = j.call(&["run", "show", "repair-run"])?;
        if current
            .pointer("/value/nodes")
            .and_then(Value::as_array)
            .is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node.get("node_id") == Some(&json!("author.review.hold")))
            })
        {
            break;
        }
        ensure(
            Instant::now() < deadline,
            "failed response never reached the supported review hold",
        )?;
        thread::sleep(Duration::from_millis(100));
    }
    let seq = sequence(j, "repair-run")?;
    j.call(&["--expected-sequence", &seq, "run", "pause", "repair-run"])?;
    let before = j.call(&["run", "timeline", "repair-run", "--limit", "1000"])?;
    let failed = j.call(&["run", "result", "repair-run", "--details"])?;
    j.retain("repair-failed.json", &failed)?;
    let proposal = j.file("repair.proposal.json")?;
    j.call(&[
        "proposal",
        "repair",
        "repair-run",
        "review",
        "--proposal",
        "repair-final",
        "--new-step",
        "repair",
        "--model",
        "operator-model",
        "--prompt",
        &j.file("review.txt")?,
        "--maximum-output-units",
        "1024",
        "--file",
        &proposal,
    ])?;
    j.refuse(&["--expected-sequence", "0", "proposal", "submit", &proposal])?;
    let submitted = j.call(&["proposal", "submit", &proposal])?;
    let next = text(&submitted, "/value/value/proposed_revision")?;
    let digest = text(&submitted, "/value/value/proposal_digest")?;
    j.refuse(&[
        "--yes",
        "proposal",
        "apply",
        "repair-run",
        "repair-final",
        &digest,
        &next,
    ])?;
    j.call(&[
        "--yes",
        "proposal",
        "approve",
        "repair-run",
        "repair-final",
        &digest,
        &next,
        "approve-final",
    ])?;
    j.call(&[
        "--yes",
        "proposal",
        "apply",
        "repair-run",
        "repair-final",
        &digest,
        &next,
    ])?;
    let seq = sequence(j, "repair-run")?;
    j.call(&[
        "--expected-sequence",
        &seq,
        "run",
        "signal",
        "repair-run",
        "--signal-id",
        "release-repair",
        "--signal-type",
        "workflow.reviewed",
    ])?;
    let seq = sequence(j, "repair-run")?;
    j.call(&["--expected-sequence", &seq, "run", "resume", "repair-run"])?;
    completed(j, "repair-run")?;
    let after = j.call(&["run", "timeline", "repair-run", "--limit", "1000"])?;
    let original = before
        .pointer("/value/items")
        .and_then(Value::as_array)
        .ok_or("original timeline absent")?;
    let retained = after
        .pointer("/value/items")
        .and_then(Value::as_array)
        .ok_or("repaired timeline absent")?;
    ensure(
        retained.get(..original.len()) == Some(original.as_slice()),
        "repair rewrote completed history",
    )?;
    j.retain("repair-history-before.json", &before)?;
    j.retain("repair-history-after.json", &after)?;
    Ok(())
}
