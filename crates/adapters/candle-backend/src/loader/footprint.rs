//! Exact required-tensor load phases and sequence cache-rate calculation.

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendId, DeviceKind, LoadError, MemoryBudget, MemoryFootprint, MemoryKind,
};

use super::manifest::InspectedShard;
use super::math::{execution_dtype_bytes, numeric_overflow};
use super::transfer_plan::TransferPlan;
use super::{VERIFICATION_BUFFER_BYTES_U64, unsupported_scalar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CalculatedFootprints {
    pub(super) final_footprint: MemoryFootprint,
    pub(super) loading_peak_footprint: MemoryFootprint,
}

pub(super) fn calculate(
    backend: BackendId,
    shards: &[InspectedShard],
    device_kind: DeviceKind,
    execution_dtype: DType,
    transfer_plan: Option<&TransferPlan>,
    transfer_owner_metadata_bytes: u64,
) -> Result<CalculatedFootprints, LoadError> {
    let execution_width =
        execution_dtype_bytes(execution_dtype).ok_or_else(|| unsupported_scalar(backend))?;
    let mut required_execution_bytes = 0_u64;
    let mut host_peak = VERIFICATION_BUFFER_BYTES_U64;

    for tensor in shards
        .iter()
        .flat_map(|shard| shard.tensors.iter())
        .filter(|tensor| tensor.required)
    {
        let execution_bytes = tensor
            .element_count
            .checked_mul(execution_width)
            .ok_or_else(|| numeric_overflow(backend))?;
        let alignment = tensor
            .source_dtype
            .alignment()
            .ok_or_else(|| unsupported_scalar(backend))?;
        let alignment_padding = alignment
            .checked_sub(1)
            .ok_or_else(|| numeric_overflow(backend))?;
        let aligned_staging = tensor
            .source_bytes
            .checked_add(alignment_padding)
            .ok_or_else(|| numeric_overflow(backend))?;
        let source_dtype = tensor
            .source_dtype
            .executable_dtype()
            .ok_or_else(|| unsupported_scalar(backend))?;

        match device_kind {
            DeviceKind::Cpu => {
                let raw_peak = required_execution_bytes
                    .checked_add(aligned_staging)
                    .and_then(|bytes| bytes.checked_add(tensor.source_bytes))
                    .and_then(|bytes| bytes.checked_add(VERIFICATION_BUFFER_BYTES_U64))
                    .ok_or_else(|| numeric_overflow(backend))?;
                host_peak = host_peak.max(raw_peak);
                if source_dtype != execution_dtype {
                    let cast_peak = required_execution_bytes
                        .checked_add(tensor.source_bytes)
                        .and_then(|bytes| bytes.checked_add(execution_bytes))
                        .and_then(|bytes| bytes.checked_add(VERIFICATION_BUFFER_BYTES_U64))
                        .ok_or_else(|| numeric_overflow(backend))?;
                    host_peak = host_peak.max(cast_peak);
                }
            }
            DeviceKind::Cuda => {}
            _ => return Err(LoadError::InvalidConfiguration),
        }
        required_execution_bytes = required_execution_bytes
            .checked_add(execution_bytes)
            .ok_or_else(|| numeric_overflow(backend))?;
    }

    match device_kind {
        DeviceKind::Cpu => cpu_footprints(backend, required_execution_bytes, host_peak),
        DeviceKind::Cuda => {
            let transfer_plan = transfer_plan.ok_or(LoadError::InvalidConfiguration)?;
            if transfer_plan.total_execution_bytes() != required_execution_bytes {
                return Err(numeric_overflow(backend));
            }
            let host_peak = VERIFICATION_BUFFER_BYTES_U64
                .checked_add(transfer_plan.maximum_host_staging_bytes())
                .and_then(|bytes| bytes.checked_add(transfer_plan.metadata_bytes()))
                .and_then(|bytes| bytes.checked_add(transfer_owner_metadata_bytes))
                .ok_or_else(|| numeric_overflow(backend))?;
            Ok(cuda_footprints(required_execution_bytes, host_peak))
        }
        _ => Err(LoadError::InvalidConfiguration),
    }
}

fn cpu_footprints(
    backend: BackendId,
    required_execution_bytes: u64,
    host_peak: u64,
) -> Result<CalculatedFootprints, LoadError> {
    let host_peak = host_peak.max(required_execution_bytes);
    let host_working_bytes = host_peak
        .checked_sub(required_execution_bytes)
        .ok_or_else(|| numeric_overflow(backend))?;
    Ok(CalculatedFootprints {
        final_footprint: MemoryFootprint {
            host_weight_bytes: required_execution_bytes,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
        },
        loading_peak_footprint: MemoryFootprint {
            host_weight_bytes: required_execution_bytes,
            device_weight_bytes: 0,
            host_working_bytes,
            device_working_bytes: 0,
        },
    })
}

const fn cuda_footprints(required_execution_bytes: u64, host_peak: u64) -> CalculatedFootprints {
    CalculatedFootprints {
        final_footprint: MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: required_execution_bytes,
            host_working_bytes: 0,
            device_working_bytes: 0,
        },
        loading_peak_footprint: MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: required_execution_bytes,
            host_working_bytes: host_peak,
            device_working_bytes: 0,
        },
    }
}

