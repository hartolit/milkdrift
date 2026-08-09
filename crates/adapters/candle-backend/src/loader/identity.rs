//! Whole-shard identity establishment from retained open files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use domain_contracts::{BackendId, LoadError};
use sha2::{Digest, Sha256};

use crate::failure::{
    CODE_HEADER_IDENTITY_MISMATCH, CODE_INSPECTION_ALLOCATION, CODE_NUMERIC_OVERFLOW,
    CODE_PAYLOAD_READ, CODE_SOURCE_IDENTITY_LENGTH, CODE_SOURCE_IDENTITY_MISMATCH,
};
use crate::source::CandleShardIdentity;

use super::manifest::InspectedShard;
use super::{host_memory_failure, invalid_model_failure};

const VERIFICATION_BUFFER_BYTES: usize = 64 * 1024;
const VERIFICATION_BUFFER_BYTES_U64: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EstablishedIdentityAuthority {
    VerifiedImmutable,
    ProjectEstablished,
    UnverifiedBaseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EstablishedShardIdentity {
    pub(super) byte_length: u64,
    pub(super) sha256: [u8; 32],
    pub(super) authority: EstablishedIdentityAuthority,
}

pub(super) fn establish_all(
    backend: BackendId,
    shards: &mut [InspectedShard],
) -> Result<(), LoadError> {
    establish_all_with_observer(backend, shards, &mut NoopIdentityObserver)
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
        let established = match shard.source_identity {
            CandleShardIdentity::VerifiedImmutable {
                byte_length,
                sha256,
            } => {
                validate_expected_length(backend, byte_length, current_length)?;
                observer.supplied(EstablishedIdentityAuthority::VerifiedImmutable);
                EstablishedShardIdentity {
                    byte_length,
                    sha256,
                    authority: EstablishedIdentityAuthority::VerifiedImmutable,
                }
            }
            CandleShardIdentity::ProjectEstablished {
                byte_length,
                sha256,
            } => {
                validate_expected_length(backend, byte_length, current_length)?;
                observer.supplied(EstablishedIdentityAuthority::ProjectEstablished);
                let computed_identity = baseline_unverified(backend, shard, observer)?;
                if computed_identity.byte_length != byte_length
                    || computed_identity.sha256 != sha256
                {
                    return Err(invalid_model_failure(
                        backend,
                        CODE_SOURCE_IDENTITY_MISMATCH,
                    ));
                }
                EstablishedShardIdentity {
                    byte_length,
                    sha256,
                    authority: EstablishedIdentityAuthority::ProjectEstablished,
                }
            }
            CandleShardIdentity::Unverified => baseline_unverified(backend, shard, observer)?,
        };
        shard.established_identity = Some(established);
    }
    Ok(())
}

