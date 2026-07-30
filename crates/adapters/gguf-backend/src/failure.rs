//! Stable conversion from GGUF and llama.cpp failures into domain errors.

use domain_contracts::{BackendFailure, BackendFailureKind, BackendId};

pub const CODE_METADATA_OPEN: u32 = 1;
pub const CODE_METADATA_READ: u32 = 2;
pub const CODE_METADATA_FORMAT: u32 = 3;
pub const CODE_NUMERIC_OVERFLOW: u32 = 4;
pub const CODE_MODEL_LOAD: u32 = 5;
pub const CODE_CONTEXT_CREATE: u32 = 6;
pub const CODE_BATCH_ADD: u32 = 7;
pub const CODE_DECODE: u32 = 8;
pub const CODE_KV_CLEAR: u32 = 9;
pub const CODE_MODEL_MISMATCH: u32 = 10;
pub const CODE_SEQUENCE_SLOT: u32 = 11;
pub const CODE_CONTENT_DIGEST_READ: u32 = 12;
pub const CODE_CONTENT_DIGEST_MISMATCH: u32 = 13;

pub const fn failure(backend: BackendId, kind: BackendFailureKind, code: u32) -> BackendFailure {
    BackendFailure::new(backend, kind, code)
}
