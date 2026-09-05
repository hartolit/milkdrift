use super::*;

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
    let cursor = Cursor::new("run:alpha", 42)?;
    assert_eq!(cursor.position_for("run:alpha")?, 42);
    assert!(cursor.position_for("run:beta").is_err());
    assert!(
        Cursor("not-base64!".to_owned())
            .position_for("run:alpha")
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
        "runs:active:workflow-a",
        42,
        binding.clone(),
        &format!("b3_{}", "3".repeat(64)),
        &key,
    )?;
    assert_eq!(
        cursor.position_for_bound("runs:active:workflow-a", &binding, &key)?,
        42
    );
    let mut other_actor = binding.clone();
    other_actor.actor = "ai:alice".to_owned();
    assert!(
        cursor
            .position_for_bound("runs:active:workflow-a", &other_actor, &key)
            .is_err()
    );
    let mut narrower_scope = binding.clone();
    narrower_scope.scope_digest = format!("b3_{}", "4".repeat(64));
    assert!(
        cursor
            .position_for_bound("runs:active:workflow-a", &narrower_scope, &key)
            .is_err()
    );
    assert!(
        cursor
            .position_for_bound("runs:active:workflow-a", &binding, &[8_u8; 32])
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

#[test]
fn layout_digest_is_independent_and_tamper_evident() -> Result<(), Box<dyn std::error::Error>> {
    let layout = LayoutDocument {
        schema_version: LAYOUT_SCHEMA_VERSION,
        workflow_id: "workflow-a".to_owned(),
        revision_id: "revision-a".to_owned(),
        generation: 1,
        author: "human:operator".to_owned(),
        digest: String::new(),
        nodes: BTreeMap::from([(
            "node-a".to_owned(),
            LayoutPoint {
                x: 1.0,
                y: 2.0,
                width: None,
                height: None,
            },
        )]),
        collapsed_groups: BTreeSet::new(),
        annotations: BTreeMap::new(),
        viewport: None,
    }
    .seal()?;
    layout.validate()?;
    let mut tampered = layout.clone();
    tampered.nodes.get_mut("node-a").ok_or("missing node")?.x = 9.0;
    assert!(tampered.validate().is_err());
    Ok(())
}

#[test]
fn public_timeline_has_no_internal_event_variant_field() -> Result<(), Box<dyn std::error::Error>> {
    let entry = TimelineEntry {
        sequence: 1,
        timestamp_ms: 2,
        category: TimelineCategory::Lifecycle,
        actor: "human:operator".to_owned(),
        run_id: "run-a".to_owned(),
        node_id: None,
        attempt_id: None,
        revision_id: None,
        summary: "run created".to_owned(),
        detail: Value::Null,
    };
    let encoded = String::from_utf8(encode_json(&entry)?)?;
    assert!(!encoded.contains("RunEventKind"));
    assert!(!encoded.contains("run_event_kind"));
    Ok(())
}
