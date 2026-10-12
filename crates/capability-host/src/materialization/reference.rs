//! Resolve selected immutable artifact identities without granting content access.
use super::{
    AdapterExecutionContext, CapabilityArtifactReference, InputReference, InvocationDataError,
    InvocationValueReference, StoreInvocationDataAccess,
};
impl StoreInvocationDataAccess {
    pub(super) fn resolve_selected_artifact(
        context: &AdapterExecutionContext,
        input: &InputReference,
    ) -> Result<CapabilityArtifactReference, InvocationDataError> {
        if let Some(selection) = context.direct_selection() {
            selection.require_input(input)?;
        }
        match input.value() {
            InvocationValueReference::Artifact { reference } => Ok(reference.clone()),
            InvocationValueReference::WorkspaceValue { .. }
            | InvocationValueReference::Inline { .. } => Err(InvocationDataError::Rejected(
                "input must be an immutable artifact reference".to_owned(),
            )),
        }
    }
}
