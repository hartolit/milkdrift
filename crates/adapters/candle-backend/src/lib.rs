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
#[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
mod load_observation;
mod loader;
mod model;
mod sequence_reservation;
mod source;
#[cfg(test)]
mod upstream;

pub use device::{CandleDeviceSummary, CudaComputeCapability};
#[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
#[doc(hidden)]
pub use load_observation::{
    CandleLoadCleanupOutcome, CandleLoadObservation, CandleLoadObservationOutcome,
    CandleLoadObservationRecorder, CandleLoadObservationSnapshot,
};
#[cfg(feature = "cuda-hardware-tests")]
#[doc(hidden)]
pub use loader::CandleHardwareLoadFault;
pub use loader::{CandleLlamaFailedPreparation, CandleLlamaLoader, CandleLlamaPreparedLoad};
pub use model::{CandleLlamaModel, CandleLlamaSequence};
pub use source::{
    CandleExpectedContentIdentity, CandleLlamaSource, CandleWeightShard, SourceError,
};
