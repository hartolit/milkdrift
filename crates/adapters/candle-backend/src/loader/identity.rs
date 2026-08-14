//! Whole-shard identity establishment from retained open files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use domain_contracts::{BackendId, LoadError};
use sha2::{Digest, Sha256};

use super::manifest::InspectedShard;
use super::payload::verification_buffer;
use super::{VERIFICATION_BUFFER_BYTES_U64, invalid_model_failure};
use crate::failure::{
    CODE_HEADER_IDENTITY_MISMATCH, CODE_NUMERIC_OVERFLOW, CODE_PAYLOAD_READ,
    CODE_SOURCE_IDENTITY_LENGTH,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContentIdentityEstablishment {
    SuppliedExpectation,
    LocallyEstablishedBaseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EstablishedContentIdentity {
    pub(super) byte_length: u64,
    pub(super) sha256: [u8; 32],
    pub(super) establishment: ContentIdentityEstablishment,
}

pub(super) fn establish_all(
    backend: BackendId,
    shards: &mut [InspectedShard],
) -> Result<(), LoadError> {
    establish_all_with_observer(backend, shards, &mut NoopIdentityObserver)
}

#[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
pub(super) fn establish_all_observed(
    backend: BackendId,
    shards: &mut [InspectedShard],
    observation: &crate::CandleLoadObservationRecorder,
) -> Result<(), LoadError> {
    struct LoadIdentityObserver<'a>(&'a crate::CandleLoadObservationRecorder);

    impl IdentityObserver for LoadIdentityObserver<'_> {
        fn baseline_bytes(&mut self, bytes: usize) {
            self.0
                .verification_only_bytes_read(u64::try_from(bytes).unwrap_or(u64::MAX));
        }
    }

    establish_all_with_observer(backend, shards, &mut LoadIdentityObserver(observation))
}

fn establish_all_with_observer<O: IdentityObserver>(
    backend: BackendId,
    shards: &mut [InspectedShard],
    observer: &mut O,
) -> Result<(), LoadError> {
    for shard in shards {
        let current_length = retained_file_length(backend, &shard.file)?;
        if current_length != shard.file_length {
            return Err(invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH));
        }
        let established = match shard.source_expected_content {
            Some(expected) => {
                validate_expected_length(backend, expected.byte_length(), current_length)?;
                observer.supplied_expectation();
                EstablishedContentIdentity {
                    byte_length: expected.byte_length(),
                    sha256: expected.sha256(),
                    establishment: ContentIdentityEstablishment::SuppliedExpectation,
                }
            }
            None => baseline_unverified(backend, shard, observer)?,
        };
        shard.established_content_identity = Some(established);
    }
    Ok(())
}

fn baseline_unverified<O: IdentityObserver>(
    backend: BackendId,
    shard: &mut InspectedShard,
    observer: &mut O,
) -> Result<EstablishedContentIdentity, LoadError> {
    shard
        .file
        .seek(SeekFrom::Start(0))
        .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
    let mut hasher = Sha256::new();
    let mut buffer = verification_buffer(backend)?;
    hash_exact(
        backend,
        &mut shard.file,
        shard.data_start,
        buffer.as_mut_slice(),
        &mut hasher,
        observer,
    )?;
    let observed_header: [u8; 32] = hasher.clone().finalize().into();
    if observed_header != shard.prefix_header_sha256 {
        return Err(invalid_model_failure(
            backend,
            CODE_HEADER_IDENTITY_MISMATCH,
        ));
    }
    let payload_bytes = shard
        .file_length
        .checked_sub(shard.data_start)
        .ok_or_else(|| invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH))?;
    hash_exact(
        backend,
        &mut shard.file,
        payload_bytes,
        buffer.as_mut_slice(),
        &mut hasher,
        observer,
    )?;
    verify_exact_eof(backend, &mut shard.file, shard.file_length)?;
    Ok(EstablishedContentIdentity {
        byte_length: shard.file_length,
        sha256: hasher.finalize().into(),
        establishment: ContentIdentityEstablishment::LocallyEstablishedBaseline,
    })
}

fn hash_exact<O: IdentityObserver>(
    backend: BackendId,
    file: &mut File,
    byte_count: u64,
    buffer: &mut [u8],
    hasher: &mut Sha256,
    observer: &mut O,
) -> Result<(), LoadError> {
    let mut remaining = byte_count;
    while remaining > 0 {
        let chunk_length = usize::try_from(remaining.min(VERIFICATION_BUFFER_BYTES_U64))
            .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
        let chunk = buffer
            .get_mut(..chunk_length)
            .ok_or_else(|| invalid_model_failure(backend, CODE_PAYLOAD_READ))?;
        file.read_exact(chunk)
            .map_err(|_| invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH))?;
        hasher.update(chunk);
        observer.baseline_bytes(chunk_length);
        remaining = remaining
            .checked_sub(
                u64::try_from(chunk_length)
                    .map_err(|_| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?,
            )
            .ok_or_else(|| invalid_model_failure(backend, CODE_NUMERIC_OVERFLOW))?;
    }
    Ok(())
}

fn verify_exact_eof(
    backend: BackendId,
    file: &mut File,
    expected_length: u64,
) -> Result<(), LoadError> {
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| invalid_model_failure(backend, CODE_PAYLOAD_READ))?
        != 0
    {
        return Err(invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH));
    }
    if retained_file_length(backend, file)? != expected_length {
        return Err(invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH));
    }
    Ok(())
}

fn retained_file_length(backend: BackendId, file: &File) -> Result<u64, LoadError> {
    file.metadata()
        .map(|metadata| metadata.len())
        .map_err(|_| invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH))
}

