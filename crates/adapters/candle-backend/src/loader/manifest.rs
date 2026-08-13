//! Weight-shard I/O, immutable identity evidence, and retained tensor inventory.

use std::fs::File;
use std::io::Read;
use std::mem::size_of;

use domain_contracts::{BackendId, CapacityExhausted, CapacityResource, LoadError, ScalarTypeSet};
use sha2::{Digest, Sha256};

use crate::failure::{
    CODE_HEADER_ALLOCATION, CODE_HEADER_BOUNDS, CODE_HEADER_DECODE, CODE_HEADER_LIMIT,
    CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW, CODE_SOURCE_IDENTITY_LENGTH,
    CODE_WEIGHT_METADATA,
};
use crate::source::{CandleExpectedContentIdentity, CandleWeightShard};

use super::safetensors::{InspectionBudget, parse_header, try_reserve_vec};
use super::{host_memory_failure, invalid_model_failure};

pub(super) use super::safetensors::{
    InspectedShard, InspectedTensor, InspectionLimits, SourceTensorDType, TensorShape,
};

const SAFETENSORS_PREFIX_BYTES: u64 = 8;
const SAFETENSORS_PREFIX_BYTES_USIZE: usize = 8;

#[derive(Debug)]
struct PreopenedShard {
    file: File,
    file_length: u64,
    prefix: [u8; SAFETENSORS_PREFIX_BYTES_USIZE],
    header_length: usize,
    data_start: u64,
    source_expected_content: Option<CandleExpectedContentIdentity>,
}

pub(super) fn inspect_weight_shards(
    backend: BackendId,
    selected: &[CandleWeightShard],
    limits: &InspectionLimits,
) -> Result<Vec<InspectedShard>, LoadError> {
    validate_shard_count(selected.len(), limits)?;
    let preopened = preopen_shards(backend, selected, limits)?;
    parse_preopened_shards(backend, preopened, limits)
}

fn validate_shard_count(count: usize, limits: &InspectionLimits) -> Result<(), LoadError> {
    if count <= limits.shards {
        return Ok(());
    }
    let required = u64::try_from(count).unwrap_or(u64::MAX);
    let available = u64::try_from(limits.shards).unwrap_or(u64::MAX);
    Err(LoadError::CapacityExhausted(CapacityExhausted::new(
        CapacityResource::BackendScratch,
        required,
        available,
    )))
}