fn baseline_unverified<O: IdentityObserver>(
    backend: BackendId,
    shard: &mut InspectedShard,
    observer: &mut O,
) -> Result<EstablishedShardIdentity, LoadError> {
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
    Ok(EstablishedShardIdentity {
        byte_length: shard.file_length,
        sha256: hasher.finalize().into(),
        authority: EstablishedIdentityAuthority::UnverifiedBaseline,
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

fn verification_buffer(backend: BackendId) -> Result<Vec<u8>, LoadError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(VERIFICATION_BUFFER_BYTES)
        .map_err(|_| host_memory_failure(backend, CODE_INSPECTION_ALLOCATION))?;
    buffer.resize(VERIFICATION_BUFFER_BYTES, 0);
    Ok(buffer)
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
    fn supplied(&mut self, _authority: EstablishedIdentityAuthority) {}
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

    use super::{EstablishedIdentityAuthority, IdentityObserver, establish_all_with_observer};
    use crate::loader::manifest::InspectedShard;
    use crate::source::CandleShardIdentity;

    static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

    #[derive(Default)]
    struct Counters {
        baseline_bytes: usize,
        verified: usize,
        project: usize,
    }

    impl IdentityObserver for Counters {
        fn baseline_bytes(&mut self, bytes: usize) {
            self.baseline_bytes += bytes;
        }

        fn supplied(&mut self, authority: EstablishedIdentityAuthority) {
            match authority {
                EstablishedIdentityAuthority::VerifiedImmutable => self.verified += 1,
                EstablishedIdentityAuthority::ProjectEstablished => self.project += 1,
                EstablishedIdentityAuthority::UnverifiedBaseline => {}
            }
        }
    }

    #[test]
    fn only_verified_immutable_identity_skips_the_pre_admission_baseline() -> Result<(), String> {
        let bytes = safetensors_bytes(br"{}", b"payload");
        let whole_sha256: [u8; 32] = Sha256::digest(bytes.as_slice()).into();
        let mut shards = vec![
            inspected_file(
                bytes.as_slice(),
                CandleShardIdentity::VerifiedImmutable {
                    byte_length: u64::try_from(bytes.len()).map_err(|error| error.to_string())?,
                    sha256: whole_sha256,
                },
            )?,
            inspected_file(
                bytes.as_slice(),
                CandleShardIdentity::ProjectEstablished {
                    byte_length: u64::try_from(bytes.len()).map_err(|error| error.to_string())?,
                    sha256: whole_sha256,
                },
            )?,
            inspected_file(bytes.as_slice(), CandleShardIdentity::Unverified)?,
        ];
        let mut counters = Counters::default();
        establish_all_with_observer(BackendId::new(1), &mut shards, &mut counters)
            .map_err(|error| format!("establish identities: {error:?}"))?;

        assert_eq!(counters.verified, 1);
        assert_eq!(counters.project, 1);
        assert_eq!(counters.baseline_bytes, bytes.len() * 2);
        let [verified_shard, project_shard, baseline_shard] = shards.as_slice() else {
            return Err("identity fixture must contain three shards".to_owned());
        };
        assert_eq!(
            verified_shard
                .established_identity
                .ok_or_else(|| "missing verified identity".to_owned())?
                .authority,
            EstablishedIdentityAuthority::VerifiedImmutable
        );
        assert_eq!(
            project_shard
                .established_identity
                .ok_or_else(|| "missing project identity".to_owned())?
                .authority,
            EstablishedIdentityAuthority::ProjectEstablished
        );
        assert_eq!(
            baseline_shard
                .established_identity
                .ok_or_else(|| "missing baseline identity".to_owned())?
                .sha256,
            whole_sha256
        );
        Ok(())
    }

    #[test]
    fn project_established_identity_rejects_changed_bytes_before_admission() -> Result<(), String> {
        let original = safetensors_bytes(br"{}", b"payload");
        let original_sha256: [u8; 32] = Sha256::digest(original.as_slice()).into();
        let mut changed = original.clone();
        let last = changed
            .last_mut()
            .ok_or_else(|| "fixture must not be empty".to_owned())?;
        *last ^= 1;
        let mut shards = vec![inspected_file(
            changed.as_slice(),
            CandleShardIdentity::ProjectEstablished {
                byte_length: u64::try_from(original.len()).map_err(|error| error.to_string())?,
                sha256: original_sha256,
            },
        )?];

        let error =
            establish_all_with_observer(BackendId::new(1), &mut shards, &mut Counters::default())
                .err()
                .ok_or_else(|| {
                    "changed project-established bytes were admitted unexpectedly".to_owned()
                })?;
        assert!(matches!(
            error,
            domain_contracts::LoadError::Backend(failure)
                if failure.code == crate::failure::CODE_SOURCE_IDENTITY_MISMATCH
        ));
        let rejected_shard = shards
            .first()
            .ok_or_else(|| "identity fixture must contain one shard".to_owned())?;
        assert!(rejected_shard.established_identity.is_none());
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
        source_identity: CandleShardIdentity,
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
            source_identity,
            established_identity: None,
            tensors: Vec::new(),
        })
    }
}