fn validate_expected_length(
    backend: BackendId,
    expected: u64,
    actual: u64,
) -> Result<(), LoadError> {
    if expected == actual {
        Ok(())
    } else {
        Err(invalid_model_failure(backend, CODE_SOURCE_IDENTITY_LENGTH))
    }
}

trait IdentityObserver {
    fn baseline_bytes(&mut self, _bytes: usize) {}
    fn supplied_expectation(&mut self) {}
}

struct NoopIdentityObserver;

impl IdentityObserver for NoopIdentityObserver {}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use domain_contracts::BackendId;
    use sha2::{Digest, Sha256};

    use super::{ContentIdentityEstablishment, IdentityObserver, establish_all_with_observer};
    use crate::loader::manifest::InspectedShard;
    use crate::source::CandleExpectedContentIdentity;

    static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct Counters {
        baseline_bytes: usize,
        supplied_expectations: usize,
    }

    impl IdentityObserver for Counters {
        fn baseline_bytes(&mut self, bytes: usize) {
            self.baseline_bytes += bytes;
        }

        fn supplied_expectation(&mut self) {
            self.supplied_expectations += 1;
        }
    }

    #[test]
    fn supplied_expectation_skips_the_local_baseline() -> Result<(), String> {
        let bytes = safetensors_bytes(br"{}", b"payload");
        let whole_sha256: [u8; 32] = Sha256::digest(bytes.as_slice()).into();
        let mut shards = vec![
            inspected_file(
                bytes.as_slice(),
                Some(CandleExpectedContentIdentity::new(
                    u64::try_from(bytes.len()).map_err(|error| error.to_string())?,
                    whole_sha256,
                )),
            )?,
            inspected_file(bytes.as_slice(), None)?,
        ];
        let mut counters = Counters::default();
        establish_all_with_observer(BackendId::new(1), &mut shards, &mut counters)
            .map_err(|error| format!("establish identities: {error:?}"))?;

        assert_eq!(counters.supplied_expectations, 1);
        assert_eq!(counters.baseline_bytes, bytes.len());
        let [supplied_shard, baseline_shard] = shards.as_slice() else {
            return Err("identity fixture must contain two shards".to_owned());
        };
        assert_eq!(
            supplied_shard
                .established_content_identity
                .ok_or_else(|| "missing supplied expectation".to_owned())?
                .establishment,
            ContentIdentityEstablishment::SuppliedExpectation
        );
        assert_eq!(
            baseline_shard
                .established_content_identity
                .ok_or_else(|| "missing baseline identity".to_owned())?
                .sha256,
            whole_sha256
        );
        assert_eq!(
            baseline_shard
                .established_content_identity
                .ok_or_else(|| "missing baseline identity".to_owned())?
                .establishment,
            ContentIdentityEstablishment::LocallyEstablishedBaseline
        );
        Ok(())
    }

    #[test]
    fn supplied_expected_length_mismatch_is_rejected_explicitly() -> Result<(), String> {
        let bytes = safetensors_bytes(br"{}", b"payload");
        let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
        let mut shards = vec![inspected_file(
            bytes.as_slice(),
            Some(CandleExpectedContentIdentity::new(
                byte_length
                    .checked_add(1)
                    .ok_or_else(|| "fixture length overflow".to_owned())?,
                Sha256::digest(bytes.as_slice()).into(),
            )),
        )?];

        let error =
            establish_all_with_observer(BackendId::new(1), &mut shards, &mut Counters::default())
                .err()
                .ok_or_else(|| "wrong expected length was admitted unexpectedly".to_owned())?;
        assert!(matches!(
            error,
            domain_contracts::LoadError::Backend(failure)
                if failure.code == crate::failure::CODE_SOURCE_IDENTITY_LENGTH
        ));
        let rejected_shard = shards
            .first()
            .ok_or_else(|| "identity fixture must contain one shard".to_owned())?;
        assert!(rejected_shard.established_content_identity.is_none());
        Ok(())
    }

    fn safetensors_bytes(header: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(header.len() as u64).to_le_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn inspected_file(
        bytes: &[u8],
        source_expected_content: Option<CandleExpectedContentIdentity>,
    ) -> Result<InspectedShard, String> {
        let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "milkdrift-candle-identity-{}-{sequence}.safetensors",
            std::process::id()
        ));
        let mut created = File::create(&path).map_err(|error| error.to_string())?;
        created
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        created.sync_all().map_err(|error| error.to_string())?;
        drop(created);
        let file = File::open(&path).map_err(|error| error.to_string())?;
        fs::remove_file(path).map_err(|error| error.to_string())?;
        let data_start = u64::from_le_bytes(
            bytes
                .get(..8)
                .ok_or_else(|| "missing prefix".to_owned())?
                .try_into()
                .map_err(|_| "invalid prefix".to_owned())?,
        )
        .checked_add(8)
        .ok_or_else(|| "data start overflow".to_owned())?;
        let prefix_length = usize::try_from(data_start).map_err(|error| error.to_string())?;
        let prefix_header_sha256: [u8; 32] = Sha256::digest(
            bytes
                .get(..prefix_length)
                .ok_or_else(|| "missing header".to_owned())?,
        )
        .into();
        Ok(InspectedShard {
            file,
            file_length: u64::try_from(bytes.len()).map_err(|error| error.to_string())?,
            data_start,
            prefix_header_sha256,
            source_expected_content,
            established_content_identity: None,
            tensors: Vec::new(),
        })
    }
}
