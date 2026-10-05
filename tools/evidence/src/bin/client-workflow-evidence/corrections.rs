//! Combined recovery, visibility and safe-export journey through actual application binaries.
use super::{Journey, setup::text};
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, write_private},
};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(super) fn replace_model(path: &Path) -> EvidenceResult<std::path::PathBuf> {
    let config: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
    let mut config = serde_json::to_value(config)?;
    *config
        .pointer_mut("/adapters/model_profiles/0/capability_id")
        .ok_or("model absent")? = json!("replacement-model");
    // The owned daemon is stopped. Keep its store and every actor/grant unchanged.
    write_private(
        &path.with_file_name("replacement-daemon.toml"),
        toml::to_string_pretty(&config)?.as_bytes(),
    )
}

fn unresolved(value: &Value, expected: usize) -> EvidenceResult {
    ensure(
        value
            .pointer("/value/workflow/selection_diagnostics")
            .and_then(Value::as_array)
            .is_some_and(|items| items.len() == expected),
        "unexpected retained-selection diagnostics",
    )
}

pub(super) fn journey(j: &Journey, original: &str) -> EvidenceResult {
    let history = j.call(&["run", "timeline", "harbor", "--limit", "1000"])?;
    let source = j.call(&["workflow", "show", original])?;
    let draft = j.file("unavailable.draft.json")?;
    let opened = j.call(&["workflow", "open", original, "--file", &draft])?;
    unresolved(&opened, 2)?;
    j.retain("unavailable-opened.json", &opened)?;
    j.call(&["workflow", "rename", &draft, "Repaired release notes"])?;
    unresolved(
        &j.call(&["workflow", "model", &draft, "draft", "replacement-model"])?,
        1,
    )?;
    let partial = j.call(&["workflow", "save", &draft])?;
    unresolved(&partial, 1)?;
    j.retain("unavailable-partial.json", &partial)?;
    unresolved(
        &j.call(&["workflow", "model", &draft, "review", "replacement-model"])?,
        0,
    )?;
    let repaired = j.call(&["workflow", "save", &draft])?;
    unresolved(&repaired, 0)?;
    let revision = text(&repaired, "/value/revision_id")?;
    ensure(
        revision != original,
        "repair failed to create an immutable child",
    )?;

    let run = "team/repaired";
    let request_file = j.file("unavailable.request.json")?;
    j.call(&[
        "--command-id",
        "combined-start",
        "run",
        "start",
        run,
        "release-notes",
        &revision,
        "--input",
        &format!("brief={}", j.file("harbor-brief.txt")?),
        "--request-file",
        &request_file,
        "--prepare-only",
    ])?;
    let saved = fs::read(&request_file)?;
    let accepted = j.call(&["run", "reconnect", &request_file])?;
    ensure(
        accepted.pointer("/value/replayed") == Some(&json!(false)),
        "new saved start was already replayed",
    )?;
    j.call(&[
        "--timeout-secs",
        "60",
        "run",
        "wait",
        run,
        "--terminal",
        "succeeded",
    ])?;
    let replay = j.call(&["run", "reconnect", &request_file])?;
    ensure(
        replay.pointer("/value/replayed") == Some(&json!(true))
            && fs::read(&request_file)? == saved,
        "recovery changed the saved request or created another start",
    )?;
    let result = j.call(&["run", "result", run, "--details"])?;
    let artifact = text(&result, "/value/outputs/0/artifact/artifact_id")?;
    let digest = text(&result, "/value/outputs/0/artifact/digest")?;
    j.retain("combined-authorized-result.json", &result)?;
    let observed = j.cli.run_with_token(
        &j.root.join("reader.token"),
        &["run", "result", run, "--details"],
        None,
    )?;
    ensure(
        observed.status.success(),
        "restricted reader lost authorized run inspection",
    )?;
    let restricted = observed.final_json()?;
    ensure(
        restricted.pointer("/value/run/terminal") == Some(&json!("succeeded"))
            && restricted.pointer("/value/outputs_restricted") == Some(&json!(true))
            && restricted
                .pointer("/value/outputs")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            && !observed.stdout.contains(&artifact)
            && !observed.stdout.contains(&digest),
        "restricted result exposed output metadata",
    )?;
    j.retain("combined-restricted-result.json", &restricted)?;
    let denied = j.cli.run_with_token(
        &j.root.join("reader.token"),
        &["artifact", "metadata", &artifact],
        None,
    )?;
    ensure(
        !denied.status.success(),
        "restricted reader could inspect protected metadata",
    )?;

    let output = j.file("combined-result.txt")?;
    j.call(&[
        "run", "result", run, "--field", "notes", "--output", &output,
    ])?;
    ensure(
        blake3::hash(&fs::read(&output)?).to_hex().as_str() == digest,
        "exported bytes contradict retained digest",
    )?;
    let competitor = j.file("combined-existing.txt")?;
    write_private(Path::new(&competitor), b"another writer's result")?;
    j.refuse(&[
        "run",
        "result",
        run,
        "--field",
        "notes",
        "--output",
        &competitor,
    ])?;
    ensure(
        fs::read(&competitor)? == b"another writer's result",
        "export overwrote a competing destination",
    )?;
    ensure(
        j.call(&["workflow", "show", original])?.get("value") == source.get("value"),
        "repair or execution rewrote the original revision",
    )?;
    let after = j.call(&["run", "timeline", "harbor", "--limit", "1000"])?;
    j.retain("combined-history-before.json", &history)?;
    j.retain("combined-history-after.json", &after)?;
    // Page cursors include the current read's authority decision; the ordered items are history.
    let original_items = history
        .pointer("/value/items")
        .and_then(Value::as_array)
        .ok_or("original history absent")?;
    let retained_items = after
        .pointer("/value/items")
        .and_then(Value::as_array)
        .ok_or("retained history absent")?;
    ensure(
        !original_items.is_empty() && retained_items == original_items,
        "repair or execution rewrote completed history",
    )?;
    j.retain("combined-corrections.json", &json!({"passed":true,"original_revision":original,
        "repaired_revision":revision,"run":run,"request_file":request_file,
        "checks":["unavailable-open","partial-save","replacement-model","input-upload","exact-replay",
            "slash-run","permission-filtered-result","verified-export","no-clobber","immutable-history"]}))?;
    Ok(())
}
