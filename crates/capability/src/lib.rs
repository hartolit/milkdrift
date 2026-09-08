//! Describe the external operation a task needs and the evidence returned by its executor.
//!
//! Start with [`CapabilityRequirement`] for a task and [`DescriptorBuilder`] for an adapter's
//! advertisement. [`CapabilityDescriptor::matches`] checks compatibility; the capability host
//! separately checks authority, health, and capacity. Once selected, a
//! [`ResolvedCapabilitySnapshot`] freezes the generation for an attempt.
//!
//! For example, ask for a read-only operation with cancellation support:
//!
//! ```
//! use milkdrift_capability::{CapabilityRequirement, OperationId, SideEffectClass};
//! let requirement = CapabilityRequirement::new(OperationId::new("example.inspect")?)
//!     .maximum_side_effect(SideEffectClass::ReadOnly)
//!     .cancellation(true);
//! assert!(requirement.cancellation_required());
//! # Ok::<(), milkdrift_capability::ContractError>(())
//! ```
//!
//! [`InvocationRequest`] carries inputs to that generation. [`InvocationEvent`] carries bounded
//! observations back; runtime decides their effect on the workflow. Use the `*Document` readers
//! and canonical writers for portable forms. Live adapters and their resource ownership belong
//! in `milkdrift-capability-host`.

mod admission;
mod bounded;
mod descriptor;
mod document;
mod identity;
mod invocation;
mod resolved;

pub use admission::{AdmissionBound, AdmissionMonetaryBound, InvocationAdmissionEnvelope};
pub use bounded::{BoundedJson, ContractError, MAX_DOCUMENT_BYTES};
pub use descriptor::{
    AdmissionConstraints, CancellationBehavior, CapabilityCategory, CapabilityDescriptor,
    CapabilityObservation, CapabilityRequirement, DescriptorBuilder, ExecutionTrustClass,
    FeatureContract, IdempotencyBehavior, Locality, OperationContract, RequirementMatch,
    ResourceObservations, SchemaContract, SideEffectClass, StreamingMode,
};
pub use document::{
    CancellationAcknowledgementDocument, CancellationRequestDocument, CapabilityDescriptorDocument,
    InvocationEventDocument, InvocationRequestDocument, ResolvedCapabilitySnapshotDocument,
};
pub use identity::{
    CapabilityId, ExtensionKey, FeatureId, IdempotencyKey, InvocationId, OperationId, PeerId,
    ProviderProfileRef, SchemaId, TrustZone,
};
pub use invocation::{
    ArtifactReference, CONTEXT_ITEM_INPUT_PREFIX, CONTEXT_MANIFEST_INPUT_NAME,
    CancellationAcknowledgement, CancellationRequest, ErrorClass, InputReference, InvocationEvent,
    InvocationEventKind, InvocationFailure, InvocationRequest, InvocationTerminal,
    InvocationValueReference, MAX_DURABLE_REFERENCE_BYTES, TerminalStatus, UsageObservation,
};
pub use resolved::ResolvedCapabilitySnapshot;
