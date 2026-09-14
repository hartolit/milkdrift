//! Replay is a read-only projection; no RuntimeService or external capabilities exist here.
use super::Result;
use milkdrift_blueprint::NodeKind;
use milkdrift_model::ContextManifestDocument;
use milkdrift_persistence::{EventPageQuery, PageSize};
use milkdrift_redb_store::offline::OfflineStore;
use milkdrift_runtime::{ContextBuildIdentity, RunProjection, validate_retained_manifest};
use milkdrift_workspace::{ArtifactId, ArtifactReference, ContentDigest, MediaType, RunId};
use serde_json::{Value, json};

pub(super) fn run(
    store: &OfflineStore,
    run: RunId,
    maximum_events: u32,
    limit: u32,
    after: Option<&str>,
) -> Result<Value> {
    let mut projection = RunProjection::new();
    let mut cursor = None;
    let mut visited = 0u64;
    loop {
        let page = store.queries().events(&EventPageQuery::new(
            run.clone(),
            cursor,
            PageSize::new(16)?,
        )?)?;
        if page.observed_head.get() > u64::from(maximum_events) {
            return Err("run exceeds --maximum-events; inspection stopped without a health verdict; choose a larger explicit bound or inspect physical families".into());
        }
        for event in &page.events {
            projection.apply(event).map_err(|_| "corrupt run history: read-only projection failed; use the resumable integrity scan")?;
            visited += 1;
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    if visited == 0 {
        return Err("run is unavailable".into());
    }
    let mut attempts = Vec::new();
    let mut next = None;
    for (id, attempt) in projection.attempts() {
        if after.is_some_and(|after| id.as_str() <= after) {
            continue;
        }
        if attempts.len() == limit as usize {
            next = attempts
                .last()
                .and_then(|v: &Value| v["attempt"].as_str())
                .map(str::to_owned);
            break;
        }
        let execution = projection.node_executions().get(attempt.execution());
        let revision = projection.revision_for_attempt(id);
        let context = match (
            execution,
            revision,
            attempt.request().and_then(|r| r.context_manifest()),
        ) {
            (Some(execution), Some(revision), Some(reference)) => context(
                store,
                reference,
                &ContextBuildIdentity {
                    run: run.clone(),
                    revision: revision.clone(),
                    node: execution.node().clone(),
                    execution: attempt.execution().clone(),
                    attempt: id.clone(),
                },
            ),
            (_, _, None) => json!({"classification": "no_frozen_manifest"}),
            _ => {
                json!({"classification": "corrupt_record", "detail": "missing governing execution/revision"})
            }
        };
        let blocks_context_recovery = attempt.state() == &milkdrift_runtime::AttemptState::Leased
            && context["classification"].as_str().is_some_and(|s| {
                !["context_reuse_check_passed", "no_frozen_manifest"].contains(&s)
            });
        let leases: Vec<_> = attempt
            .leases()
            .iter()
            .filter_map(|id| projection.leases().get(id))
            .collect();
        attempts.push(json!({"attempt": id, "execution": attempt.execution(), "node": execution.map(|e| e.node()),
            "revision": revision, "invocation": attempt.invocation(), "state": attempt.state(),
            "uncertain": attempt.is_unresolved(), "leases": leases, "context": context,
            "blocks_context_recovery": blocks_context_recovery}));
    }
    Ok(
        json!({"run": run, "lifecycle": projection.lifecycle(), "through_sequence": projection.sequence(),
        "history_compacted_through": projection.history_compacted_through(), "events_examined": visited,
        "attempts": attempts, "next_attempt": next,
        "scope": "retained attempt frontier; completed detail can be compacted. Context checks do not establish overall startup readiness, current authority, provider availability or external outcome.",
        "disposition": "Preserve this generation before repair. If retained context blocks normal startup, start the daemon with --recovery and its existing configuration/grants to review and approve a prospective repair. Restart normally to validate all active state before execution; inspection does not authorize reuse or settle external effects."}),
    )
}

fn context(
    store: &OfflineStore,
    reference: &milkdrift_capability::ArtifactReference,
    identity: &ContextBuildIdentity,
) -> Value {
    match read_context(store, reference, identity) {
        Ok(value) => value,
        Err(error) => {
            json!({"classification": error, "detail": "frozen evidence remains unchanged; no reuse decision is available"})
        }
    }
}

fn read_context(
    store: &OfflineStore,
    reference: &milkdrift_capability::ArtifactReference,
    identity: &ContextBuildIdentity,
) -> std::result::Result<Value, &'static str> {
    let reference = ArtifactReference::new(
        ArtifactId::new(reference.identity()).map_err(|_| "corrupt_reference")?,
        ContentDigest::from_hex(reference.digest()).map_err(|_| "corrupt_reference")?,
        MediaType::new(reference.media_type().ok_or("corrupt_reference")?)
            .map_err(|_| "corrupt_reference")?,
        reference.size_bytes().ok_or("corrupt_reference")?,
    );
    let bytes = store
        .artifact_bytes(&reference, milkdrift_model::MAX_MODEL_DOCUMENT_BYTES as u64)
        .map_err(|error| match error {
            milkdrift_persistence::PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::Unavailable,
                ..
            } => "unavailable_content",
            milkdrift_persistence::PersistenceError::Bounds { .. } => "content_read_bound_exceeded",
            _ => "corrupt_content",
        })?;
    let document = ContextManifestDocument::from_json(&bytes).map_err(|error| match error {
        milkdrift_model::ModelContractError::UnsupportedVersion { .. } => "unsupported_schema",
        _ => "corrupt_record",
    })?;
    let manifest = document.body();
    let revision = store
        .revision(&identity.revision)
        .map_err(|_| "corrupt_revision")?
        .ok_or("unavailable_revision")?;
    let Some(NodeKind::Task { config }) = revision
        .semantic()
        .nodes()
        .get(&identity.node)
        .map(|n| n.kind())
    else {
        return Err("corrupt_governing_task");
    };
    let validation = validate_retained_manifest(manifest, identity, config.context_policy());
    // Never serialize saved omissions or selected content. Legacy source/size fields
    // may encode metadata that the corrected selector would have hidden. The original
    // digest identifies the untouched manifest; this projection is not a replacement.
    Ok(
        json!({"classification": if validation.is_ok() { "context_reuse_check_passed" } else { "unsafe_but_readable" },
        "reason": validation.err().map(|e| e.to_string()), "manifest_artifact": reference.artifact(),
        "manifest_digest": manifest.digest(), "schema_version": manifest.schema_version(),
        "selection_policy_version": manifest.policy_version(), "policy_digest": manifest.policy_digest(),
        "policy": config.context_policy(), "selected_count": manifest.entries().len(),
        "omission_count": manifest.omissions().len(), "omissions": manifest.omissions().iter().take(128)
            .map(|o| json!({"reason": o.reason, "required": o.required})).collect::<Vec<_>>(),
        "omissions_truncated": manifest.omissions().len() > 128,
        "redaction": "all source identities, sizes, provenance and content from selections/omissions are withheld"}),
    )
}
