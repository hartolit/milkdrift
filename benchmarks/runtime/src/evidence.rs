//! Benchmark-local translation of public load facts into stable report records.

use application_runtime::{ApplicationDevice, ApplicationScalarType};
use domain_contracts::{
    DeviceKind, ExecutionDevice, LoadPlan, MemoryFootprint, ScalarType, ScalarTypeSet,
};
use inference_runtime::LoadReceipt;

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::report::{
    DeviceIdentity, E0LoadReceiptRecord, ExecutionDeviceRecord, MemoryFootprintRecord,
    PreparedLoadRecord,
};

pub(crate) fn prepared_load_record(plan: &LoadPlan) -> BenchmarkResult<PreparedLoadRecord> {
    validate_prepared_load_plan(plan)?;
    Ok(PreparedLoadRecord {
        configuration_declared_scalar: plan
            .descriptor
            .metadata
            .configuration_declared_scalar_type
            .map(scalar_type_label),
        observed_tensor_scalars: scalar_type_set_labels(
            plan.descriptor.metadata.observed_tensor_scalar_types,
        ),
        planned_execution_scalar: scalar_type_label(plan.execution_scalar_type),
        planned_execution_device: execution_device_record(
            plan.accepted_configuration.execution_device,
        )?,
        exact_final_footprint: footprint_record(plan.final_footprint),
        loading_peak_footprint: footprint_record(plan.loading_peak_footprint),
    })
}

pub(crate) fn e0_load_receipt_record(
    plan: &LoadPlan,
    receipt: &LoadReceipt,
) -> BenchmarkResult<E0LoadReceiptRecord> {
    validate_load_receipt(plan, receipt)?;
    Ok(E0LoadReceiptRecord {
        actual_execution_scalar: scalar_type_label(receipt.execution_scalar_type),
        actual_execution_device: execution_device_record(receipt.execution_device)?,
        reserved_footprint: footprint_record(receipt.reserved_footprint),
    })
}

pub(crate) fn validate_load_receipt(plan: &LoadPlan, receipt: &LoadReceipt) -> BenchmarkResult {
    if receipt.handle != plan.accepted_configuration.handle
        || receipt.execution_device != plan.accepted_configuration.execution_device
        || receipt.execution_scalar_type != plan.execution_scalar_type
        || receipt.descriptor != plan.descriptor
        || receipt.reserved_footprint != plan.final_footprint
    {
        return Err(BenchmarkError::new(
            "E0 load receipt did not match the exact observer preparation and final ownership plan",
        ));
    }
    Ok(())
}

pub(crate) fn validate_prepared_load_plan(plan: &LoadPlan) -> BenchmarkResult {
    let final_footprint = plan.final_footprint;
    let loading_peak = plan.loading_peak_footprint;
    let final_host = final_footprint.checked_host_bytes().ok_or_else(|| {
        BenchmarkError::new("prepared exact final host footprint overflowed its component sum")
    })?;
    let final_device = final_footprint.checked_device_bytes().ok_or_else(|| {
        BenchmarkError::new("prepared exact final device footprint overflowed its component sum")
    })?;
    let loading_host = loading_peak.checked_host_bytes().ok_or_else(|| {
        BenchmarkError::new("prepared loading-peak host footprint overflowed its component sum")
    })?;
    let loading_device = loading_peak.checked_device_bytes().ok_or_else(|| {
        BenchmarkError::new("prepared loading-peak device footprint overflowed its component sum")
    })?;
    let loading_contains_final = loading_peak.contains_components(final_footprint);

    if plan
        .descriptor
        .metadata
        .observed_tensor_scalar_types
        .is_empty()
        || (final_footprint.host_weight_bytes().is_zero()
            && final_footprint.device_weight_bytes().is_zero())
        || loading_host < final_host
        || loading_device < final_device
        || !loading_contains_final
    {
        return Err(BenchmarkError::new(
            "prepared load plan did not contain nonempty observed layout, exact final weights, and a component-wise loading peak",
        ));
    }

    match plan.accepted_configuration.execution_device.kind {
        DeviceKind::Cpu
            if !final_footprint.host_weight_bytes().is_zero()
                && final_footprint.device_weight_bytes().is_zero()
                && final_footprint.device_working_bytes().is_zero() =>
        {
            Ok(())
        }
        DeviceKind::Cuda
            if final_footprint.host_weight_bytes().is_zero()
                && !final_footprint.device_weight_bytes().is_zero() =>
        {
            Ok(())
        }
        DeviceKind::Cpu | DeviceKind::Cuda => Err(BenchmarkError::new(
            "prepared exact final footprint was placed on a different memory domain than its execution device",
        )),
        _ => Err(BenchmarkError::new(
            "runtime evidence supports only explicit CPU or CUDA prepared-load devices",
        )),
    }
}

