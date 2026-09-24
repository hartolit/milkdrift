//! Copy only declared accepted terminal fields into the caller's artifact namespace. Repeating
//! publication uses the data owner's content-bound identity; it never reruns the internal method.
use super::{PublishedWorkflowService, failure, limits, rejected};
use milkdrift_capability::{ArtifactReference, InvocationEvent, InvocationEventKind};
use milkdrift_capability_host::AdapterExecutionContext;
use milkdrift_persistence::{
    PeerExecutionSnapshot,
    published::{PublishedInvocationPlan, PublishedInvocationSource, PublishedMethod},
};
use milkdrift_runtime::{ExecutorError, RunCommandDocument, RunProjection};
use milkdrift_workspace::WorkspaceValue;

impl PublishedWorkflowService {
    fn result_context(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<AdapterExecutionContext, ExecutorError> {
        match &plan.source {
            PublishedInvocationSource::Local { run, .. } => {
                AdapterExecutionContext::published_local_result(
                    plan,
                    &self.runtime.projection(run).map_err(failure)?,
                    self.store.as_ref(),
                )
                .map_err(failure)
            }
            PublishedInvocationSource::Serving { caller, execution } => {
                let Some(PeerExecutionSnapshot::Hot(record)) = self
                    .store
                    .peer_execution(caller, execution)
                    .map_err(failure)?
                else {
                    return Err(rejected("pending public result lost its serving owner"));
                };
                AdapterExecutionContext::published_serving_result(plan, &record).map_err(failure)
            }
        }
    }

    pub(super) fn public_outputs(
        &self,
        method: &PublishedMethod,
        plan: &PublishedInvocationPlan,
        projection: &RunProjection,
        next_sequence: u64,
    ) -> Result<(Option<InvocationEvent>, Vec<ArtifactReference>), ExecutorError> {
        let context = self.result_context(plan)?;
        let terminal = projection
            .terminal()
            .ok_or_else(|| rejected("public result requires internal terminal evidence"))?;
        let create =
            RunCommandDocument::from_json(plan.create_command.as_bytes()).map_err(failure)?;
        let service = self.service_command_request(plan, &create)?;
        let mut results = Vec::new();
        for (index, (name, output)) in method.outputs.iter().enumerate() {
            let reference = terminal
                .outputs()
                .iter()
                .find(|reference| reference.key() == &output.field)
                .ok_or_else(|| {
                    rejected("declared public output is absent from the accepted terminal result")
                })?;
            let mut workspace_read = service.clone();
            workspace_read.operation = milkdrift_authority::AuthorityOperation::ReadWorkspaceValue;
            workspace_read.resources.workspace_scope = Some(reference.scope().scope().clone());
            self.current_decision(workspace_read)?;
            let entry = self
                .store
                .value(reference)
                .map_err(failure)?
                .ok_or_else(|| rejected("accepted terminal output is unavailable"))?;
            let bytes = match entry.value() {
                WorkspaceValue::Json(value) => {
                    if output.media_type != "application/json" {
                        return Err(rejected("JSON result contradicts its published media type"));
                    }
                    serde_json::to_vec(value.value()).map_err(failure)?
                }
                WorkspaceValue::Artifact(reference) => {
                    if reference.media_type().as_str() != output.media_type
                        || reference.size_bytes() > output.maximum_bytes
                    {
                        return Err(rejected(
                            "accepted result exceeds its public content contract",
                        ));
                    }
                    self.authorize_artifact(service.clone(), reference.artifact())?;
                    self.read_accepted_output(plan, reference)?
                }
            };
            let copied = self
                .data
                .publish_bytes(
                    &context,
                    &plan.request,
                    name,
                    &output.media_type,
                    &bytes,
                    limits(output.maximum_bytes),
                )
                .map_err(failure)?;
            if next_sequence == index as u64 + 1 {
                return Ok((
                    Some(
                        InvocationEvent::new(
                            plan.invocation.clone(),
                            next_sequence,
                            InvocationEventKind::Output {
                                name: name.clone(),
                                reference: copied,
                            },
                        )
                        .map_err(failure)?,
                    ),
                    Vec::new(),
                ));
            }
            results.push(copied);
        }
        Ok((None, results))
    }
}

impl PublishedWorkflowService {
    fn read_accepted_output(
        &self,
        plan: &PublishedInvocationPlan,
        reference: &milkdrift_workspace::ArtifactReference,
    ) -> Result<Vec<u8>, ExecutorError> {
        let mut bytes = Vec::new();
        loop {
            let offset = bytes.len() as u64;
            let chunk = self
                .store
                .read_chunk(
                    &milkdrift_persistence::ArtifactReadRequest::new(
                        reference.clone(),
                        offset,
                        65_536,
                        milkdrift_persistence::ArtifactReadAuthority::Authorized {
                            actor: plan.service.actor.clone(),
                            evidence: milkdrift_persistence::EvidenceId::new(format!(
                                "published:{}:{offset}",
                                plan.invocation
                            ))
                            .map_err(failure)?,
                        },
                    )
                    .map_err(failure)?,
                )
                .map_err(failure)?;
            if chunk.offset != offset
                || (chunk.bytes.is_empty() && !chunk.end_of_artifact)
                || offset
                    .checked_add(chunk.bytes.len() as u64)
                    .is_none_or(|size| size > reference.size_bytes())
            {
                return Err(rejected("accepted public result returned an invalid range"));
            }
            bytes.extend_from_slice(&chunk.bytes);
            if chunk.end_of_artifact {
                break;
            }
        }
        if bytes.len() as u64 != reference.size_bytes()
            || milkdrift_workspace::ContentDigest::for_bytes(&bytes) != reference.digest()
        {
            return Err(rejected(
                "accepted public result bytes do not match their immutable reference",
            ));
        }
        Ok(bytes)
    }
}
