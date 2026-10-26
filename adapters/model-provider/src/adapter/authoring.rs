//! Safe task connector choices from this endpoint's model and publication contract.

use std::collections::BTreeMap;

use milkdrift_capability::OperationId;
use milkdrift_capability::{TaskAuthoringContract, TaskAuthoringPort};
use milkdrift_model::MODEL_GENERATE_OPERATION;

use super::{RESPONSE_MEDIA, STRUCTURED_MEDIA};
use crate::EndpointProfile;

pub(super) fn contract(profile: &EndpointProfile) -> BTreeMap<OperationId, TaskAuthoringContract> {
    let max_bytes = profile.limits().max_response_bytes;
    let outputs = [
        ("model_response", "Complete response", true, RESPONSE_MEDIA),
        (
            "final_text",
            "Answer text (when returned)",
            false,
            "text/plain",
        ),
        (
            "structured_output",
            "Structured response (when returned)",
            false,
            STRUCTURED_MEDIA,
        ),
    ]
    .into_iter()
    .map(|(name, label, required, media_type)| TaskAuthoringPort {
        name: name.into(),
        label: label.into(),
        required,
        accepts_artifacts: true,
        media_type: Some(media_type.into()),
        max_bytes,
    })
    .collect();
    let Ok(operation) = OperationId::new(MODEL_GENERATE_OPERATION) else {
        return BTreeMap::new();
    };
    BTreeMap::from([(
        operation,
        TaskAuthoringContract {
            input_media: milkdrift_capability::TaskInputMedia::TextJsonOrImage,
            label: profile.model().into(),
            inputs: Vec::new(),
            allows_additional_inputs: true,
            outputs,
            default_output: Some("model_response".into()),
        },
    )])
}
