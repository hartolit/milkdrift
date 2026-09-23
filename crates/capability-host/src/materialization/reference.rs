//! Resolve selected artifact identities without granting content access.
use super::{
    AdapterExecutionContext, CapabilityArtifactReference, InputReference, InvocationDataError,
    InvocationValueReference, StoreInvocationDataAccess, WorkspaceValue, WorkspaceValueReference,
    capability_artifact_reference,
};
impl StoreInvocationDataAccess {
    pub(super) fn resolve_selected_artifact(
        &self,
        context: &AdapterExecutionContext,
        input: &InputReference,
    ) -> Result<CapabilityArtifactReference, InvocationDataError> {
        if let Some(selection) = context.direct_selection() {
            selection.require_input(input)?;
        }
        match input.value() {
            InvocationValueReference::Artifact { reference } => Ok(reference.clone()),
            InvocationValueReference::WorkspaceValue { identity, version } => {
                let reference: WorkspaceValueReference = serde_json::from_str(identity)
                    .map_err(|e| InvocationDataError::Integrity(e.to_string()))?;
                if version != &reference.version().get().to_string() {
                    return Err(InvocationDataError::Integrity(
                        "contradictory workspace input version".to_owned(),
                    ));
                }
                let entry = self
                    .store
                    .value(&reference)
                    .map_err(|e| InvocationDataError::Integrity(e.to_string()))?
                    .ok_or_else(|| {
                        InvocationDataError::Integrity("selected workspace input absent".to_owned())
                    })?;
                let WorkspaceValue::Artifact(reference) = entry.value() else {
                    return Err(InvocationDataError::Rejected(
                        "selected value is not an immutable artifact".to_owned(),
                    ));
                };
                capability_artifact_reference(reference)
            }
            InvocationValueReference::Inline { .. } => Err(InvocationDataError::Rejected(
                "candidate must be an immutable artifact".to_owned(),
            )),
        }
    }
}
