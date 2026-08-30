use super::*;

pub(super) fn accepted_sequence(
    request: &CommandRequest,
    sequence: u64,
    kind: &str,
) -> Result<CommandAccepted, PublicFailure> {
    Ok(CommandAccepted {
        command_id: request.command_id.clone(),
        replayed: false,
        resulting_sequence: Some(sequence),
        result_type: kind.to_owned(),
        value: json!({"resulting_sequence": sequence}),
    })
}

pub(super) fn map_resolve(action: ResolveAction) -> ExternalWorkAction {
    match action {
        ResolveAction::Query => ExternalWorkAction::Query,
        ResolveAction::Retry => ExternalWorkAction::Retry,
        ResolveAction::Compensate => ExternalWorkAction::Compensate,
        ResolveAction::Retain => ExternalWorkAction::Retain,
        ResolveAction::ResolveSucceeded => ExternalWorkAction::ResolveSucceeded,
        ResolveAction::ResolveFailed => ExternalWorkAction::ResolveFailed,
    }
}

pub(super) fn public_revision_summary(
    value: &milkdrift_persistence::RevisionSummary,
) -> PublicRevisionSummary {
    PublicRevisionSummary {
        revision_id: value.revision.as_str().to_owned(),
        workflow_id: value.workflow.as_str().to_owned(),
        lineage_sequence: value.lineage_sequence,
        semantic_digest: value.content_digest.as_str().to_owned(),
        parents: value
            .parents
            .iter()
            .map(|parent| parent.as_str().to_owned())
            .collect(),
    }
}

