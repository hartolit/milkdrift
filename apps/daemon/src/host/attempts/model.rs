//! Expose bounded generation choices without publishing prompts or provider metadata.

use milkdrift_blueprint::{BindingSource, NodeId, PortId};
use milkdrift_control_protocol::ModelGenerationRead;
use milkdrift_model::{MODEL_TASK_INPUT_NAME, ModelResponseDocument, ModelTaskRequestDocument};
use milkdrift_persistence::RevisionStore;

use super::{LocatedAttempt, Owner};
use crate::host::{
    ActorSession, PublicFailure,
    read_model::{internal, parse_revision_id, public_persistence, snake_debug},
};

impl Owner {
    pub(super) fn attach_model_generation(
        &mut self,
        session: &ActorSession,
        located: &mut LocatedAttempt,
    ) -> Result<(), PublicFailure> {
        if located
            .value
            .operation_contract
            .as_ref()
            .is_none_or(|contract| contract.operation != "model.generate")
        {
            return Ok(());
        }
        let mut diagnostics = ModelGenerationRead {
            requested_output_units: None,
            reasoning_effort: None,
            reasoning_maximum_units: None,
            finish_reason: None,
        };
        let revision = self
            .store
            .revision(&parse_revision_id(&located.revision_id)?)
            .map_err(public_persistence)?;
        if let Some(revision) = revision
            && self.revision(session, &located.revision_id).is_ok()
            && let Some(node) = revision
                .semantic()
                .nodes()
                .get(&NodeId::new(&located.node_id).map_err(|_| internal())?)
            && let Some(input) = node
                .data_inputs()
                .get(&PortId::new(MODEL_TASK_INPUT_NAME).map_err(|_| internal())?)
            && let Some(BindingSource::Literal { value }) = input.binding()
            && let Ok(document) = ModelTaskRequestDocument::from_json(
                &serde_json::to_vec(value.value()).map_err(|_| internal())?,
            )
        {
            diagnostics.requested_output_units = Some(document.body().maximum_output_units());
            diagnostics.reasoning_effort = document
                .body()
                .reasoning()
                .and_then(|reasoning| reasoning.effort)
                .map(|effort| snake_debug(&effort));
            diagnostics.reasoning_maximum_units = document
                .body()
                .reasoning()
                .and_then(|reasoning| reasoning.maximum_units);
        }
        if let Some(output) = located
            .value
            .outputs
            .iter()
            .find(|output| output.name == "model_response")
            && output.artifact.size <= milkdrift_model::MAX_MODEL_DOCUMENT_BYTES as u64
        {
            let mut bytes = Vec::new();
            while (bytes.len() as u64) < output.artifact.size {
                let chunk = match self.artifact_range(
                    session,
                    &output.artifact.artifact_id,
                    bytes.len() as u64,
                    262_144,
                    "model-generation-inspection",
                ) {
                    Ok(chunk) => chunk,
                    Err(error)
                        if matches!(
                            error.code,
                            milkdrift_control_protocol::ErrorCode::Unauthorized
                                | milkdrift_control_protocol::ErrorCode::NotFound
                        ) =>
                    {
                        bytes.clear();
                        break;
                    }
                    Err(error) => return Err(error),
                };
                if chunk.bytes.is_empty() || chunk.metadata.digest != output.artifact.digest {
                    return Err(internal());
                }
                bytes.extend(chunk.bytes);
                if chunk.end {
                    break;
                }
            }
            if !bytes.is_empty() {
                let document = ModelResponseDocument::from_json(&bytes).map_err(|_| internal())?;
                diagnostics.finish_reason = Some(snake_debug(&document.body().finish_reason()));
            }
        }
        located.value.model_generation = Some(diagnostics);
        Ok(())
    }
}
