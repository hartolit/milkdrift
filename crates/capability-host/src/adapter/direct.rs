//! Explicit request inputs, without a workflow-history lookup or an implicit filesystem source.
use crate::InvocationDataError;
use milkdrift_capability::{
    ArtifactReference, InputReference, InvocationRequest, InvocationValueReference,
};

/// A frozen direct selection derived from the exact accepted request.
/// Construction checks shape and byte ceilings; the serving owner must authorize every referenced
/// artifact against stored metadata before constructing an adapter context.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectInputSelection {
    request: InvocationRequest,
    artifacts: Vec<ArtifactReference>,
    canonical: String,
}

impl DirectInputSelection {
    /// Selects only supplied inline values and exact immutable artifact references.
    /// Workflow context and workspace-value references are refused, never silently omitted.
    pub fn new(
        request: &InvocationRequest,
        maximum_bytes: u64,
    ) -> Result<Self, InvocationDataError> {
        if request.context_manifest().is_some() {
            return Err(rejected(
                "direct input cannot carry a workflow context manifest",
            ));
        }
        let mut artifacts = Vec::new();
        let mut selected = Vec::new();
        let mut bytes = 0_u64;
        for input in request.inputs() {
            if input.name() == milkdrift_capability::CONTEXT_MANIFEST_INPUT_NAME
                || input
                    .name()
                    .starts_with(milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX)
            {
                return Err(rejected(
                    "direct input cannot use reserved workflow context bindings",
                ));
            }
            let (size, fact) = match input.value() {
                InvocationValueReference::Inline { value } => {
                    let content = serde_json::to_vec(value.value())
                        .map_err(|error| rejected(&error.to_string()))?;
                    let size = u64::try_from(content.len())
                        .map_err(|_| rejected("direct input size overflow"))?;
                    (
                        size,
                        serde_json::json!({"name": input.name(), "type": "inline", "digest": blake3::hash(&content).to_hex().as_str(), "size_bytes": size}),
                    )
                }
                InvocationValueReference::Artifact { reference } => {
                    let size = reference.size_bytes().ok_or_else(|| {
                        rejected("direct artifact input requires an exact byte size")
                    })?;
                    if reference.media_type().is_none() {
                        return Err(rejected(
                            "direct artifact input requires an exact media type",
                        ));
                    }
                    if !artifacts.contains(reference) {
                        artifacts.push(reference.clone());
                    }
                    (
                        size,
                        serde_json::json!({"name": input.name(), "type": "artifact", "reference": reference}),
                    )
                }
                InvocationValueReference::WorkspaceValue { .. } => {
                    return Err(rejected(
                        "direct selection cannot read workflow workspace values",
                    ));
                }
            };
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| rejected("direct input byte accounting overflow"))?;
            if bytes > maximum_bytes {
                return Err(rejected(
                    "direct selection exceeds its input byte allowance",
                ));
            }
            selected.push(fact);
        }
        let canonical = serde_json::to_string(&serde_json::json!({
            "schema_version": 1, "origin": "direct", "policy": "explicit_inputs_only",
            "maximum_bytes": maximum_bytes, "selected_bytes": bytes, "inputs": selected
        }))
        .map_err(|error| rejected(&error.to_string()))?;
        Ok(Self {
            request: request.clone(),
            artifacts,
            canonical,
        })
    }

    /// Canonical bounded selection evidence suitable for a model's selection header.
    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical
    }

    /// Checks the immutable request facts after the serving owner scopes invocation identities.
    pub fn validate_request(&self, request: &InvocationRequest) -> Result<(), InvocationDataError> {
        if self.request.capability() != request.capability()
            || self.request.operation() != request.operation()
            || self.request.provider_profile() != request.provider_profile()
            || self.request.inputs() != request.inputs()
            || self.request.extensions() != request.extensions()
            || request.context_manifest().is_some()
        {
            return Err(rejected("request differs from its frozen direct selection"));
        }
        Ok(())
    }

    pub(crate) fn require_input(&self, input: &InputReference) -> Result<(), InvocationDataError> {
        if !self.request.inputs().contains(input) {
            return Err(rejected("input is outside the frozen direct selection"));
        }
        Ok(())
    }

    /// Refuses nested or substituted references before a data-access implementation reads them.
    pub fn require_artifact(
        &self,
        reference: &ArtifactReference,
    ) -> Result<(), InvocationDataError> {
        if !self.artifacts.contains(reference) {
            return Err(rejected("artifact is outside the frozen direct selection"));
        }
        Ok(())
    }
}

fn rejected(message: &str) -> InvocationDataError {
    InvocationDataError::Rejected(message.to_owned())
}