pub(super) fn public_run(value: milkdrift_control::RunInspection) -> RunRead {
    let (lifecycle, terminal) = match value.lifecycle {
        milkdrift_runtime::RunLifecycle::Uncreated => ("uncreated".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Created => ("created".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Running => ("running".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Paused => ("paused".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Cancelling => ("cancelling".to_owned(), None),
        milkdrift_runtime::RunLifecycle::Terminal(outcome) => {
            ("terminal".to_owned(), Some(snake_debug(&outcome)))
        }
    };
    let nodes = value
        .executions
        .into_iter()
        .map(|node| NodeRead {
            execution_id: node.execution.as_str().to_owned(),
            node_id: node.node.as_str().to_owned(),
            revision_id: node.revision.as_str().to_owned(),
            state: snake_debug(&node.state),
            attempt_count: node.attempt_count,
            latest_attempt: node.latest_attempt.map(public_attempt),
        })
        .collect::<Vec<_>>();
    let uncertainty_count = u32::try_from(
        nodes
            .iter()
            .filter(|node| {
                node.latest_attempt
                    .as_ref()
                    .is_some_and(|attempt| attempt.uncertain)
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    RunRead {
        run_id: value.run.as_str().to_owned(),
        sequence: value.sequence.get(),
        lifecycle,
        terminal,
        workflow_id: value.workflow.map(|workflow| workflow.as_str().to_owned()),
        revision_id: value.revision.map(|revision| revision.as_str().to_owned()),
        semantic_digest: value
            .revision_digest
            .map(|digest| digest.as_str().to_owned()),
        nodes,
        uncertainty_count,
    }
}

pub(super) fn empty_attempt_read(attempt: &str, state: &str) -> AttemptRead {
    AttemptRead {
        attempt_id: attempt.to_owned(),
        invocation_id: None,
        state: state.to_owned(),
        capability_id: None,
        descriptor_revision: None,
        capability_provenance: None,
        provider_profile: None,
        peer_id: None,
        context_manifest: None,
        context: None,
        context_access: "absent".to_owned(),
        terminal: None,
        uncertain: false,
    }
}

pub(super) fn public_invocation_artifact(
    artifact: &milkdrift_capability::ArtifactReference,
) -> ArtifactMetadataRead {
    ArtifactMetadataRead {
        artifact_id: artifact.identity().to_owned(),
        digest: artifact.digest().to_owned(),
        size: artifact.size_bytes().unwrap_or(0),
        content_type: artifact
            .media_type()
            .unwrap_or("application/octet-stream")
            .to_owned(),
        disposition_name: None,
        sensitivity: "restricted".to_owned(),
    }
}

pub(super) fn public_attempt(value: milkdrift_control::AttemptInspection) -> AttemptRead {
    let has_context_manifest = value.context_manifest.is_some();
    let capability_id = value
        .capability
        .as_ref()
        .map(|capability| capability.capability().as_str().to_owned());
    let descriptor_revision = value
        .capability
        .as_ref()
        .map(milkdrift_capability::ResolvedCapabilitySnapshot::descriptor_revision);
    let capability_provenance = value.capability.as_ref().map(public_capability_provenance);
    let provider_profile = value.capability.as_ref().and_then(|capability| {
        capability
            .provider_profile()
            .map(|profile| profile.as_str().to_owned())
    });
    let context_manifest = value
        .context_manifest
        .as_ref()
        .map(public_invocation_artifact);
    AttemptRead {
        attempt_id: value.attempt.as_str().to_owned(),
        invocation_id: value
            .invocation
            .map(|invocation| invocation.as_str().to_owned()),
        state: snake_debug(&value.state),
        capability_id,
        descriptor_revision,
        capability_provenance,
        provider_profile,
        peer_id: None,
        context_manifest,
        context: None,
        context_access: if has_context_manifest {
            "metadata_only".to_owned()
        } else {
            "absent".to_owned()
        },
        terminal: value.terminal.as_ref().map(snake_debug),
        uncertain: value.external_outcome.is_some(),
    }
}

pub(super) fn public_capability_provenance(
    snapshot: &milkdrift_capability::ResolvedCapabilitySnapshot,
) -> milkdrift_control_protocol::CapabilityProvenanceRead {
    let process = snapshot
        .descriptor_extensions()
        .iter()
        .find(|(key, _)| key.as_str() == "org.milkdrift/process-profile")
        .map(|(_, value)| value.value());
    let implementation = process.and_then(|value| value.get("implementation"));
    let string = |value: Option<&serde_json::Value>| {
        value.and_then(serde_json::Value::as_str).map(str::to_owned)
    };
    milkdrift_control_protocol::CapabilityProvenanceRead {
        snapshot_digest: snapshot.digest().to_owned(),
        execution_trust: snake_debug(&snapshot.execution_trust()),
        implementation_identity: string(
            implementation.and_then(|value| value.get("identity_digest")),
        ),
        implementation_content_digest: string(
            implementation.and_then(|value| value.get("content_digest")),
        ),
        implementation_size_bytes: implementation
            .and_then(|value| value.get("size_bytes"))
            .and_then(serde_json::Value::as_u64),
        process_profile_digest: string(process.and_then(|value| value.get("profile_digest"))),
        execution_policy_digest: string(
            process.and_then(|value| value.get("execution_policy_digest")),
        ),
        package_revision: string(implementation.and_then(|value| value.get("package_revision"))),
        documentation_reference: string(
            implementation.and_then(|value| value.get("documentation_reference")),
        ),
    }
}

pub(super) fn public_timeline(event: &milkdrift_persistence::RunEventEnvelope) -> TimelineEntry {
    let kind = serde_json::to_value(event.kind()).unwrap_or(Value::Null);
    let kind_name = kind
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("runtime_fact");
    let category = timeline_category(kind_name);
    let actor = kind
        .get("actor")
        .and_then(Value::as_str)
        .unwrap_or("service:runtime")
        .to_owned();
    let node_id = string_field(&kind, &["node", "node_id"]);
    let attempt_id = string_field(&kind, &["attempt", "attempt_id"]);
    let revision_id = string_field(&kind, &["revision", "to_revision", "from_revision"]);
    TimelineEntry {
        sequence: event.sequence().get(),
        timestamp_ms: event.occurred_at().get(),
        category,
        actor,
        run_id: event.run_id().as_str().to_owned(),
        node_id,
        attempt_id,
        revision_id,
        summary: timeline_summary(category),
        detail: json!({"event_id": event.event_id().as_str()}),
    }
}

pub(super) fn timeline_category(kind: &str) -> TimelineCategory {
    if kind.contains("artifact") || kind.contains("output") {
        TimelineCategory::Artifact
    } else if kind.contains("reconciliation") || kind.contains("revision_adoption") {
        TimelineCategory::Reconciliation
    } else if kind.contains("recovery") || kind.contains("re_leased") {
        TimelineCategory::Recovery
    } else if kind.contains("uncertain")
        || kind.contains("retained")
        || kind.contains("late_terminal")
    {
        TimelineCategory::Uncertainty
    } else if kind.contains("signal") || kind.contains("timer") || kind.contains("wait") {
        TimelineCategory::Coordination
    } else if kind.contains("decision") || kind.contains("authority") {
        TimelineCategory::Authority
    } else if kind.contains("node") || kind.contains("lease") || kind.contains("attempt") {
        TimelineCategory::Execution
    } else if kind.contains("progress") || kind.contains("usage") {
        TimelineCategory::Progress
    } else {
        TimelineCategory::Lifecycle
    }
}

pub(super) fn timeline_summary(category: TimelineCategory) -> String {
    match category {
        TimelineCategory::Lifecycle => "run lifecycle changed",
        TimelineCategory::Execution => "node execution changed",
        TimelineCategory::Progress => "execution progress observed",
        TimelineCategory::Artifact => "artifact or output published",
        TimelineCategory::Coordination => "workflow coordination changed",
        TimelineCategory::Authority => "authority decision recorded",
        TimelineCategory::Recovery => "recovery fact recorded",
        TimelineCategory::Reconciliation => "revision reconciliation changed",
        TimelineCategory::Uncertainty => "external outcome requires attention",
    }
    .to_owned()
}

pub(super) fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str).map(str::to_owned))
}

pub(super) fn public_artifact_metadata(
    value: &milkdrift_workspace::ArtifactMetadata,
) -> ArtifactMetadataRead {
    ArtifactMetadataRead {
        artifact_id: value.reference().artifact().as_str().to_owned(),
        digest: value.reference().digest().to_hex(),
        size: value.reference().size_bytes(),
        content_type: value.reference().media_type().as_str().to_owned(),
        disposition_name: None,
        sensitivity: snake_debug(&value.sensitivity()),
    }
}

pub(super) fn diff_keys<K, V>(
    subject: &str,
    left: &BTreeMap<K, V>,
    right: &BTreeMap<K, V>,
    output: &mut Vec<RevisionChange>,
) where
    K: Ord + ToString,
    V: PartialEq,
{
    for key in left.keys().chain(right.keys()).collect::<BTreeSet<_>>() {
        let change = match (left.get(key), right.get(key)) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (Some(left), Some(right)) if left != right => "changed",
            _ => continue,
        };
        output.push(RevisionChange {
            change: change.to_owned(),
            subject: subject.to_owned(),
            identity: Some(key.to_string()),
            detail: Value::Null,
        });
    }
}

pub(super) fn parse_run_state(value: &str) -> Result<IndexedRunState, PublicFailure> {
    match value {
        "created" => Ok(IndexedRunState::Created),
        "runnable" => Ok(IndexedRunState::Runnable),
        "active" => Ok(IndexedRunState::Active),
        "paused" => Ok(IndexedRunState::Paused),
        "cancelling" => Ok(IndexedRunState::Cancelling),
        "waiting" => Ok(IndexedRunState::Waiting),
        "uncertain" => Ok(IndexedRunState::Uncertain),
        "terminal" => Ok(IndexedRunState::Terminal),
        _ => Err(invalid("unknown run state filter")),
    }
}

pub(super) fn parse_revision_id(value: &str) -> Result<RevisionId, PublicFailure> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| invalid(&error.to_string()))
}

pub(super) fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

pub(super) fn snake_debug(value: &impl std::fmt::Debug) -> String {
    let source = format!("{value:?}");
    let mut result = String::with_capacity(source.len() + 4);
    for (index, character) in source.chars().enumerate() {
        if character.is_ascii_uppercase() && index != 0 {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}

pub(super) fn bounded(value: &str) -> String {
    if value.len() <= 4_096 {
        return value.to_owned();
    }
    let mut end = 4_096;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

pub(super) fn cursor_binding(
    session: &ActorSession,
    exact_resource_and_filter: &str,
) -> Result<CursorBinding, PublicFailure> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.continuation-scope.v1\0");
    hasher.update(exact_resource_and_filter.as_bytes());
    Ok(CursorBinding {
        actor: session.actor.as_str().to_owned(),
        grant_id: session.grant.identity().as_str().to_owned(),
        grant_revision: session.grant.revision(),
        grant_digest: session
            .grant
            .digest()
            .map_err(|_| internal())?
            .as_str()
            .to_owned(),
        scope_digest: format!("b3_{}", hasher.finalize()),
    })
}

pub(super) fn invalid(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::InvalidInput, bounded(message), false)
}

pub(super) fn conflict(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::Conflict, message, false)
}

pub(super) fn unauthorized() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Unauthorized,
        "authority denied the operation",
        false,
    )
}

