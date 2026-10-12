//! Preparation failures must leave bounded, usable evidence without submitting execution.
use super::*;
use std::{path::Path, sync::atomic::AtomicUsize};

fn arguments(output: &Path, specifications: &[String]) -> TestResult<Vec<String>> {
    let mut args = [
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
    .to_vec();
    args.push(output.to_str().ok_or("output path")?.into());
    args.extend(["--timeout-secs".into(), "2".into()]);
    for spec in specifications {
        args.extend(["--input".into(), spec.clone()]);
    }
    Ok(args)
}

fn artifact() -> String {
    response(
        json!({"artifact_id":"input:retained","digest":blake3::hash(b"first text").to_hex().to_string(),
        "size":10,"content_type":"text/plain","sensitivity":"restricted","disposition_name":null}),
    )
}

fn prepared_invocation() -> TestResult<String> {
    use milkdrift_capability::{
        AdmissionConstraints, ArtifactReference, BoundedJson, CancellationBehavior,
        CapabilityCategory, CapabilityId, DescriptorBuilder, IdempotencyBehavior, InputReference,
        InvocationId, InvocationRequest, InvocationValueReference, Locality, OperationContract,
        OperationId, PeerId, ResolvedCapabilitySnapshot, SchemaContract, SchemaId, SideEffectClass,
        StreamingMode,
    };
    use milkdrift_peer_protocol::{
        CatalogDigest, DirectInvocationRequest, ExecutionLimits, PeerRequestId,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let capability = CapabilityId::new("test-capability")?;
    let operation = OperationId::new("test.execute")?;
    let schema = SchemaContract::new(
        SchemaId::new("test.input")?,
        1,
        BoundedJson::new(json!({"type":"object"}))?,
    )?;
    let descriptor = DescriptorBuilder::new(
        capability.clone(),
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(1, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        operation.clone(),
        OperationContract::new(
            schema.clone(),
            schema,
            BTreeSet::from([StreamingMode::None]),
            CancellationBehavior::Unsupported,
            IdempotencyBehavior::Unsupported,
            SideEffectClass::ReadOnly,
            BTreeMap::new(),
        )?,
    )]))
    .build()?;
    let request = DirectInvocationRequest {
        host: PeerId::new("host-test")?,
        request_id: PeerRequestId::new("stable-request")?,
        catalog_generation: 1,
        catalog_digest: CatalogDigest::new(format!("b3_{}", "1".repeat(64)))?,
        selection: ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?,
        request: InvocationRequest::new(
            InvocationId::new("stable-request")?,
            capability,
            operation,
            None,
            None,
            vec![InputReference::new(
                "first",
                InvocationValueReference::Artifact {
                    reference: ArtifactReference::new(
                        "input:retained",
                        blake3::hash(b"first text").to_hex().to_string(),
                        Some("text/plain".into()),
                        Some(10),
                    )?,
                },
            )?],
            BTreeMap::new(),
        )?,
        deadline_unix_ms: 12345,
        limits: ExecutionLimits {
            nested_invocations: None,
            artifact_bytes: 4096,
            duration_ms: 1000,
            cost_micros: 0,
            cost_currency: None,
            input_units: None,
            output_units: None,
            observations: 16,
        },
    };
    Ok(response(serde_json::to_value(request)?))
}

fn records_of<'a>(records: &'a [Value], kind: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| record.get("type").and_then(Value::as_str) == Some(kind))
        .collect()
}

#[test]
fn later_local_errors_refuse_before_any_invocation_upload() -> TestResult {
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
    {
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
            let args = arguments(&output, &specifications)?;
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
    {
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
            let first_upload = 1;
            let mut replies = vec![negotiation()];
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
            let args = arguments(&output, &specs)?;
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
        let mut responses = vec![negotiation(), artifact()];
        if publication_conflict {
            responses.push(prepared_invocation()?);
        }
        let server = Server::with_response_hook(responses, move |index| {
            if publication_conflict && index == 2 {
                std::fs::write(&destination, b"competing writer")?;
            }
            Ok(())
        })?;
        let mut specs = vec![format!("first={}", first.display())];
        if !publication_conflict {
            specs.push(format!("second={}", first.display()));
        }
        let args = arguments(&output, &specs)?;
        let (exit, records, _) =
            server.invoke(&args.iter().map(String::as_str).collect::<Vec<_>>(), false)?;
        assert_retained(&records)?;
        if publication_conflict {
            assert_eq!(exit, 2, "{records:?}");
            assert_eq!(
                records
                    .last()
                    .and_then(|record| record.pointer("/error/classification")),
                Some(&json!("invalid_input"))
            );
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
