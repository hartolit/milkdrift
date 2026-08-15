//! Sequential retained-shard reading and whole-file identity verification.

use std::io::{Read, Seek, SeekFrom};

use domain_contracts::{LoadError, LoadFailureStage};
use sha2::{Digest, Sha256};

use crate::failure::{
    CODE_HEADER_IDENTITY_MISMATCH, CODE_NUMERIC_OVERFLOW, CODE_PAYLOAD_READ,
    CODE_SOURCE_IDENTITY_LENGTH, CODE_SOURCE_IDENTITY_MISMATCH, CODE_WEIGHT_METADATA,
    tensor_failure_location,
};

use super::observer::{HashedRange, MaterializationObserver};
use super::payload::{AlignedPayload, verification_buffer};
use super::prepared::{CandleLlamaPreparedLoad, RequiredTensorFacts};
use super::{VERIFICATION_BUFFER_BYTES_U64, invalid_model_failure, with_tensor};

#[cfg(test)]
thread_local! {
    pub(super) static TEST_REQUIRED_PAYLOAD_READ_FAILURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl CandleLlamaPreparedLoad {
    pub(super) fn materialize_shard<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let expected = self
            .shards
            .get(shard_index)
            .and_then(|shard| shard.established_content_identity)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        self.validate_shard_length(shard_index, expected.byte_length)?;
        self.seek_shard_start(shard_index)?;

        let mut hasher = Sha256::new();
        let mut verification_buffer = verification_buffer(self.backend)?;
        self.verify_shard_header(
            shard_index,
            verification_buffer.as_mut_slice(),
            &mut hasher,
            observer,
        )?;
        self.stream_tensor_ranges(
            shard_index,
            verification_buffer.as_mut_slice(),
            &mut hasher,
            observer,
        )?;
        self.verify_shard_eof(shard_index, expected.byte_length)?;
        let observed_sha256: [u8; 32] = hasher.finalize().into();
        if observed_sha256 != expected.sha256 {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_MISMATCH,
            ));
        }
        observer.whole_shard_verified(expected.establishment);
        self.flush_shard_final_batch(shard_index, observer)
    }

    fn verify_shard_header<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        verification_buffer: &mut [u8],
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let shard = self
            .shards
            .get(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let header_bytes = shard.data_start;
        let expected_header = shard.prefix_header_sha256;
        self.read_ignored_range(
            shard_index,
            header_bytes,
            HashedRange::PrefixHeader,
            verification_buffer,
            hasher,
            observer,
        )?;
        let observed_header: [u8; 32] = hasher.clone().finalize().into();
        if observed_header == expected_header {
            Ok(())
        } else {
            Err(invalid_model_failure(
                self.backend,
                CODE_HEADER_IDENTITY_MISMATCH,
            ))
        }
    }

    fn stream_tensor_ranges<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        verification_buffer: &mut [u8],
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let tensor_count = self
            .shards
            .get(shard_index)
            .map(|shard| shard.tensors.len())
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        for tensor_index in 0..tensor_count {
            let required = self
                .shards
                .get(shard_index)
                .and_then(|shard| shard.tensors.get(tensor_index))
                .map(|tensor| tensor.required)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
            if required {
                self.stream_required_tensor(shard_index, tensor_index, hasher, observer)?;
            } else {
                let source_bytes = self
                    .shards
                    .get(shard_index)
                    .and_then(|shard| shard.tensors.get(tensor_index))
                    .map(|tensor| tensor.source_bytes)
                    .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
                self.read_ignored_range(
                    shard_index,
                    source_bytes,
                    HashedRange::IgnoredTensor,
                    verification_buffer,
                    hasher,
                    observer,
                )?;
            }
        }
        Ok(())
    }

    fn stream_required_tensor<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        tensor_index: usize,
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let facts = self.required_tensor_facts(shard_index, tensor_index)?;
        let payload = self.read_required_payload(shard_index, &facts, hasher, observer)?;
        self.materialize_required_tensor(facts, payload, observer)
    }

    fn validate_shard_length(
        &self,
        shard_index: usize,
        expected_length: u64,
    ) -> Result<(), LoadError> {
        let shard = self
            .shards
            .get(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let current_length = shard
            .file
            .metadata()
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?
            .len();
        if current_length == shard.file_length && current_length == expected_length {
            Ok(())
        } else {
            Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ))
        }
    }

    fn seek_shard_start(&mut self, shard_index: usize) -> Result<(), LoadError> {
        self.shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file
            .seek(SeekFrom::Start(0))
            .map(|_| ())
            .map_err(|_| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))
    }

    fn read_ignored_range<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        byte_count: u64,
        range: HashedRange,
        buffer: &mut [u8],
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<(), LoadError> {
        let mut remaining = byte_count;
        while remaining > 0 {
            let chunk_length = usize::try_from(remaining.min(VERIFICATION_BUFFER_BYTES_U64))
                .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
            let chunk = buffer
                .get_mut(..chunk_length)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?;
            self.shards
                .get_mut(shard_index)
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
                .file
                .read_exact(chunk)
                .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?;
            hasher.update(chunk);
            observer.hashed_range(range, chunk_length);
            self.record_verification_only_bytes(chunk_length);
            remaining = remaining
                .checked_sub(
                    u64::try_from(chunk_length)
                        .map_err(|_| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?,
                )
                .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        }
        Ok(())
    }

    fn required_tensor_facts(
        &mut self,
        shard_index: usize,
        tensor_index: usize,
    ) -> Result<RequiredTensorFacts, LoadError> {
        let tensor = self
            .shards
            .get_mut(shard_index)
            .and_then(|shard| shard.tensors.get_mut(tensor_index))
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let location = tensor_failure_location(
            shard_index,
            tensor_index,
            tensor.name.as_str(),
            Some(tensor.source_dtype.scalar_type()),
        )
        .ok_or_else(|| invalid_model_failure(self.backend, CODE_NUMERIC_OVERFLOW))?;
        Ok(RequiredTensorFacts {
            shard_index,
            tensor_index,
            name: std::mem::take(&mut tensor.name),
            source_dtype: tensor.source_dtype,
            shape: tensor.shape,
            source_bytes: tensor.source_bytes,
            location,
        })
    }

    fn read_required_payload<O: MaterializationObserver>(
        &mut self,
        shard_index: usize,
        facts: &RequiredTensorFacts,
        hasher: &mut Sha256,
        observer: &mut O,
    ) -> Result<AlignedPayload, LoadError> {
        let mut payload =
            AlignedPayload::allocate(self.backend, facts.source_dtype, facts.source_bytes)
                .map_err(|error| {
                    with_tensor(error, LoadFailureStage::HostMaterialization, facts.location)
                })?;
        let destination = payload
            .as_mut_slice(self.backend)
            .map_err(|error| with_tensor(error, LoadFailureStage::PayloadRead, facts.location))?;
        #[cfg(test)]
        if TEST_REQUIRED_PAYLOAD_READ_FAILURES.with(|remaining| {
            let value = remaining.get();
            if value == 0 {
                false
            } else {
                remaining.set(value - 1);
                true
            }
        }) {
            return Err(with_tensor(
                invalid_model_failure(self.backend, CODE_PAYLOAD_READ),
                LoadFailureStage::PayloadRead,
                facts.location,
            ));
        }
        self.shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?
            .file
            .read_exact(destination)
            .map_err(|_| {
                with_tensor(
                    invalid_model_failure(self.backend, CODE_PAYLOAD_READ),
                    LoadFailureStage::PayloadRead,
                    facts.location,
                )
            })?;
        let hashed_bytes = destination.len();
        hasher.update(&*destination);
        observer.hashed_range(HashedRange::RequiredTensor, hashed_bytes);
        self.record_required_bytes(hashed_bytes);
        Ok(payload)
    }

    fn verify_shard_eof(
        &mut self,
        shard_index: usize,
        expected_length: u64,
    ) -> Result<(), LoadError> {
        let shard = self
            .shards
            .get_mut(shard_index)
            .ok_or_else(|| invalid_model_failure(self.backend, CODE_WEIGHT_METADATA))?;
        let mut trailing = [0_u8; 1];
        if shard
            .file
            .read(&mut trailing)
            .map_err(|_| invalid_model_failure(self.backend, CODE_PAYLOAD_READ))?
            != 0
        {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ));
        }
        let final_length = shard
            .file
            .metadata()
            .map_err(|_| invalid_model_failure(self.backend, CODE_SOURCE_IDENTITY_LENGTH))?
            .len();
        if final_length != shard.file_length || final_length != expected_length {
            return Err(invalid_model_failure(
                self.backend,
                CODE_SOURCE_IDENTITY_LENGTH,
            ));
        }
        Ok(())
    }
}
