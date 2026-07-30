//! CPU GGUF backend implemented through safe llama.cpp bindings.

#![deny(unsafe_code)]

mod digest;
mod failure;
mod loader;
mod metadata;
mod model;
mod source;
mod tokenizer;

pub use digest::{Sha256Digest, Sha256DigestParseError, sha256_digest, sha256_file};
pub use loader::{BackendInitializationError, GgufBackendRuntime, GgufLoader};
pub use metadata::{GgufMetadata, MetadataError, inspect_metadata};
pub use model::{GgufModel, GgufSequence};
pub use source::{GgufExecutionConfiguration, GgufInspectionLimits, GgufSource, SourceError};
pub use tokenizer::{
    GgufBoundaryToken, GgufOwnedStreamingDecoder, GgufStreamingDecoder, GgufTokenizer,
    GgufTokenizerLoadError,
};
