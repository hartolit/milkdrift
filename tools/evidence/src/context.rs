//! Measure explicit input selection and isolated materialization through production host ports.
use crate::{EvidenceResult, ScenarioMeasurement};
use milkdrift_capability::{
    BoundedJson, CapabilityId, InputReference, InvocationId, InvocationRequest,
    InvocationValueReference, OperationId,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, DirectInputSelection, InputMaterialization, InvocationDataAccess,
    MaterializationLimits, StoreInvocationDataAccess,
};
use milkdrift_persistence::ArtifactReadAuthority;
use std::{collections::BTreeMap, sync::Arc};

fn request() -> EvidenceResult<InvocationRequest> {
    let inputs = (0..64)
        .map(|index| {
            InputReference::new(
                format!("input-{index:04}"),
                InvocationValueReference::Inline {
                    value: BoundedJson::new(
                        serde_json::json!({"index":index,"text":"x".repeat(1024)}),
                    )?,
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(InvocationRequest::new(
        InvocationId::new("evidence-explicit-inputs")?,
        CapabilityId::new("evidence-process")?,
        OperationId::new("process.execute")?,
        None,
        None,
        inputs,
        BTreeMap::new(),
    )?)
}

/// Freeze 64 explicit named inputs and verify their exact request binding.
/// # Errors
/// Returns invocation/selection bound or canonical identity failures.
pub fn explicit_input_selection() -> EvidenceResult<ScenarioMeasurement> {
    let request = request()?;
    let selection = DirectInputSelection::new(&request, 1_048_576)?;
    selection.validate_request(&request)?;
    let bytes = selection.canonical_json().as_bytes();
    Ok(ScenarioMeasurement::new(
        "context/explicit_input_selection_64",
        64,
        u64::try_from(bytes.len())?,
        bytes,
    ))
}

/// Materialize 64 selected values into isolated files and inspect the actual captured bytes.
/// # Errors
/// Returns storage, selection, filesystem, budget or content mismatch failures.
pub fn explicit_input_materialization() -> EvidenceResult<ScenarioMeasurement> {
    let request = request()?;
    let context = AdapterExecutionContext::direct(DirectInputSelection::new(&request, 1_048_576)?);
    let root = tempfile::tempdir()?;
    let store = Arc::new(milkdrift_redb_store::RedbStore::open(
        root.path().join("store"),
    )?);
    let data = StoreInvocationDataAccess::new(
        store,
        root.path().join("scratch"),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let selections = request
        .inputs()
        .iter()
        .map(|input| InputMaterialization::new(input.name(), format!("{}.json", input.name())))
        .collect::<Result<Vec<_>, _>>()?;
    let limits = MaterializationLimits {
        max_files: 64,
        max_file_bytes: 16384,
        max_total_bytes: 1_048_576,
        max_path_bytes: 256,
        max_directory_depth: 4,
        chunk_bytes: 16384,
    };
    let materialized = data.materialize(&context, &request, &selections, limits)?;
    let mut evidence = Vec::new();
    for input in request.inputs() {
        let path = materialized
            .input_path(input.name())
            .ok_or("selected input path absent")?;
        let bytes = std::fs::read(path)?;
        if bytes != data.read_input_bytes(&context, input, limits)? {
            return Err("materialized bytes differ from selection".into());
        }
        evidence.extend(bytes);
    }
    Ok(ScenarioMeasurement::new(
        "context/explicit_input_materialization_64",
        64,
        u64::try_from(evidence.len())?,
        &evidence,
    ))
}
