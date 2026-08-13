//! Optional frontend-neutral application services over Milkdrift's local inference engine.
//!
//! This crate is a reference application kit: it composes immutable Hub resolution, narrow
//! persistence, device selection, the local E0 runtime, bounded decoded output, and cooperative
//! shutdown. It is not the only supported Milkdrift API and it is not a workflow control plane.
//! Engine-level and future workflow consumers may use lower layers directly.
//!
//! # Truthful lifecycle state
//!
//! [`ResolvedModel`] means one immutable repository commit and tokenizer were resolved and
//! validated for application use. Its optional scalar value is only a recognized configuration
//! declaration; it is not an execution fact or a claim that every tensor is homogeneous.
//!
//! [`LoadedModel`] is published only after a correlated E0 receipt passes generic transaction
//! checks. Its device and scalar are actual verified execution facts from that receipt. A loaded
//! model is never exposed at the same time as [`ApplicationRetainedModel`].
//!
//! Retained state means lower model ownership was not released. [`ApplicationRetainedOwnership`]
//! distinguishes exact, unverified, and unknown ownership without inventing byte counts.
//! [`ApplicationModelCleanupDisposition`] independently reports lower retryability, E1
//! coordination retry, worker disconnection, or process-lifetime retention. Selection and normal
//! load admission remain locked until explicit release evidence is observed.
//!
//! # Headless driving loop
//!
//! A headless host drives the same bounded API as a GUI: submit one operation, poll a bounded
//! number of events, pull bounded output, and inspect [`ApplicationState`] for admission. The
//! following complete lifecycle is compile-checked without requiring a frontend or exposing
//! Candle, Safetensors, persistence-record, or presentation types.
//!
//! ```no_run
//! use std::thread;
//! use std::time::Duration;
//!
//! use application_runtime::{
//!     ApplicationDevice, ApplicationEvent, ApplicationOutputRecordKind, ApplicationOutputState,
//!     ApplicationRuntime, ApplicationRuntimeConfiguration, GenerationSettings, ModelSelection,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let configuration = ApplicationRuntimeConfiguration::new("milkdrift.redb");
//! let mut runtime = ApplicationRuntime::start(configuration)?;
//! let selection = ModelSelection::new("TinyLlama/TinyLlama-1.1B-Chat-v1.0", "main");
//!
//! runtime.select_device(ApplicationDevice::Cpu)?;
//! runtime.resolve_model(selection.clone())?;
//! loop {
//!     match runtime.poll_event() {
//!         Some(ApplicationEvent::ModelResolved { .. }) => break,
//!         Some(ApplicationEvent::ModelResolutionFailed { failure }) => return Err(failure.into()),
//!         _ => thread::sleep(Duration::from_millis(1)),
//!     }
//! }
//!
//! runtime.load_model(&selection)?;
//! loop {
//!     match runtime.poll_event() {
//!         Some(ApplicationEvent::ModelLoaded { .. }) => break,
//!         Some(ApplicationEvent::ModelLoadFailed { failure }) => return Err(failure.into()),
//!         Some(ApplicationEvent::ModelCleanupPending { .. }) => {
//!             let cleanup = runtime.state().retained_model().ok_or("missing retained state")?;
//!             return Err(cleanup.primary_failure().clone().into());
//!         }
//!         _ => thread::sleep(Duration::from_millis(1)),
//!     }
//! }
//!
//! let request_id = runtime.start_generation("A bounded prompt", GenerationSettings::default())?;
//! let mut cancellation_requested = false;
//! let mut generation_released = false;
//! while !generation_released {
//!     let _event = runtime.poll_event();
//!     if !cancellation_requested && runtime.state().can_cancel_generation() {
//!         runtime.cancel_generation(request_id)?;
//!         cancellation_requested = true;
//!     }
//!     generation_released = runtime.pull_output(|batch| {
//!         batch.records().any(|record| {
//!             matches!(
//!                 record.kind,
//!                 ApplicationOutputRecordKind::State(ApplicationOutputState::Released(_))
//!             )
//!         })
//!     })?;
//!     if !generation_released {
//!         thread::sleep(Duration::from_millis(1));
//!     }
//! }
//!
//! runtime.unload_model()?;
//! loop {
//!     if runtime.state().can_retry_model_cleanup() {
//!         runtime.retry_model_cleanup()?;
//!     }
//!     match runtime.poll_event() {
//!         Some(ApplicationEvent::ModelUnloaded { .. })
//!         | Some(ApplicationEvent::ModelCleanupReleased { .. }) => break,
//!         _ => thread::sleep(Duration::from_millis(1)),
//!     }
//! }
//!
//! runtime.shutdown()?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod chat;
mod configuration;
mod conversation;
mod error;
mod event;
mod generation;
mod hub_worker;
mod local;
mod retention;
mod runtime;
mod selection;
mod shutdown;
mod state;
mod support;
mod unload;

pub use chat::{ChatCompatibility, ContextDiagnostics};
pub use configuration::{
    AcceleratorMemoryPolicy, ApplicationHubConfiguration, ApplicationPreferences,
    ApplicationRuntimeConfiguration, ApplicationTiming,
};
pub use conversation::{
    ConversationProvenance, ConversationRecord, ConversationRecordId, ConversationRetention,
    ConversationRole, ConversationTokenEstimate, ResponseAttempt, ResponseAttemptId,
    ResponseAttemptState,
};

pub use domain_contracts::MemoryFootprint;
pub use error::{
    ApplicationConfigurationField, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationWorker, GenerationSettingsField,
};
pub use event::ApplicationEvent;
pub use generation::{
    ApplicationOutputBatch, ApplicationOutputRecord, ApplicationOutputRecordKind,
    ApplicationOutputState, ApplicationTextRange, GenerationSeed, GenerationSettings,
    GenerationTerminalKind,
};
pub use retention::{
    ApplicationConservativeFootprint, ApplicationModelCleanupDisposition, ApplicationRetainedModel,
    ApplicationRetainedModelResource, ApplicationRetainedOwnership,
};
pub use runtime::ApplicationRuntime;

/// Executes the complete download-free E1 CUDA hardware suite.
///
/// This entry point exists only for the explicitly feature-gated harness-free test target.
/// It is not a product runtime API.
///
/// # Errors
///
/// Returns a diagnostic when opt-in is absent or any registered hardware case fails.
#[cfg(feature = "cuda-hardware-tests")]
#[doc(hidden)]
pub fn __run_cuda_hardware_suite() -> Result<(), String> {
    runtime::cuda_hardware::run_hardware_suite()
}

pub use selection::{
    ApplicationComputeCapability, ApplicationDevice, ApplicationDeviceDiscoveryFailure,
    ApplicationDeviceDiscoveryFailureKind, ApplicationDeviceSummary,
    ApplicationDeviceUnavailableReason, ApplicationScalarType, ImmutableModelIdentity,
    ModelSelection,
};
pub use state::{
    ApplicationActivity, ApplicationGenerationMode, ApplicationState, GenerationPhase,
    GenerationSummary, GenerationTerminal, GenerationTerminalOutcome, LoadedModel, ResolvedModel,
};
pub use unload::ModelUnloadBehavior;
