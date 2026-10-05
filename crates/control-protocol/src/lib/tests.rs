use super::*;

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
fn semantic_comparison_wire_preserves_categories_and_truncation()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = br#"{"from_revision":"from","to_revision":"to","changes":[{"change":"changed","subject":"metadata","identity":"description","detail":null},{"change":"removed","subject":"input","identity":"brief","detail":null},{"change":"added","subject":"agreement","identity":null,"detail":null}],"truncated":true}"#;
    let value: RevisionDiffRead = decode_json(bytes)?;
    assert!(value.truncated);
    assert_eq!(value.changes.len(), 3);
    assert_eq!(
        serde_json::from_slice::<Value>(bytes)?,
        serde_json::to_value(&value)?
    );
    assert_eq!(
        decode_json::<RevisionDiffRead>(&encode_json(&value)?)?,
        value
    );
    Ok(())
}

#[test]
fn start_inputs_use_named_artifacts_and_preserve_empty_request_encoding()
-> Result<(), Box<dyn std::error::Error>> {
    let old_shape = serde_json::json!({"type":"start_run", "run_id":"run", "workflow_id":"workflow", "revision_id":"revision"});
    let command: Command = decode_json(&serde_json::to_vec(&old_shape)?)?;
    assert_eq!(serde_json::to_value(command)?, old_shape);
    let mut with_inputs = old_shape;
    with_inputs
        .as_object_mut()
        .ok_or("fixture must be an object")?
        .insert(
            "inputs".to_owned(),
            serde_json::json!([{"name":"brief", "artifact_id":"input:one"}]),
        );
    let command: Command = decode_json(&serde_json::to_vec(&with_inputs)?)?;
    assert_eq!(serde_json::to_value(command)?, with_inputs);
    for inputs in [
        serde_json::json!([{"name":"brief", "path":"/server/file"}]),
        serde_json::json!([{"name":"brief", "artifact_id":42}]),
        serde_json::json!({"brief":"input:one"}),
    ] {
        with_inputs
            .as_object_mut()
            .ok_or("fixture must be an object")?
            .insert("inputs".to_owned(), inputs);
        assert!(decode_json::<Command>(&serde_json::to_vec(&with_inputs)?).is_err());
    }
    Ok(())
}

#[test]
fn authoring_wire_uses_explicit_sources_and_rejects_unknown_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = br#"{"protocol":{"major":2,"minor":19},"command_id":"edit-1","expected_sequence":null,"expected_revision":null,"reason":"Connect the brief","evidence":[],"command":{"type":"author_blueprint","draft":{"workflow_id":"release-notes","base_revision":null,"mutations":[]},"edit":{"type":"connect","step":"review","input":"brief","source":{"type":"run_input","name":"brief"}},"save":false}}"#;
    let request: CommandRequest = decode_json(bytes)?;
    request.validate()?;
    assert_eq!(
        decode_json::<CommandRequest>(&encode_json(&request)?)?,
        request
    );
    let mut value: Value = serde_json::from_slice(bytes)?;
    value
        .get_mut("command")
        .ok_or("fixture field command missing")?
        .get_mut("edit")
        .ok_or("fixture field edit missing")?
        .get_mut("source")
        .ok_or("fixture field source missing")?
        .as_object_mut()
        .ok_or("fixture must be an object")?
        .insert("conversation".to_owned(), Value::Bool(true));
    assert!(decode_json::<CommandRequest>(&serde_json::to_vec(&value)?).is_err());
    value
        .get_mut("command")
        .ok_or("fixture field command missing")?
        .get_mut("edit")
        .ok_or("fixture field edit missing")?
        .as_object_mut()
        .ok_or("fixture must be an object")?
        .insert(
            "source".to_owned(),
            serde_json::json!({"type":"all_history"}),
        );
    assert!(decode_json::<CommandRequest>(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}

#[test]
fn declared_input_edits_roundtrip_and_refuse_ambiguous_fields()
-> Result<(), Box<dyn std::error::Error>> {
    for value in [
        serde_json::json!({"type":"rename_input", "name":"breif", "new_name":"brief"}),
        serde_json::json!({"type":"remove_input", "name":"unused"}),
    ] {
        let edit: BlueprintEdit = decode_json(&serde_json::to_vec(&value)?)?;
        assert_eq!(serde_json::to_value(edit)?, value);
        let mut invalid = value;
        invalid
            .as_object_mut()
            .ok_or("object absent")?
            .insert("disconnect_consumers".into(), Value::Bool(true));
        assert!(decode_json::<BlueprintEdit>(&serde_json::to_vec(&invalid)?).is_err());
    }
    Ok(())
}

#[test]
fn run_accounting_distinguishes_legacy_unavailable_from_explicit_inactive()
-> Result<(), Box<dyn std::error::Error>> {
    let mut document = serde_json::json!({"run_id":"ordinary", "sequence":1, "lifecycle":"created", "terminal":null,
        "workflow_id":"workflow", "revision_id":null, "semantic_digest":null, "nodes":[], "governing_agreement": null, "agreement_adoptions": 0, "uncertainty_count":0});
    let legacy: RunRead = decode_json(&serde_json::to_vec(&document)?)?;
    assert!(legacy.controller_accounting.is_null());
    document
        .as_object_mut()
        .ok_or("fixture must be an object")?
        .insert(
            "controller_accounting".to_owned(),
            serde_json::json!({"state":"inactive"}),
        );
    let current: RunRead = decode_json(&serde_json::to_vec(&document)?)?;
    assert_eq!(
        current.controller_accounting,
        serde_json::json!({"state":"inactive"})
    );
    assert_eq!(serde_json::to_value(current)?, document);
    Ok(())
}

#[test]
fn output_edits_use_strict_unit_and_selection_wire_shapes() -> Result<(), Box<dyn std::error::Error>>
{
    for value in [
        serde_json::json!({"type":"clear_output"}),
        serde_json::json!({"type":"output_limit", "step":"review", "maximum_output_units":1024}),
    ] {
        let edit: BlueprintEdit = decode_json(&serde_json::to_vec(&value)?)?;
        assert_eq!(serde_json::to_value(edit)?, value);
        let mut invalid = value;
        invalid
            .as_object_mut()
            .ok_or("object absent")?
            .insert("replace_step".into(), Value::Bool(true));
        assert!(decode_json::<BlueprintEdit>(&serde_json::to_vec(&invalid)?).is_err());
    }
    assert!(
        decode_json::<BlueprintEdit>(
            br#"{"type":"output_limit","step":"review","maximum_output_units":-1}"#
        )
        .is_err()
    );
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
    for minor in [0, PROTOCOL_MINOR - 1, PROTOCOL_MINOR + 1, u16::MAX] {
        assert!(matches!(
            ProtocolVersion {
                major: PROTOCOL_MAJOR,
                minor
            }
            .negotiate(),
            Err(ProtocolError::UnsupportedMinor { .. })
        ));
    }
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
    let mut tampered = layout;
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
