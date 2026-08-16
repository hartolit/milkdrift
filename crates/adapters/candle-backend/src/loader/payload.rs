//! Bounded source-payload staging and CPU tensor construction.

use candle_core::{DType, Device, Tensor};
use domain_contracts::{BackendId, LoadError};

use crate::failure::{
    CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW, CODE_PAYLOAD_READ, CODE_TENSOR_MATERIALIZE,
};

use super::manifest::{SourceTensorDType, TensorShape};
use super::{
    VERIFICATION_BUFFER_BYTES, host_memory_failure, invalid_model_failure, map_candle_load_error,
    unsupported_scalar,
};

/// One allocation whose selected sub-slice satisfies the source dtype's alignment.
#[derive(Debug)]
pub(super) struct AlignedPayload {
    bytes: Vec<u8>,
    start: usize,
    end: usize,
}

impl AlignedPayload {
    pub(super) fn allocate(
        backend: BackendId,
        source_dtype: SourceTensorDType,
        source_bytes: u64,
    ) -> Result<Self, LoadError> {
        let alignment = source_dtype
            .alignment()
            .ok_or_else(|| unsupported_scalar(backend))?;
        let alignment_padding = alignment
            .checked_sub(1)
            .ok_or_else(|| numeric_error(backend))?;
        let allocation_bytes = source_bytes
            .checked_add(alignment_padding)
            .ok_or_else(|| numeric_error(backend))?;
        let allocation_length =
            usize::try_from(allocation_bytes).map_err(|_| numeric_error(backend))?;
        let source_length = usize::try_from(source_bytes).map_err(|_| numeric_error(backend))?;
        let alignment = usize::try_from(alignment).map_err(|_| numeric_error(backend))?;

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(allocation_length)
            .map_err(|_| host_memory_failure(backend, CODE_TENSOR_MATERIALIZE))?;
        bytes.resize(allocation_length, 0);
        let start = bytes.as_ptr().align_offset(alignment);
        let end = start
            .checked_add(source_length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid_model_failure(backend, CODE_TENSOR_MATERIALIZE))?;
        Ok(Self { bytes, start, end })
    }

    pub(super) fn as_mut_slice(&mut self, backend: BackendId) -> Result<&mut [u8], LoadError> {
        self.bytes
            .get_mut(self.start..self.end)
            .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))
    }

    pub(super) fn as_slice(&self, backend: BackendId) -> Result<&[u8], LoadError> {
        self.bytes
            .get(self.start..self.end)
            .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))
    }
}

pub(super) fn source_tensor(
    backend: BackendId,
    payload: &AlignedPayload,
    source_dtype: DType,
    shape: TensorShape,
) -> Result<Tensor, LoadError> {
    Tensor::from_raw_buffer(
        payload.as_slice(backend)?,
        source_dtype,
        shape.as_slice(),
        &Device::Cpu,
    )
    .map_err(|error| map_candle_load_error(backend, &Device::Cpu, &error, CODE_TENSOR_MATERIALIZE))
}

pub(super) fn verification_buffer(backend: BackendId) -> Result<Vec<u8>, LoadError> {
    let capacity = VERIFICATION_BUFFER_BYTES
        .checked_to_usize()
        .ok_or_else(|| numeric_error(backend))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    buffer.resize(capacity, 0);
    Ok(buffer)
}

const fn numeric_error(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW)
}
