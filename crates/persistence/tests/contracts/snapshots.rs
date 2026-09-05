use super::*;

#[test]
fn snapshot_checksum_and_history_prefix_are_verified() -> Result<(), Box<dyn std::error::Error>> {
    let event = sample_event(1)?;
    let history = milkdrift_persistence::history_digest(std::slice::from_ref(&event))?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-001")?,
        event.run_id().clone(),
        RunSequence::FIRST,
        history,
        1,
        br#"{"projection":"created"}"#.to_vec(),
    )?;
    let encoded = snapshot.to_canonical_json()?;
    assert_eq!(SnapshotDocument::from_json(&encoded)?, snapshot);
    assert_eq!(
        snapshot.envelope_schema_version(),
        SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2
    );
    let mut tampered_value: serde_json::Value = serde_json::from_slice(&encoded)?;
    tampered_value["run"] = json!("run-tampered");
    let tampered = serde_json::to_vec(&tampered_value)?;
    assert!(matches!(
        SnapshotDocument::from_json(&tampered),
        Err(PersistenceError::Corruption(_))
    ));
    let encoded_payload = tampered_value["encoded_payload"]
        .as_str()
        .ok_or("snapshot encoded payload is not a string")?
        .to_owned();
    let replacement = if encoded_payload.starts_with('A') {
        "B"
    } else {
        "A"
    };
    tampered_value["run"] = json!(event.run_id());
    tampered_value["encoded_payload"] = json!(format!(
        "{replacement}{}",
        encoded_payload.get(1..).ok_or("encoded payload is empty")?
    ));
    assert!(matches!(
        SnapshotDocument::from_json(&serde_json::to_vec(&tampered_value)?),
        Err(PersistenceError::Corruption(_))
    ));

    let duplicate = String::from_utf8(encoded)?.replacen(
        "{",
        &format!("{{\"schema_version\":{SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2},"),
        1,
    );
    assert!(matches!(
        SnapshotDocument::from_json(duplicate.as_bytes()),
        Err(PersistenceError::Json(_))
    ));
    assert!(milkdrift_persistence::history_digest(&[sample_event(2)?]).is_err());
    Ok(())
}

#[test]
fn snapshot_payload_larger_than_generic_json_array_limit_roundtrips_canonically()
-> Result<(), Box<dyn std::error::Error>> {
    let payload = (0..16_384_u32)
        .map(|index| u8::try_from(index % 251))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-large-payload")?,
        RunId::new("run-large-snapshot-payload")?,
        RunSequence::FIRST,
        IntegrityDigest::new(format!("b3_{}", "1".repeat(64)))?,
        3,
        payload,
    )?;
    let encoded = snapshot.to_canonical_json()?;
    assert_eq!(SnapshotDocument::from_json(&encoded)?, snapshot);
    assert_eq!(snapshot.to_canonical_json()?, encoded);
    let value: serde_json::Value = serde_json::from_slice(&encoded)?;
    assert!(value["encoded_payload"].is_string());
    assert!(value.get("payload").is_none());
    assert!(value["encoded_payload"].as_array().is_none());
    Ok(())
}

#[test]
fn snapshot_base64_and_closed_wire_are_strict() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-strict-wire")?,
        RunId::new("run-strict-wire")?,
        RunSequence::FIRST,
        IntegrityDigest::new(format!("b3_{}", "2".repeat(64)))?,
        3,
        b"projection".to_vec(),
    )?;
    let mut value: serde_json::Value = serde_json::from_slice(&snapshot.to_canonical_json()?)?;
    value["encoded_payload"] = json!("cHJvamVjdGlvbg");
    assert!(matches!(
        SnapshotDocument::from_json(&serde_json::to_vec(&value)?),
        Err(PersistenceError::InvalidDocument(_))
    ));
    value["encoded_payload"] = json!("cHJvamVjdGlvbg==trailing");
    assert!(matches!(
        SnapshotDocument::from_json(&serde_json::to_vec(&value)?),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let mut unknown: serde_json::Value = serde_json::from_slice(&snapshot.to_canonical_json()?)?;
    unknown["future_field"] = json!(true);
    assert!(matches!(
        SnapshotDocument::from_json(&serde_json::to_vec(&unknown)?),
        Err(PersistenceError::Json(_))
    ));
    Ok(())
}

#[test]
fn snapshot_bounds_apply_before_document_construction() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-bounds")?,
        RunId::new("run-bounds")?,
        RunSequence::FIRST,
        IntegrityDigest::new(format!("b3_{}", "3".repeat(64)))?,
        3,
        b"projection".to_vec(),
    )?;
    let mut value: serde_json::Value = serde_json::from_slice(&snapshot.to_canonical_json()?)?;
    value["encoded_payload"] = json!(STANDARD.encode(vec![0_u8; MAX_SNAPSHOT_PAYLOAD_BYTES + 1]));
    let decoded_over_bound = serde_json::to_vec(&value)?;
    assert!(decoded_over_bound.len() <= MAX_SNAPSHOT_DOCUMENT_BYTES);
    assert!(matches!(
        SnapshotDocument::from_json(&decoded_over_bound),
        Err(PersistenceError::Bounds {
            location: "snapshot.payload",
            ..
        })
    ));

    let oversized_document = vec![b' '; MAX_SNAPSHOT_DOCUMENT_BYTES + 1];
    assert!(matches!(
        SnapshotDocument::from_json(&oversized_document),
        Err(PersistenceError::Bounds {
            location: "snapshot.document",
            ..
        })
    ));
    Ok(())
}

#[test]
fn snapshot_v1_envelope_is_an_unsupported_optional_optimization() {
    let v1 = br#"{"schema_version":1,"payload":[1]}"#;
    assert!(matches!(
        SnapshotDocument::from_json(v1),
        Err(PersistenceError::UnsupportedVersion {
            document: "snapshot_envelope",
            found: 1,
            supported: SNAPSHOT_ENVELOPE_SCHEMA_VERSION_V2,
        })
    ));
}

#[test]
fn maximum_snapshot_encoding_has_formula_bounded_amplification()
-> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES,
        MAX_SNAPSHOT_PAYLOAD_BYTES.div_ceil(3) * 4
    );
    assert_eq!(
        MAX_SNAPSHOT_DOCUMENT_BYTES,
        MAX_SNAPSHOT_ENCODED_PAYLOAD_BYTES + 1_024
    );
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-maximum")?,
        RunId::new("run-maximum")?,
        RunSequence::FIRST,
        IntegrityDigest::new(format!("b3_{}", "4".repeat(64)))?,
        3,
        vec![0_u8; MAX_SNAPSHOT_PAYLOAD_BYTES],
    )?;
    let encoded = snapshot.to_canonical_json()?;
    assert!(encoded.len() <= MAX_SNAPSHOT_DOCUMENT_BYTES);
    assert_eq!(SnapshotDocument::from_json(&encoded)?, snapshot);
    Ok(())
}
