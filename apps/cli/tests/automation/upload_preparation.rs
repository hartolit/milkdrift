//! Preparation failures must leave bounded, usable evidence without submitting execution.
use super::*;
use std::{path::Path, sync::atomic::AtomicUsize};

fn arguments(
    invocation: bool,
    output: &Path,
    specifications: &[String],
) -> TestResult<Vec<String>> {
    let mut args = if invocation {
        [
            "invocation",
            "prepare",
            "test-capability",
            "test.execute",
            "--host",
            "host-test",
            "--request-id",
            "stable-request",
            "--output",
        ]
        .map(str::to_owned)
        .to_vec()
    } else {
        [
            "run",
            "start",
            "run-one",
            "workflow-one",
            "rev_0000000000000000000000000000000000000000000000000000000000000000",
            "--prepare-only",
            "--request-file",
        ]
        .map(str::to_owned)
        .to_vec()
    };
    args.push(output.to_str().ok_or("output path")?.into());
    args.extend(["--timeout-secs".into(), "2".into()]);
    for spec in specifications {
        args.extend(["--input".into(), spec.clone()]);
    }
    Ok(args)
}

fn authority() -> String {
    response(
        json!({"host":"host-test","actor":"human-test","grant_id":"grant-test",
        "grant_revision":1,"grant_digest":"digest","revocation_generation":0,"operations":[]}),
    )
}

fn artifact() -> String {
    response(
        json!({"artifact_id":"input:retained","digest":blake3::hash(b"first text").to_hex().to_string(),
        "size":10,"content_type":"text/plain","sensitivity":"restricted","disposition_name":null}),
    )
}

fn base_responses(invocation: bool) -> Vec<String> {
    let mut replies = vec![negotiation()];
    if !invocation {
        replies.push(authority());
    }
    replies
}

fn records_of<'a>(records: &'a [Value], kind: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| record.get("type").and_then(Value::as_str) == Some(kind))
        .collect()
}

#[test]
fn later_local_errors_refuse_before_any_upload_in_both_consumers() -> TestResult {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.txt");
    let invalid_utf8 = root.path().join("invalid.txt");
    let oversized = root.path().join("oversized.txt");
    std::fs::write(&first, b"first text")?;
    std::fs::write(&invalid_utf8, [0xff])?;
    std::fs::write(
        &oversized,
        vec![b'x'; milkdrift_control_protocol::MAX_INPUT_UPLOAD_BYTES + 1],
    )?;
    for invocation in [false, true] {
        let first_spec = format!("first={}", first.display());
        let mut cases = vec![
            vec![first_spec.clone(), "second=".into()],
            vec![first_spec.clone(), format!("first={}", first.display())],
            vec![first_spec.clone(), format!("bad/name={}", first.display())],
            vec![
                first_spec.clone(),
                format!("second={}", root.path().join("absent").display()),
            ],
            vec![
                first_spec.clone(),
                format!("second={}", root.path().display()),
            ],
            vec![
                first_spec.clone(),
                format!("second={}", invalid_utf8.display()),
            ],
            vec![
                first_spec.clone(),
                format!("second={}", oversized.display()),
            ],
            vec![first_spec, "second=-".into(), "third=-".into()],
        ];
        cases.push(
            (0..=256)
                .map(|index| format!("input{index}={}", first.display()))
                .collect(),
        );
        for specifications in cases {
            let requests = Arc::new(AtomicUsize::new(0));
            let observed = requests.clone();
            let server = Server::with_response_hook(vec![negotiation()], move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })?;
            let output = root.path().join("request.json");
            let args = arguments(invocation, &output, &specifications)?;
            let (exit, records, _) =
                server.invoke(&args.iter().map(String::as_str).collect::<Vec<_>>(), false)?;
            assert_eq!(exit, 2, "{specifications:?}: {records:?}");
            assert_eq!(
                requests.load(Ordering::SeqCst),
                1,
                "only negotiation is allowed"
            );
            assert_eq!(records.len(), 1);
            assert!(!output.exists());
        }
    }
    Ok(())
}

#[test]
fn changed_or_missing_later_files_preserve_the_first_upload_record() -> TestResult {
    for invocation in [false, true] {
        for remove in [false, true] {
            let root = tempfile::tempdir()?;
            let first = root.path().join("first.txt");
            let second = root.path().join("second.txt");
            std::fs::write(&first, b"first text")?;
            std::fs::write(&second, b"second text")?;
            let specs = [
                format!("first={}", first.display()),
                format!("second={}", second.display()),
            ];
            let first_upload = if invocation { 1 } else { 2 };
            let mut replies = base_responses(invocation);
            replies.push(artifact());
            let requests = Arc::new(AtomicUsize::new(0));
            let observed = requests.clone();
            let server = Server::with_response_hook(replies, move |index| {
                observed.fetch_add(1, Ordering::SeqCst);
                if index == first_upload {
                    if remove {
                        std::fs::remove_file(&second)?;
                    } else {
                        std::fs::write(&second, b"changed text")?;
                    }
                }
                Ok(())
            })?;
            let output = root.path().join("request.json");
            let args = arguments(invocation, &output, &specs)?;
            let (exit, records, _) =
                server.invoke(&args.iter().map(String::as_str).collect::<Vec<_>>(), false)?;
            assert_eq!(exit, 2, "{records:?}");
            assert_eq!(requests.load(Ordering::SeqCst), first_upload + 1);
            assert_retained(&records)?;
            assert!(!output.exists());
        }
    }
    Ok(())
}

fn assert_retained(records: &[Value]) -> TestResult {
    let pending = records_of(records, "input.uploading");
    let first = pending.first().ok_or("pending upload")?;
    assert!(
        first
            .pointer("/value/recovery")
            .and_then(Value::as_str)
            .ok_or("recovery")?
            .contains("original bytes")
    );
    assert_eq!(first.pointer("/final"), Some(&json!(false)));
    let completed = records_of(records, "input.uploaded");
    assert_eq!(completed.len(), 1);
    assert_eq!(
        completed
            .first()
            .and_then(|value| value.pointer("/value/artifact/artifact_id")),
        Some(&json!("input:retained"))
    );
    assert!(!serde_json::to_string(records)?.contains("first text"));
    Ok(())
}

#[test]
fn interrupted_upload_and_recovery_publication_keep_exact_progress() -> TestResult {
    for publication_conflict in [false, true] {
        let root = tempfile::tempdir()?;
        let first = root.path().join("first.txt");
        std::fs::write(&first, b"first text")?;
        let output = root.path().join("request.json");
        let destination = output.clone();
        let server = Server::with_response_hook(
            vec![negotiation(), authority(), artifact()],
            move |index| {
                if publication_conflict && index == 2 {
                    std::fs::write(&destination, b"competing writer")?;
                }
                Ok(())
            },
        )?;
        let mut specs = vec![format!("first={}", first.display())];
        if !publication_conflict {
            specs.push(format!("second={}", first.display()));
        }
        let args = arguments(false, &output, &specs)?;
        let (exit, records, _) =
            server.invoke(&args.iter().map(String::as_str).collect::<Vec<_>>(), false)?;
        assert_retained(&records)?;
        if publication_conflict {
            assert_eq!(exit, 9, "{records:?}");
            assert_eq!(std::fs::read(&output)?, b"competing writer");
        } else {
            assert_eq!(exit, 10, "{records:?}");
            assert_eq!(records_of(&records, "input.uploading").len(), 2);
            assert!(!output.exists());
        }
        assert!(
            !std::fs::read_dir(root.path())?.any(|entry| entry.is_ok_and(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".milkdrift-output-")))
        );
    }
    Ok(())
}
