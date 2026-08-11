//! CPU-default and feature-gated CUDA Llama adapter that quarantines Candle
//! behind `domain-contracts`.
//!
//! CUDA execution is compiled only with the non-default `cuda` feature and is
//! always selected explicitly; enabling the feature never changes CPU requests.
//! Candle's upstream Llama
//! implementation grows its KV cache with tensor concatenation, so this crate
//! deliberately does not advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`.

#![forbid(unsafe_code)]

mod device;
mod failure;
mod loader;
mod model;
mod sequence_reservation;
mod source;

pub use device::{CandleDeviceSummary, CudaComputeCapability};
pub use loader::{CandleLlamaFailedPreparation, CandleLlamaLoader, CandleLlamaPreparedLoad};
pub use model::{CandleLlamaModel, CandleLlamaSequence};
pub use source::{CandleLlamaSource, CandleShardIdentity, CandleWeightShard, SourceError};