pub(crate) const fn footprint_record(value: MemoryFootprint) -> MemoryFootprintRecord {
    MemoryFootprintRecord {
        host_weight_bytes: value.host_weight_bytes().as_u64(),
        device_weight_bytes: value.device_weight_bytes().as_u64(),
        host_working_bytes: value.host_working_bytes().as_u64(),
        device_working_bytes: value.device_working_bytes().as_u64(),
    }
}

pub(crate) fn execution_device_record(
    value: ExecutionDevice,
) -> BenchmarkResult<ExecutionDeviceRecord> {
    let kind = match value.kind {
        DeviceKind::Cpu => "cpu",
        DeviceKind::Cuda => "cuda",
        _ => {
            return Err(BenchmarkError::new(
                "runtime evidence supports only explicit CPU or CUDA execution devices",
            ));
        }
    };
    Ok(ExecutionDeviceRecord {
        kind,
        id: value.id.get(),
    })
}

pub(crate) const fn scalar_type_label(value: ScalarType) -> &'static str {
    match value {
        ScalarType::F32 => "F32",
        ScalarType::F16 => "F16",
        ScalarType::Bf16 => "BF16",
        ScalarType::I8 => "I8",
        ScalarType::U8 => "U8",
        ScalarType::Other(_) => "OTHER",
        _ => "UNKNOWN",
    }
}

pub(crate) const fn application_scalar_type_label(value: ApplicationScalarType) -> &'static str {
    match value {
        ApplicationScalarType::F32 => "F32",
        ApplicationScalarType::F16 => "F16",
        ApplicationScalarType::Bf16 => "BF16",
    }
}

pub(crate) const fn application_device_record(value: ApplicationDevice) -> DeviceIdentity {
    match value {
        ApplicationDevice::Cpu => DeviceIdentity {
            kind: "cpu",
            id: 0,
            ordinal: None,
        },
        ApplicationDevice::Cuda { ordinal } => DeviceIdentity {
            kind: "cuda",
            id: ordinal as u64,
            ordinal: Some(ordinal),
        },
    }
}

pub(crate) fn scalar_type_set_labels(value: ScalarTypeSet) -> Vec<&'static str> {
    let categories = [
        ScalarType::F32,
        ScalarType::F16,
        ScalarType::Bf16,
        ScalarType::I8,
        ScalarType::U8,
        ScalarType::Other(0),
    ];
    categories
        .into_iter()
        .filter(|scalar| value.contains(*scalar))
        .map(scalar_type_label)
        .collect()
}

#[cfg(test)]
mod tests {
    use domain_contracts::{ScalarType, ScalarTypeSet};

    use super::scalar_type_set_labels;

    #[test]
    fn observed_scalar_labels_follow_stable_scalar_set_bit_order() {
        let observed = ScalarTypeSet::from_scalar(ScalarType::Bf16)
            .union(ScalarTypeSet::from_scalar(ScalarType::F32));
        assert_eq!(scalar_type_set_labels(observed), ["F32", "BF16"]);
    }
}