pub(super) fn validate_memory_plan(
    backend: BackendId,
    footprint: MemoryFootprint,
    budget: MemoryBudget,
    currently_available_device_bytes: Option<u64>,
) -> Result<(), LoadError> {
    let required_host = footprint
        .checked_host_bytes()
        .ok_or_else(|| numeric_overflow(backend))?;
    if required_host > budget.host_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes,
        });
    }
    let required_device = footprint
        .checked_device_bytes()
        .ok_or_else(|| numeric_overflow(backend))?;
    if required_device > budget.device_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes,
        });
    }
    if let Some(available) = currently_available_device_bytes
        && required_device > available
    {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: available,
        });
    }
    Ok(())
}

pub(super) fn sequence_cache_bytes_per_token(
    backend: BackendId,
    config: &Config,
    execution_dtype: DType,
) -> Result<u64, LoadError> {
    let scalar_bytes =
        execution_dtype_bytes(execution_dtype).ok_or_else(|| unsupported_scalar(backend))?;
    let head_dimension = config
        .hidden_size
        .checked_div(config.num_attention_heads)
        .ok_or_else(|| numeric_overflow(backend))?;
    let factors = [
        u64::try_from(config.num_hidden_layers),
        Ok(2_u64),
        u64::try_from(config.num_key_value_heads),
        u64::try_from(head_dimension),
        Ok(scalar_bytes),
    ];
    factors.into_iter().try_fold(1_u64, |total, factor| {
        let factor = factor.map_err(|_| numeric_overflow(backend))?;
        total
            .checked_mul(factor)
            .ok_or_else(|| numeric_overflow(backend))
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use candle_core::DType;
    use candle_transformers::models::llama::Config;
    use domain_contracts::{
        BackendId, DeviceKind, LoadError, MemoryBudget, MemoryFootprint, MemoryKind,
    };

    use super::{
        CalculatedFootprints, calculate, sequence_cache_bytes_per_token, validate_memory_plan,
    };
    use crate::loader::identity::{ContentIdentityEstablishment, EstablishedContentIdentity};
    use crate::loader::manifest::{
        InspectedShard, InspectedTensor, SourceTensorDType, TensorShape,
    };
    use crate::loader::transfer_batch::TransferBatchOwner;
    use crate::loader::transfer_plan::{MAXIMUM_BATCH_ENTRIES, TransferPlan};

    #[test]
    fn exact_cpu_and_cuda_formulas_use_required_tensors_only() -> Result<(), String> {
        let shard = calculation_shard(vec![
            calculation_tensor("required.f16", SourceTensorDType::F16, 10, true)?,
            calculation_tensor("required.f32", SourceTensorDType::F32, 4, true)?,
            calculation_tensor("ignored.f64", SourceTensorDType::F64, 10_000, false)?,
        ])?;
        let config = test_config();
        let backend = BackendId::new(1);

        assert_eq!(
            calculate(
                backend,
                std::slice::from_ref(&shard),
                DeviceKind::Cpu,
                DType::F16,
                None,
                0,
            )
            .map_err(|error| format!("CPU footprint: {error:?}"))?,
            CalculatedFootprints {
                final_footprint: MemoryFootprint {
                    host_weight_bytes: 28,
                    device_weight_bytes: 0,
                    host_working_bytes: 0,
                    device_working_bytes: 0,
                },
                loading_peak_footprint: MemoryFootprint {
                    host_weight_bytes: 28,
                    device_weight_bytes: 0,
                    host_working_bytes: 65_563,
                    device_working_bytes: 0,
                },
            }
        );
        let transfer_plan = TransferPlan::build(backend, std::slice::from_ref(&shard), DType::F16)
            .map_err(|error| format!("transfer plan: {error:?}"))?;
        let transfer_owner = TransferBatchOwner::allocate(backend, MAXIMUM_BATCH_ENTRIES)
            .map_err(|error| format!("transfer owner: {error:?}"))?;
        let owner_metadata = transfer_owner
            .metadata_bytes(backend)
            .map_err(|error| format!("owner metadata: {error:?}"))?;
        let expected_cuda_host = 65_536_u64
            .checked_add(transfer_plan.maximum_host_staging_bytes())
            .and_then(|bytes| bytes.checked_add(transfer_plan.metadata_bytes()))
            .and_then(|bytes| bytes.checked_add(owner_metadata))
            .ok_or_else(|| "expected CUDA host footprint overflow".to_owned())?;
        assert_eq!(
            calculate(
                backend,
                std::slice::from_ref(&shard),
                DeviceKind::Cuda,
                DType::F16,
                Some(&transfer_plan),
                owner_metadata,
            )
            .map_err(|error| format!("CUDA footprint: {error:?}"))?,
            CalculatedFootprints {
                final_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 28,
                    host_working_bytes: 0,
                    device_working_bytes: 0,
                },
                loading_peak_footprint: MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 28,
                    host_working_bytes: expected_cuda_host,
                    device_working_bytes: 0,
                },
            }
        );
        assert_eq!(
            sequence_cache_bytes_per_token(backend, &config, DType::F16)
                .map_err(|error| format!("F16 cache rate: {error:?}"))?,
            32
        );
        assert_eq!(
            sequence_cache_bytes_per_token(backend, &config, DType::F32)
                .map_err(|error| format!("F32 cache rate: {error:?}"))?,
            64
        );
        Ok(())
    }

    #[test]
    fn loading_budget_and_current_device_availability_are_exact() -> Result<(), String> {
        let backend = BackendId::new(1);
        let footprint = MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 100,
            host_working_bytes: 50,
            device_working_bytes: 0,
        };
        let exact = MemoryBudget {
            host_bytes: 50,
            device_bytes: 100,
        };
        validate_memory_plan(backend, footprint, exact, Some(100))
            .map_err(|error| format!("exact budget rejected: {error:?}"))?;

        for (budget, available, kind, required, observed_available) in [
            (
                MemoryBudget {
                    host_bytes: 49,
                    device_bytes: 100,
                },
                Some(100),
                MemoryKind::Host,
                50,
                49,
            ),
            (
                MemoryBudget {
                    host_bytes: 50,
                    device_bytes: 99,
                },
                Some(100),
                MemoryKind::Device,
                100,
                99,
            ),
            (exact, Some(99), MemoryKind::Device, 100, 99),
        ] {
            let error = validate_memory_plan(backend, footprint, budget, available)
                .err()
                .ok_or_else(|| "one-byte-low budget unexpectedly passed".to_owned())?;
            assert_eq!(
                error,
                LoadError::InsufficientMemory {
                    kind,
                    required_bytes: required,
                    available_bytes: observed_available,
                }
            );
        }
        Ok(())
    }

    fn calculation_shard(tensors: Vec<InspectedTensor>) -> Result<InspectedShard, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let file = File::open(executable).map_err(|error| error.to_string())?;
        Ok(InspectedShard {
            file,
            file_length: 0,
            data_start: 0,
            prefix_header_sha256: [0; 32],
            source_expected_content: None,
            established_content_identity: Some(EstablishedContentIdentity {
                byte_length: 0,
                sha256: [0; 32],
                establishment: ContentIdentityEstablishment::LocallyEstablishedBaseline,
            }),
            tensors,
        })
    }

    fn calculation_tensor(
        name: &str,
        source_dtype: SourceTensorDType,
        element_count: u64,
        required: bool,
    ) -> Result<InspectedTensor, String> {
        let dimension = usize::try_from(element_count).map_err(|error| error.to_string())?;
        let bits = element_count
            .checked_mul(source_dtype.bits_per_element())
            .ok_or_else(|| "source bits overflow".to_owned())?;
        let source_bytes = bits
            .checked_div(8)
            .ok_or_else(|| "source bytes overflow".to_owned())?;
        let shape = TensorShape::from_slice(&[dimension])
            .ok_or_else(|| "test shape overflow".to_owned())?;
        Ok(InspectedTensor {
            name: name.to_owned(),
            source_dtype,
            shape,
            data_start: 0,
            source_bytes,
            element_count,
            required,
        })
    }

    fn test_config() -> Config {
        Config {
            hidden_size: 8,
            intermediate_size: 16,
            vocab_size: 16,
            num_hidden_layers: 1,
            num_attention_heads: 2,
            num_key_value_heads: 2,
            use_flash_attn: false,
            rms_norm_eps: 1e-5,
            rope_theta: 10_000.0,
            bos_token_id: None,
            eos_token_id: None,
            rope_scaling: None,
            max_position_embeddings: 16,
            tie_word_embeddings: false,
        }
    }
}