fn preopen_shards(
    backend: BackendId,
    selected: &[CandleWeightShard],
    limits: &InspectionLimits,
) -> Result<Vec<PreopenedShard>, LoadError> {
    let mut preopened = Vec::new();
    try_reserve_vec(&mut preopened, selected.len())
        .map_err(|()| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    let mut aggregate_header_bytes = 0_u64;

    for selected_shard in selected {
        let mut file = File::open(selected_shard.path())
            .map_err(|_| invalid_model_failure(backend, CODE_WEIGHT_METADATA))?;
        let metadata = file
            .metadata()
            .map_err(|_| invalid_model_failure(backend, CODE_WEIGHT_METADATA))?;
        if !metadata.is_file() {
            return Err(invalid_model_failure(backend, CODE_WEIGHT_METADATA));
        }
        let file_length = metadata.len();
        validate_expected_length(backend, selected_shard.expected_content(), file_length)?;

        let mut prefix = [0_u8; SAFETENSORS_PREFIX_BYTES_USIZE];
        file.read_exact(&mut prefix)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        let header_length_u64 = u64::from_le_bytes(prefix);
        if header_length_u64 == 0 || header_length_u64 > limits.per_shard_header_bytes {
            return Err(invalid_model_failure(backend, CODE_HEADER_LIMIT));
        }
        aggregate_header_bytes = aggregate_header_bytes
            .checked_add(header_length_u64)
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        if aggregate_header_bytes > limits.aggregate_header_bytes {
            return Err(invalid_model_failure(backend, CODE_HEADER_LIMIT));
        }
        let data_start = SAFETENSORS_PREFIX_BYTES
            .checked_add(header_length_u64)
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        if data_start > file_length {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        let header_length = usize::try_from(header_length_u64)
            .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        preopened.push(PreopenedShard {
            file,
            file_length,
            prefix,
            header_length,
            data_start,
            source_expected_content: selected_shard.expected_content(),
        });
    }
    Ok(preopened)
}

fn validate_expected_length(
    backend: BackendId,
    expected_content: Option<CandleExpectedContentIdentity>,
    file_length: u64,
) -> Result<(), LoadError> {
    if expected_content.is_some_and(|expected| expected.byte_length() != file_length) {
        Err(invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH))
    } else {
        Ok(())
    }
}

fn parse_preopened_shards(
    backend: BackendId,
    preopened: Vec<PreopenedShard>,
    limits: &InspectionLimits,
) -> Result<Vec<InspectedShard>, LoadError> {
    let mut budget = InspectionBudget::new(limits);
    let mut shards = Vec::new();
    try_reserve_vec(&mut shards, preopened.len())
        .map_err(|()| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;

    for mut shard in preopened {
        budget.add_inventory(backend, size_of::<InspectedShard>())?;
        let header = read_header(backend, &mut shard)?;
        let actual_payload_bytes = shard
            .file_length
            .checked_sub(shard.data_start)
            .ok_or_else(|| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
        let mut tensors = parse_header(backend, header.as_slice(), &mut budget)?;
        validate_tensor_layout(backend, &mut tensors, actual_payload_bytes)?;

        let mut prefix_header_hasher = Sha256::new();
        prefix_header_hasher.update(shard.prefix);
        prefix_header_hasher.update(header.as_slice());
        shards.push(InspectedShard {
            file: shard.file,
            file_length: shard.file_length,
            data_start: shard.data_start,
            prefix_header_sha256: prefix_header_hasher.finalize().into(),
            source_expected_content: shard.source_expected_content,
            established_content_identity: None,
            tensors,
        });
    }
    Ok(shards)
}

fn read_header(backend: BackendId, shard: &mut PreopenedShard) -> Result<Vec<u8>, LoadError> {
    let mut header = Vec::new();
    try_reserve_vec(&mut header, shard.header_length)
        .map_err(|()| host_memory_failure(backend, CODE_HEADER_ALLOCATION))?;
    header.resize(shard.header_length, 0);
    shard
        .file
        .read_exact(&mut header)
        .map_err(|_| invalid_model_failure(backend, CODE_HEADER_BOUNDS))?;
    if header.first().copied() != Some(b'{') {
        return Err(invalid_model_failure(backend, CODE_HEADER_DECODE));
    }
    Ok(header)
}

pub(super) fn observed_scalar_types(shards: &[InspectedShard]) -> ScalarTypeSet {
    let mut observed = ScalarTypeSet::EMPTY;
    for tensor in shards.iter().flat_map(|shard| shard.tensors.iter()) {
        observed.insert(tensor.source_dtype.scalar_type());
    }
    observed
}

fn validate_tensor_layout(
    backend: BackendId,
    tensors: &mut [InspectedTensor],
    actual_payload_bytes: u64,
) -> Result<(), LoadError> {
    tensors.sort_unstable_by(|left, right| {
        left.data_start
            .cmp(&right.data_start)
            .then_with(|| left.source_bytes.cmp(&right.source_bytes))
            .then_with(|| left.name.cmp(&right.name))
    });

    let mut expected_start = 0_u64;
    for tensor in tensors {
        if tensor.data_start != expected_start {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        let end = tensor
            .data_start
            .checked_add(tensor.source_bytes)
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        if end > actual_payload_bytes {
            return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
        }
        expected_start = end;
    }
    if expected_start != actual_payload_bytes {
        return Err(invalid_model_failure(backend, CODE_HEADER_BOUNDS));
    }
    Ok(())
}
