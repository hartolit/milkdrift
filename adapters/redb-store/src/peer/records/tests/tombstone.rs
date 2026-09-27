//! Archived disclosure validates each binding and output bound independently of hot history.
use super::*;
use milkdrift_capability::{
    ArtifactReference, InvocationEvent, InvocationEventKind, InvocationId, PeerId,
};
use milkdrift_peer_protocol::{ObservationCategory, PeerObservation, ServingCaller};
use milkdrift_persistence::{
    PeerArchivedDisposition, PeerExecutionAccounting, PeerExecutionSnapshot, PeerExecutionTombstone,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn tombstone() -> TestResult<PeerExecutionTombstone> {
    let record = published::record()?;
    let authority = record.authority.request();
    Ok(PeerExecutionTombstone {
        schema_version: milkdrift_persistence::SERVING_EXECUTION_TOMBSTONE_SCHEMA_VERSION,
        published_invocation: record.published_invocation,
        output_observations: Vec::new(),
        caller: record.caller,
        authorization: record.request.authorization.clone(),
        relationship_generation: 1,
        request_id: record.request.request_id.clone(),
        request_digest: record.request.request_digest.clone(),
        execution: record.execution,
        acceptance_sequence: 1,
        accepted_at_unix_ms: 100,
        catalog_generation: 1,
        catalog_digest: record.request.catalog_digest.as_str().to_owned(),
        capability: record.request.selection.capability().clone(),
        capability_generation: 1,
        capability_digest: record.request.selection.digest().to_owned(),
        operation: record.request.selection.operation().clone(),
        side_effect: record.request.selection.operation_contract().side_effect(),
        idempotency: record.request.selection.operation_contract().idempotency(),
        authority: milkdrift_persistence::PeerAcceptedAuthoritySummary {
            decision: authority.decision.clone(),
            actor: authority.actor.clone(),
            grant: authority.grant.clone(),
            grant_revision: authority.grant_revision,
            grant_digest: authority.grant_digest.clone(),
            revocation_generation: authority.revocation_generation,
            policy: record.authority.policy().clone(),
            policy_version: record.authority.policy_version(),
            decision_digest: record.authority.digest().to_owned(),
        },
        disposition: PeerArchivedDisposition::Uncertain {
            uncertain_at_unix_ms: 150,
            reason: "lost terminal".into(),
        },
        cancellation: None,
        last_observation_sequence: 0,
        observation_digest: record.observation_digest,
        accounting: PeerExecutionAccounting::default(),
        compacted_through_sequence: 0,
        archived_at_unix_ms: 200,
    })
}

fn output(execution: &PeerExecutionId, sequence: u64) -> TestResult<PeerObservation> {
    Ok(PeerObservation {
        execution: execution.clone(),
        sequence,
        category: ObservationCategory::Artifact,
        event: InvocationEvent::new(
            InvocationId::new("invocation:origin")?,
            sequence,
            InvocationEventKind::Output {
                name: format!("output-{sequence}"),
                reference: ArtifactReference::new(
                    "artifact:output",
                    "0".repeat(64),
                    Some("text/plain".into()),
                    Some(1),
                )?,
            },
        )?,
        observed_at_unix_ms: 120,
    })
}

#[test]
fn archived_readers_refuse_cross_bound_links_and_malformed_output_manifests() -> TestResult {
    for case in [
        "valid",
        "source",
        "invocation",
        "capability",
        "generation",
        "count",
        "maximum",
        "overflow",
        "execution",
        "order",
        "sequence",
        "nonoutput",
        "caller",
        "actor",
    ] {
        let mut value = tombstone()?;
        let count = match case {
            "maximum" => 256,
            "overflow" => 257,
            _ => 2,
        };
        value.output_observations = (1..=count)
            .map(|s| output(&value.execution, s))
            .collect::<TestResult<_>>()?;
        value.last_observation_sequence = count;
        value.compacted_through_sequence = count;
        value.accounting.outputs = u32::try_from(count)?;
        value.accounting.observations = u32::try_from(count)?;
        // Change the enclosing facts where they have no other role in output validation.
        match case {
            "source" => {
                value
                    .published_invocation
                    .as_mut()
                    .ok_or("plan absent")?
                    .source =
                    milkdrift_persistence::published::PublishedInvocationSource::Serving {
                        caller: value.caller.clone(),
                        execution: PeerExecutionId::new("execution:other")?,
                    };
            }
            "invocation" => {
                let plan = value.published_invocation.as_mut().ok_or("plan absent")?;
                plan.invocation = InvocationId::new("invocation:other")?;
                plan.allowance =
                    milkdrift_persistence::ControllerAccountDeclaration::for_published_invocation(
                        plan.child_run.clone(),
                        plan.invocation.clone(),
                        plan.method_digest.clone(),
                        plan.allowance.budget().clone(),
                    )?;
                plan.request = milkdrift_capability::InvocationRequest::new(
                    plan.invocation.clone(),
                    plan.capability.clone(),
                    plan.request.operation().clone(),
                    None,
                    None,
                    Vec::new(),
                    Default::default(),
                )?;
            }
            "capability" => {
                value.capability = milkdrift_capability::CapabilityId::new("method:other")?
            }
            "generation" => value.capability_generation += 1,
            "count" => value.accounting.outputs -= 1,
            "execution" => {
                value.output_observations[0].execution = PeerExecutionId::new("execution:other")?
            }
            "order" => value.output_observations[1] = value.output_observations[0].clone(),
            "sequence" => value.output_observations[1] = output(&value.execution, count + 1)?,
            "nonoutput" => {
                value.output_observations[0].category = ObservationCategory::Progress;
                value.output_observations[0].event = InvocationEvent::new(
                    InvocationId::new("invocation:origin")?,
                    1,
                    InvocationEventKind::Progress {
                        message: "progress".into(),
                        completed_units: None,
                        total_units: None,
                    },
                )?;
            }
            "caller" => {
                value.published_invocation = None;
                value.caller =
                    ServingCaller::peer(&PeerId::new("serving")?, &PeerId::new("other")?);
            }
            "actor" => value.authority.actor = milkdrift_authority::ActorRef::new("peer:other")?,
            _ => {}
        }
        if let Some(plan) = &value.published_invocation {
            plan.validate()?;
        }
        for observation in &value.output_observations {
            observation.validate()?;
        }
        let root = tempfile::tempdir()?;
        let db = Database::create(root.path().join("peer.redb"))?;
        let write = db.begin_write()?;
        crate::schema::initialize_tables(&write)?;
        write
            .open_table(PEER_EXECUTION_LOCATIONS)?
            .insert(value.execution.as_str(), 2)?;
        let bytes = crate::json::encode(&value, "peer execution tombstone")?;
        write
            .open_table(PEER_EXECUTION_TOMBSTONES)?
            .insert(value.execution.as_str(), bytes.as_slice())?;
        let check = |result| {
            if matches!(case, "valid" | "maximum") {
                assert!(
                    matches!(result, Ok(Some(PeerExecutionSnapshot::Archived(actual))) if *actual == value),
                    "{case}"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(PersistenceError::Storage {
                            class: StorageFailureClass::Corruption,
                            ..
                        })
                    ),
                    "{case}"
                );
            }
        };
        check(super::super::snapshot_optional_in_transaction(
            &write,
            &value.execution,
        ));
        write.commit()?;
        check(super::super::snapshot_optional_in_read_transaction(
            &db.begin_read()?,
            &value.execution,
        ));
    }
    Ok(())
}
