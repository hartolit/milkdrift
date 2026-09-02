//! Pure, provider-neutral capability and invocation contracts for Milkdrift.
//!
//! The crate describes what an executor claims and what an invocation observed.
//! It deliberately owns no live registry, credentials, provider client, transport,
//! or executor lifecycle.

mod admission;
mod bounded;
mod descriptor;
mod document;
mod identity;
mod invocation;
mod resolved;

pub use admission::{AdmissionBound, AdmissionMonetaryBound, InvocationAdmissionEnvelope};
pub use bounded::{BoundedJson, ContractError, MAX_DOCUMENT_BYTES, MAX_JSON_DEPTH};
pub use descriptor::{
    AdmissionConstraints, CancellationBehavior, CapabilityCategory, CapabilityDescriptor,
    CapabilityObservation, CapabilityRequirement, DescriptorBuilder, ExecutionTrustClass,
    FeatureContract, IdempotencyBehavior, Locality, OperationContract, RequirementMatch,
    ResourceObservations, SchemaContract, SideEffectClass, StreamingMode,
};
pub use document::{
    CancellationAcknowledgementDocument, CancellationRequestDocument, CapabilityDescriptorDocument,
    INVOCATION_REQUEST_SCHEMA_VERSION_V2, InvocationEventDocument, InvocationRequestDocument,
    RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2, ResolvedCapabilitySnapshotDocument,
    SCHEMA_VERSION_V1,
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
