//! Validate adapter-owned task authoring contracts against registered operations.

use crate::AdapterError;
use milkdrift_capability::{OperationId, TaskAuthoringContract};
use std::collections::BTreeMap;

pub(crate) fn validate_contracts(
    contracts: &BTreeMap<OperationId, TaskAuthoringContract>,
    descriptor: &milkdrift_capability::CapabilityDescriptor,
) -> Result<(), AdapterError> {
    if contracts.len() > 256
        || contracts
            .keys()
            .any(|operation| descriptor.operation(operation).is_none())
    {
        return Err(AdapterError::rejected(
            "task authoring metadata names an unavailable operation",
        ));
    }
    for contract in contracts.values() {
        contract
            .validate()
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
    }
    Ok(())
}
