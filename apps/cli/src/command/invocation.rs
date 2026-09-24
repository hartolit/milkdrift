//! Preserve saved request identity and distinguish admission from observed outcomes.
use crate::{InvocationCommand, error::CliError, session::CliSession};
use milkdrift_peer_protocol::{
    DirectInvocationRequest, PeerCancellationRequest, PeerExecutionId, PeerRequestId,
};

pub(super) async fn execute(
    session: &CliSession,
    command: &InvocationCommand,
) -> Result<(), CliError> {
    let client = session.client();
    match command {
        InvocationCommand::Output {
            execution,
            artifact,
            destination,
        } => {
            let execution = PeerExecutionId::new(execution)
                .map_err(|error| CliError::Invalid(error.to_string()))?;
            let mut file = crate::output::PendingFile::create(destination)?;
            let mut offset = 0;
            let mut hash = blake3::Hasher::new();
            let mut expected = None;
            loop {
                let chunk = client
                    .invocation_output(&execution, artifact, offset, 65_536)
                    .await?;
                if expected
                    .as_ref()
                    .is_some_and(|value| value != &chunk.metadata)
                {
                    return Err(CliError::Internal(
                        "output metadata changed between ranges".to_owned(),
                    ));
                }
                if chunk.bytes.is_empty() && !chunk.complete {
                    return Err(CliError::Internal(
                        "output download made no progress".to_owned(),
                    ));
                }
                std::io::Write::write_all(&mut file, &chunk.bytes)
                    .map_err(|error| CliError::Internal(error.to_string()))?;
                hash.update(&chunk.bytes);
                offset += chunk.bytes.len() as u64;
                let reference = chunk.metadata.reference();
                if chunk.complete {
                    if hash.finalize().to_hex().as_str() != reference.digest().to_hex() {
                        return Err(CliError::Internal(
                            "output bytes contradict the immutable digest".to_owned(),
                        ));
                    }
                    file.commit()
                        .map_err(|error| CliError::Internal(error.to_string()))?;
                    return session.output("invocation.output", &serde_json::json!({
                        "execution": execution, "artifact": reference, "destination": destination
                    }));
                }
                expected = Some(chunk.metadata);
            }
        }
        InvocationCommand::Catalog => {
            session.output("invocation.catalog", &client.execution_discovery().await?)
        }
        InvocationCommand::Prepare {
            capability,
            operation,
            host,
            request_id,
            inputs,
            output,
        } => {
            use milkdrift_capability::{
                IdempotencyKey, InputReference, InvocationId, InvocationRequest,
                InvocationValueReference, OperationId, ResolvedCapabilitySnapshot, SideEffectClass,
            };
            let invalid = |error: &dyn std::fmt::Display| CliError::Invalid(error.to_string());
            let inputs: Vec<InputReference> = serde_json::from_value(
                session
                    .read_json(
                        inputs,
                        milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                        "invocation inputs",
                    )
                    .await?,
            )
            .map_err(|error| invalid(&error))?;
            if inputs.iter().any(|input| {
                matches!(
                    input.value(),
                    InvocationValueReference::WorkspaceValue { .. }
                )
            }) {
                return Err(CliError::Invalid(
                    "direct inputs must be inline values or artifact references".to_owned(),
                ));
            }
            let discovery = client.execution_discovery().await?;
            if discovery.host.as_str() != host {
                return Err(CliError::Invalid(
                    "discovery belongs to another host".to_owned(),
                ));
            }
            let operation = OperationId::new(operation).map_err(|error| invalid(&error))?;
            let entry = discovery
                .catalog
                .entries
                .iter()
                .find(|entry| {
                    entry.descriptor.identity().as_str() == capability
                        && entry.invocable_operations.contains(&operation)
                        && !entry.draining
                })
                .ok_or_else(|| {
                    CliError::Invalid("capability operation is not currently callable".to_owned())
                })?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| invalid(&error))?;
            let deadline_unix_ms = u64::try_from(now.as_millis())
                .ok()
                .and_then(|now| now.checked_add(discovery.limits.duration_ms))
                .ok_or_else(|| {
                    CliError::Invalid("invocation deadline exceeds the clock range".to_owned())
                })?;
            let selection =
                ResolvedCapabilitySnapshot::from_descriptor(&entry.descriptor, &operation)
                    .map_err(|error| invalid(&error))?;
            // The saved request ID is already the caller's stable replay identity. Operations
            // promising idempotent writes also require a key at adapter entry.
            let key = (selection.operation_contract().side_effect()
                == SideEffectClass::IdempotentWrite)
                .then(|| IdempotencyKey::new(request_id))
                .transpose()
                .map_err(|error| invalid(&error))?;
            let request = DirectInvocationRequest {
                host: discovery.host,
                request_id: PeerRequestId::new(request_id).map_err(|error| invalid(&error))?,
                catalog_generation: discovery.catalog.generation,
                catalog_digest: discovery.catalog.digest,
                selection,
                request: InvocationRequest::new(
                    InvocationId::new(request_id).map_err(|error| invalid(&error))?,
                    entry.descriptor.identity().clone(),
                    operation,
                    entry.descriptor.provider_profile().cloned(),
                    key,
                    inputs,
                    Default::default(),
                )
                .map_err(|error| invalid(&error))?,
                limits: discovery.limits,
                deadline_unix_ms,
            };
            request
                .selection
                .validate_request(&request.request)
                .map_err(|error| invalid(&error))?;
            let bytes = serde_json::to_vec_pretty(&request).map_err(|error| invalid(&error))?;
            session.write_exact_document(Some(output), &bytes)?;
            session.output(
                "invocation.prepare",
                &serde_json::json!({
                    "request_id": request.request_id, "host": request.host,
                    "deadline_unix_ms": deadline_unix_ms, "output": output,
                }),
            )
        }
        InvocationCommand::Submit { file } => {
            let value = session
                .read_json(
                    file,
                    milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                    "direct invocation",
                )
                .await?;
            let request: DirectInvocationRequest = serde_json::from_value(value)
                .map_err(|error| CliError::Invalid(error.to_string()))?;
            let accepted = client.invoke(&request).await?;
            if matches!(
                accepted,
                milkdrift_peer_protocol::InvocationAcceptance::Rejected { .. }
            ) {
                return Err(CliError::InvocationFailed(Box::new(
                    serde_json::to_value(accepted)
                        .map_err(|error| CliError::Internal(error.to_string()))?,
                )));
            }
            session.output("invocation.submit", &accepted)
        }
        InvocationCommand::Lookup { request } => {
            let request = PeerRequestId::new(request)
                .map_err(|error| CliError::Invalid(error.to_string()))?;
            session.output(
                "invocation.lookup",
                &client.invocation_lookup(&request).await?,
            )
        }
        InvocationCommand::Show { execution } => session.output(
            "invocation.show",
            &client.invocation(&execution_id(execution)?).await?,
        ),
        InvocationCommand::Observations {
            execution,
            after,
            limit,
        } => session.output(
            "invocation.observations",
            &client
                .invocation_observations(&execution_id(execution)?, *after, *limit)
                .await?,
        ),
        InvocationCommand::Wait { execution } => {
            let execution = execution_id(execution)?;
            let mut after = 0;
            loop {
                let page = client
                    .invocation_observations(&execution, after, 128)
                    .await?;
                after = page.next_sequence;
                if page.closed {
                    let terminal = page
                        .observations
                        .iter()
                        .find_map(|observation| observation.event.kind().terminal())
                        .or_else(|| match &page.history {
                            milkdrift_peer_protocol::ObservationHistory::Archived { summary } => {
                                summary
                                    .final_observation
                                    .as_ref()
                                    .and_then(|observation| observation.event.kind().terminal())
                            }
                            milkdrift_peer_protocol::ObservationHistory::Hot => None,
                        });
                    if !terminal.is_some_and(|terminal| {
                        terminal.status() == milkdrift_capability::TerminalStatus::Success
                    }) {
                        return Err(CliError::InvocationFailed(Box::new(
                            serde_json::to_value(page)
                                .map_err(|error| CliError::Internal(error.to_string()))?,
                        )));
                    }
                    return session.output("invocation.wait", &page);
                }
                if !page.observations.is_empty() {
                    if session.cli().json {
                        println!(
                            "{}",
                            crate::output::encode(
                                "invocation.wait",
                                None,
                                "observing",
                                serde_json::to_value(&page)
                                    .map_err(|error| CliError::Internal(error.to_string()))?,
                                serde_json::Value::Null,
                                false
                            )?
                        );
                    } else {
                        session.output("invocation.wait", &page)?;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
        InvocationCommand::Cancel {
            execution,
            request_id,
            sequence,
        } => {
            let request = PeerCancellationRequest {
                execution: execution_id(execution)?,
                request_id: PeerRequestId::new(request_id)
                    .map_err(|error| CliError::Invalid(error.to_string()))?,
                sequence: *sequence,
                reason: session.cli().reason.clone(),
            };
            session.output(
                "invocation.cancel",
                &client.cancel_invocation(&request).await?,
            )
        }
    }
}

fn execution_id(value: &str) -> Result<PeerExecutionId, CliError> {
    PeerExecutionId::new(value).map_err(|error| CliError::Invalid(error.to_string()))
}