pub(super) fn unauthorized_decision(decision: &AuthorityDecisionSnapshot) -> PublicFailure {
    let mut failure = unauthorized();
    failure
        .details
        .insert("decision_digest".to_owned(), decision.digest().to_owned());
    failure.details.insert(
        "reason_codes".to_owned(),
        decision
            .reason_codes()
            .iter()
            .map(snake_debug)
            .collect::<Vec<_>>()
            .join(","),
    );
    failure
}

pub(super) fn not_found() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::NotFound,
        "requested resource was not found",
        false,
    )
}

pub(super) fn corruption(message: &str) -> PublicFailure {
    PublicFailure::new(ErrorCode::Corruption, message, false)
}

pub(super) fn internal() -> PublicFailure {
    PublicFailure::new(
        ErrorCode::Internal,
        "internal control operation failed",
        false,
    )
}

pub(super) fn public_protocol(error: milkdrift_control_protocol::ProtocolError) -> PublicFailure {
    match error {
        milkdrift_control_protocol::ProtocolError::UnsupportedMajor { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        milkdrift_control_protocol::ProtocolError::Bounds(_) => {
            PublicFailure::new(ErrorCode::Overload, bounded(&error.to_string()), false)
        }
        _ => invalid(&error.to_string()),
    }
}

pub(super) fn public_control(error: ControlError) -> PublicFailure {
    match error {
        ControlError::AuthorizationDenied {
            reasons,
            decision_digest,
        } => {
            let mut failure = unauthorized();
            if let Some(digest) = decision_digest {
                failure.details.insert("decision_digest".to_owned(), digest);
            }
            failure.details.insert(
                "reason_codes".to_owned(),
                reasons
                    .iter()
                    .map(snake_debug)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            failure
        }
        ControlError::StaleRunSequence { expected, actual } => {
            let mut failure = conflict("run sequence guard is stale");
            failure
                .details
                .insert("expected_sequence".to_owned(), expected.get().to_string());
            failure
                .details
                .insert("actual_sequence".to_owned(), actual.get().to_string());
            failure
        }
        ControlError::ApprovalRequired { .. }
        | ControlError::ProposalState(_)
        | ControlError::BaseRevisionMismatch => conflict(&bounded(&error.to_string())),
        ControlError::BaseRevisionNotFound => not_found(),
        ControlError::UnsupportedVersion { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        ControlError::Persistence(error) => public_persistence(error),
        ControlError::Runtime(milkdrift_runtime::RuntimeError::Persistence(error)) => {
            public_persistence(error)
        }
        ControlError::Runtime(milkdrift_runtime::RuntimeError::AuthorizationDenied { .. }) => {
            unauthorized()
        }
        ControlError::Runtime(error) if error.to_string().contains("transition") => {
            conflict(&bounded(&error.to_string()))
        }
        _ => invalid(&bounded(&error.to_string())),
    }
}

pub(super) fn public_persistence(error: PersistenceError) -> PublicFailure {
    match error {
        PersistenceError::SequenceConflict {
            expected, actual, ..
        } => {
            let mut failure = conflict("run sequence guard is stale");
            failure
                .details
                .insert("expected_sequence".to_owned(), expected.get().to_string());
            failure
                .details
                .insert("actual_sequence".to_owned(), actual.get().to_string());
            failure
        }
        PersistenceError::IdempotencyConflict { .. }
        | PersistenceError::ExternalCommandIdempotencyConflict { .. }
        | PersistenceError::ImmutableConflict { .. }
        | PersistenceError::WorkspaceUsageConflict { .. }
        | PersistenceError::LeaseRevisionConflict { .. } => conflict(&bounded(&error.to_string())),
        PersistenceError::NotFound { .. } => not_found(),
        PersistenceError::ArtifactAccessDenied(_) => unauthorized(),
        PersistenceError::Corruption(_) => PublicFailure::new(
            ErrorCode::Corruption,
            "durable integrity verification failed",
            false,
        ),
        PersistenceError::UnsupportedVersion { .. }
        | PersistenceError::MigrationRequired { .. } => PublicFailure::new(
            ErrorCode::UnsupportedVersion,
            bounded(&error.to_string()),
            false,
        ),
        PersistenceError::Storage { class, .. } => {
            let code = match class {
                milkdrift_persistence::StorageFailureClass::Corruption => ErrorCode::Corruption,
                milkdrift_persistence::StorageFailureClass::ResourceExhausted => {
                    ErrorCode::Overload
                }
                milkdrift_persistence::StorageFailureClass::Unavailable
                | milkdrift_persistence::StorageFailureClass::OwnerBusy => ErrorCode::Unavailable,
                milkdrift_persistence::StorageFailureClass::Migration => {
                    ErrorCode::UnsupportedVersion
                }
                milkdrift_persistence::StorageFailureClass::Internal => ErrorCode::Internal,
            };
            PublicFailure::new(
                code,
                "durable storage operation failed",
                matches!(code, ErrorCode::Unavailable | ErrorCode::Overload),
            )
        }
        PersistenceError::Bounds {
            location: "application_receipt_retention",
            ..
        } => PublicFailure::new(
            ErrorCode::Overload,
            "durable command receipt retention bound was reached",
            false,
        ),
        _ => invalid(&bounded(&error.to_string())),
    }
}
