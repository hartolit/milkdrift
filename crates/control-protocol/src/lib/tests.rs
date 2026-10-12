use super::*;

#[test]
fn capability_snapshots_have_one_closed_complete_set_wire_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../../tests/fixtures/capability-snapshot-v3.0.json").trim_end();
    let observation: Observation = decode_json(fixture.as_bytes())?;
    assert_eq!(String::from_utf8(encode_json(&observation)?)?, fixture);
    let Observation::CapabilitySnapshot(values) = observation else {
        return Err("wrong fixture variant".into());
    };
    assert_eq!(values.len(), 2);
    assert_eq!(values.first().ok_or("old absent")?.generation, 1);
    assert_eq!(values.get(1).ok_or("new absent")?.generation, 2);
    assert_eq!(
        encode_json(&Observation::CapabilitySnapshot(vec![]))?,
        br#"{"type":"capability_snapshot","value":[]}"#
    );
    for invalid in [
        fixture.replace("capability_snapshot", "capability"),
        fixture.replacen("\"type\":", "\"partial\":true,\"type\":", 1),
        fixture.replacen("\"generation\":1", "\"generation\":1,\"removed\":true", 1),
        "{\"type\":\"capability_snapshot\",\"value\":null}".into(),
    ] {
        assert!(decode_json::<Observation>(invalid.as_bytes()).is_err());
    }
    assert!(
        ProtocolVersion {
            major: 2,
            minor: 19
        }
        .negotiate()
        .is_err()
    );
    Ok(())
}

#[test]
fn response_admission_and_encoding_obey_the_normal_reader_structure_limits()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::json!("escaped\"\\\n text");
    for _ in 0..JSON_LIMITS.maximum_depth {
        value = serde_json::json!([value]);
    }
    // A raw value at the depth ceiling fits; adding a response envelope does not.
    let bytes = encode_json(&value)?;
    assert_eq!(decode_json::<Value>(&bytes)?, value);
    assert!(validate_response_capacity(&value).is_err());
    let too_deep = serde_json::json!([value]);
    assert!(decode_json::<Value>(&serde_json::to_vec(&too_deep)?).is_err());
    assert!(encode_json(&too_deep).is_err());
    Ok(())
}

#[test]
fn version_and_cursor_are_explicit_and_feed_bound() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        ProtocolVersion::CURRENT.negotiate()?,
        ProtocolVersion::CURRENT
    );
    assert!(matches!(
        ProtocolVersion { major: 1, minor: 0 }.negotiate(),
        Err(ProtocolError::UnsupportedMajor { .. })
    ));
    for minor in [PROTOCOL_MINOR + 1, u16::MAX] {
        assert!(matches!(
            ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor
            }
            .negotiate(),
            Err(ProtocolError::UnsupportedMinor { .. })
        ));
    }
    let cursor = Cursor::new("workflow:alpha", 42)?;
    assert_eq!(cursor.position_for("workflow:alpha")?, 42);
    assert!(cursor.position_for("workflow:beta").is_err());
    assert!(
        Cursor("not-base64!".to_owned())
            .position_for("workflow:alpha")
            .is_err()
    );
    Ok(())
}

#[test]
fn bound_cursor_rejects_other_actor_grant_scope_and_rotated_credential()
-> Result<(), Box<dyn std::error::Error>> {
    let binding = CursorBinding {
        actor: "human:alice".to_owned(),
        grant_id: "grant:alice".to_owned(),
        grant_revision: 3,
        grant_digest: format!("b3_{}", "1".repeat(64)),
        scope_digest: format!("b3_{}", "2".repeat(64)),
    };
    let key = [7_u8; 32];
    let cursor = Cursor::new_bound(
        "workflows:active:workflow-a",
        42,
        binding.clone(),
        &format!("b3_{}", "3".repeat(64)),
        &key,
    )?;
    assert_eq!(
        cursor.position_for_bound("workflows:active:workflow-a", &binding, &key)?,
        42
    );
    let mut other_actor = binding.clone();
    other_actor.actor = "ai:alice".to_owned();
    assert!(
        cursor
            .position_for_bound("workflows:active:workflow-a", &other_actor, &key)
            .is_err()
    );
    let mut narrower_scope = binding.clone();
    narrower_scope.scope_digest = format!("b3_{}", "4".repeat(64));
    assert!(
        cursor
            .position_for_bound("workflows:active:workflow-a", &narrower_scope, &key)
            .is_err()
    );
    assert!(
        cursor
            .position_for_bound("workflows:active:workflow-a", &binding, &[8_u8; 32])
            .is_err()
    );
    Ok(())
}

#[test]
fn duplicate_json_keys_and_unbounded_pages_are_rejected() {
    assert!(
        decode_json::<VersionRequest>(br#"{"protocol":{"major":1,"major":1,"minor":0}}"#).is_err()
    );
    assert!(
        PageRequest {
            cursor: None,
            limit: MAX_PAGE_ITEMS + 1
        }
        .validate()
        .is_err()
    );
}
