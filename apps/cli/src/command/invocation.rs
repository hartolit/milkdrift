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
            let outcome = async {
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
            .await;
            file.finish(outcome)
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
            input,
            output,
        } => {
            use milkdrift_capability::{
                CapabilityId, InputReference, InvocationValueReference, OperationId, PeerId,
            };
            let invalid = |error: &dyn std::fmt::Display| CliError::Invalid(error.to_string());
            let mut destination = crate::output::PendingFile::create(output)?;
            let outcome = async {
                let mut inputs: Vec<InputReference> = match inputs {
                    Some(inputs) => serde_json::from_value(
                        session
                            .read_json(
                                inputs,
                                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
                                "invocation inputs",
                            )
                            .await?,
                    )
                    .map_err(|error| invalid(&error))?,
                    None => Vec::new(),
                };
                let mut names: std::collections::BTreeSet<String> =
                    inputs.iter().map(|input| input.name().to_owned()).collect();
                if names.len() != inputs.len() {
                    return Err(CliError::Invalid("input names must be distinct".into()));
                }
                for (name, artifact) in super::input::upload_text(
                    session,
                    host,
                    request_id,
                    "invocation-input",
                    input,
                    &mut names,
                )
                .await?
                {
                    let reference = milkdrift_capability::ArtifactReference::new(
                        artifact.artifact_id,
                        artifact.digest,
                        Some(artifact.content_type),
                        Some(artifact.size),
                    )
                    .map_err(|error| invalid(&error))?;
                    inputs.push(
                        InputReference::new(name, InvocationValueReference::Artifact { reference })
                            .map_err(|error| invalid(&error))?,
                    );
                }
                let request = client
                    .prepare_invocation(&milkdrift_peer_protocol::DirectInvocationDraft {
                        host: PeerId::new(host).map_err(|error| invalid(&error))?,
                        request_id: PeerRequestId::new(request_id)
                            .map_err(|error| invalid(&error))?,
                        capability: CapabilityId::new(capability)
                            .map_err(|error| invalid(&error))?,
                        operation: OperationId::new(operation).map_err(|error| invalid(&error))?,
                        inputs,
                        limits: None,
                    })
                    .await?;
                let bytes = serde_json::to_vec_pretty(&request).map_err(|error| invalid(&error))?;
                use std::io::Write as _;
                destination
                    .write_all(&bytes)
                    .and_then(|()| destination.commit())
                    .map_err(|error| invalid(&error))?;
                session.output(
                    "invocation.prepare",
                    &serde_json::json!({
                        "request_id": request.request_id, "host": request.host,
                        "deadline_unix_ms": request.deadline_unix_ms, "output": output,
                    }),
                )
            }
            .await;
            destination.finish(outcome)
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
                        crate::output::line(format_args!(
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
                        ))?;
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
