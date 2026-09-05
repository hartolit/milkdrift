use super::*;

#[test]
fn registration_binds_bytes_profile_policy_trust_and_attempt_provenance() -> TestResult {
    let executable_owner = tempfile::tempdir()?;
    let executable = copy_test_helper(executable_owner.path(), "pinned-helper")?;
    let data = Arc::new(TestDataAccess::new()?);
    let canonical_executable_root = executable
        .parent()
        .ok_or("test executable has no parent")?
        .canonicalize()?;
    let canonical_data_root = data.root.canonicalize()?;
    let mut value = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    retarget_profile(&mut value, &executable)?;
    let profile = parse_profile(&value)?;
    let adapter = LocalProcessAdapter::new(
        profile.clone(),
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    assert_eq!(
        adapter.descriptor().execution_trust(),
        ExecutionTrustClass::TrustedHostProcess
    );
    let requirements = adapter.authority_requirements();
    assert!(
        requirements
            .filesystem
            .contains(&FilesystemScope::from_canonical_host_path(
                &canonical_executable_root,
                BTreeSet::from([AccessMode::Execute]),
            )?)
    );
    assert!(
        requirements
            .filesystem
            .contains(&FilesystemScope::from_canonical_host_path(
                &canonical_data_root,
                BTreeSet::from([AccessMode::Read, AccessMode::Write]),
            )?)
    );
    let extension = process_extension(&adapter)?;
    assert_eq!(
        extension["implementation"]["content_digest"],
        json!(profile.implementation().content_digest())
    );
    assert_eq!(
        extension["implementation"]["size_bytes"],
        json!(profile.implementation().size_bytes())
    );
    for digest in [
        &extension["implementation"]["identity_digest"],
        &extension["profile_digest"],
        &extension["execution_policy_digest"],
    ] {
        assert!(
            digest
                .as_str()
                .is_some_and(|value| value.starts_with("b3_"))
        );
    }

    let sandbox = CapabilityRequirement::new(profile.operation().clone())
        .execution_trust(ExecutionTrustClass::SandboxedProcess);
    let mismatch = adapter.descriptor().matches(&sandbox);
    assert!(!mismatch.is_match());
    assert_eq!(mismatch.mismatch_reasons(), &["execution_trust"]);

    let snapshot =
        ResolvedCapabilitySnapshot::from_descriptor(adapter.descriptor(), profile.operation())?;
    assert_eq!(
        snapshot.execution_trust(),
        ExecutionTrustClass::TrustedHostProcess
    );
    assert_eq!(
        snapshot.descriptor_extensions(),
        adapter.descriptor().extensions()
    );
    let request = request(&profile, "invocation-admission-envelope", Vec::new())?;
    let context = context()?;
    let invocation = AdapterInvocation::with_context(&snapshot, &request, &context);
    let first_envelope = adapter.admission_envelope(&invocation)?;
    let second_envelope = adapter.admission_envelope(&invocation)?;
    assert_eq!(first_envelope, second_envelope);
    assert!(matches!(
        first_envelope.input_units(),
        AdmissionBound::NotApplicable
    ));
    assert!(matches!(
        first_envelope.output_units(),
        AdmissionBound::NotApplicable
    ));
    assert!(matches!(
        first_envelope.monetary_cost(),
        AdmissionBound::NotApplicable
    ));
    assert_eq!(first_envelope.artifact_bytes().bounded(), Some(&4_194_304));
    Ok(())
}

#[test]
fn profile_semantics_change_policy_and_descriptor_identity() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let first_value = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    let first = LocalProcessAdapter::new(
        parse_profile(&first_value)?,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let mut second_value = first_value;
    second_value["profile"]["revision"] = json!(2);
    second_value["profile"]["descriptor_revision"] = json!(2);
    second_value["profile"]["arguments"] = json!(["exit", "7"]);
    let second = LocalProcessAdapter::new(
        parse_profile(&second_value)?,
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let first_extension = process_extension(&first)?;
    let second_extension = process_extension(&second)?;
    assert_ne!(
        first_extension["profile_digest"],
        second_extension["profile_digest"]
    );
    assert_ne!(
        first_extension["execution_policy_digest"],
        second_extension["execution_policy_digest"]
    );
    assert_eq!(
        first_extension["implementation"]["identity_digest"],
        second_extension["implementation"]["identity_digest"]
    );
    assert_ne!(first.descriptor(), second.descriptor());
    Ok(())
}

#[test]
fn documentation_changes_metadata_but_not_execution_identity_or_policy() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let first_value = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    let first = LocalProcessAdapter::new(
        parse_profile(&first_value)?,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let mut second_value = first_value;
    second_value["profile"]["implementation"]["documentation_reference"] =
        json!("urn:milkdrift:test-helper:updated-docs");
    let second = LocalProcessAdapter::new(
        parse_profile(&second_value)?,
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let first_extension = process_extension(&first)?;
    let second_extension = process_extension(&second)?;
    assert_eq!(
        first_extension["implementation"]["identity_digest"],
        second_extension["implementation"]["identity_digest"]
    );
    assert_eq!(
        first_extension["execution_policy_digest"],
        second_extension["execution_policy_digest"]
    );
    assert_ne!(
        first_extension["profile_digest"],
        second_extension["profile_digest"]
    );
    assert_ne!(first.descriptor(), second.descriptor());
    Ok(())
}

#[test]
fn changed_bytes_make_health_sticky_unavailable_even_after_restore() -> TestResult {
    let executable_owner = tempfile::tempdir()?;
    let executable = copy_test_helper(executable_owner.path(), "mutable-helper")?;
    let original = fs::read(&executable)?;
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    retarget_profile(&mut value, &executable)?;
    let adapter = LocalProcessAdapter::new(
        parse_profile(&value)?,
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    adapter.start()?;
    fs::write(&executable, b"changed executable bytes")?;
    let changed = adapter.health(10)?;
    assert!(!changed.available());
    assert_eq!(changed.health_summary(), "tool_size_mismatch");
    fs::write(&executable, original)?;
    let restored = adapter.health(11)?;
    assert!(!restored.available());
    assert_eq!(restored.health_summary(), "tool_size_mismatch");
    Ok(())
}

#[test]
fn pre_spawn_replacement_is_rejected_before_child_entry_and_remains_invalidated() -> TestResult {
    let executable_owner = tempfile::tempdir()?;
    let executable = copy_test_helper(executable_owner.path(), "pre-entry-helper")?;
    let original = fs::read(&executable)?;
    let marker_owner = tempfile::tempdir()?;
    let marker = marker_owner.path().join("entered");
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(
        &data.root,
        vec![json!("mark"), json!(marker.to_string_lossy())],
    )?;
    retarget_profile(&mut value, &executable)?;
    let profile = parse_profile(&value)?;
    let first_request = request(&profile, "invocation-pre-entry-replaced", Vec::new())?;
    let adapter = Arc::new(LocalProcessAdapter::new(
        profile.clone(),
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?);
    let descriptor = adapter.descriptor().clone();
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, profile.operation())?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 2,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    host.register(
        descriptor.clone(),
        adapter,
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            1,
            true,
            0,
            "registered",
        )?),
    )?;

    fs::write(&executable, b"changed executable bytes")?;
    let first_reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &first_request, &context()?, &first_reporter)?;
    let first_events = first_reporter.events()?;
    assert_eq!(
        terminal_status(&first_events),
        Some(TerminalStatus::Rejected)
    );
    assert_eq!(
        terminal_failure_code(&first_events),
        Some("tool_size_mismatch")
    );
    assert!(!marker.exists());

    fs::write(&executable, original)?;
    let second_request = request(&profile, "invocation-restored-stale-generation", Vec::new())?;
    let second_reporter = TestReporter::default();
    host.execute_exact_with_context(&snapshot, &second_request, &context()?, &second_reporter)?;
    let second_events = second_reporter.events()?;
    assert_eq!(
        terminal_status(&second_events),
        Some(TerminalStatus::Rejected)
    );
    assert_eq!(
        terminal_failure_code(&second_events),
        Some("tool_size_mismatch")
    );
    assert!(!marker.exists());

    value["profile"]["revision"] = json!(2);
    value["profile"]["descriptor_revision"] = json!(2);
    let replacement_profile = parse_profile(&value)?;
    let replacement_adapter = Arc::new(LocalProcessAdapter::new(
        replacement_profile,
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?);
    let replacement_descriptor = replacement_adapter.descriptor().clone();
    host.register(
        replacement_descriptor.clone(),
        replacement_adapter.clone(),
        None,
    )?;
    host.refresh_health(
        replacement_descriptor.identity(),
        replacement_descriptor.descriptor_revision(),
        2,
    )?;
    assert!(replacement_adapter.health(3)?.available());
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_target_replacement_and_root_escape_are_rejected() -> TestResult {
    use std::os::unix::fs::symlink;

    let allowed = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let target = copy_test_helper(allowed.path(), "target")?;
    let escaped_target = copy_test_helper(outside.path(), "escaped-target")?;
    let configured = allowed.path().join("configured-link");
    symlink(&target, &configured)?;
    let data = Arc::new(TestDataAccess::new()?);
    let mut value = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    retarget_profile(&mut value, &configured)?;
    let adapter = LocalProcessAdapter::new(
        parse_profile(&value)?,
        data,
        Arc::new(InMemorySecretResolver::new()),
    )?;
    adapter.start()?;
    fs::remove_file(&configured)?;
    symlink(&escaped_target, &configured)?;
    let observation = adapter.health(1)?;
    assert!(!observation.available());
    assert_eq!(observation.health_summary(), "tool_path_resolution_changed");
    Ok(())
}

#[test]
fn wrong_digest_revision_bounds_and_future_schema_are_refused() -> TestResult {
    let data = Arc::new(TestDataAccess::new()?);
    let mut wrong_digest = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    wrong_digest["profile"]["implementation"]["content_digest"] =
        json!(format!("b3_{}", "0".repeat(64)));
    let profile = parse_profile(&wrong_digest)?;
    let error = LocalProcessAdapter::new(
        profile,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )
    .err()
    .ok_or("wrong executable digest must fail registration")?;
    assert!(error.to_string().contains("tool_content_digest_mismatch"));

    let mut revision = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    revision["profile"]["descriptor_revision"] = json!(2);
    assert!(parse_profile(&revision).is_err());

    let mut oversized = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    oversized["profile"]["implementation"]["documentation_reference"] = json!("x".repeat(1025));
    assert!(parse_profile(&oversized).is_err());

    let mut future = profile_value(&data.root, vec![json!("exit"), json!("0")])?;
    future["schema_version"] = json!(3);
    assert!(ProcessProfileDocument::from_json(&serde_json::to_vec(&future)?).is_err());
    Ok(())
}
