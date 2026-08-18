//! Pure, provider-neutral capability and invocation contracts for Milkdrift.
//!
//! The crate describes what an executor claims and what an invocation observed.
//! It deliberately owns no live registry, credentials, provider client, transport,
//! or executor lifecycle.

mod bounded;
mod descriptor;
mod document;
mod identity;
mod invocation;

pub use bounded::{BoundedJson, ContractError, MAX_DOCUMENT_BYTES, MAX_JSON_DEPTH};
pub use descriptor::{
    AdmissionConstraints, CancellationBehavior, CapabilityCategory, CapabilityDescriptor,
    CapabilityObservation, CapabilityRequirement, DescriptorBuilder, FeatureContract,
    IdempotencyBehavior, Locality, OperationContract, RequirementMatch, ResourceObservations,
    SchemaContract, SideEffectClass, StreamingMode,
};
pub use document::{
    CancellationAcknowledgementDocument, CancellationRequestDocument, CapabilityDescriptorDocument,
    InvocationEventDocument, InvocationRequestDocument, SCHEMA_VERSION_V1, canonical_json_bytes,
};
pub use identity::{
    CapabilityId, ExtensionKey, FeatureId, IdempotencyKey, InvocationId, OperationId,
    ProviderProfileRef, SchemaId, TrustZone,
};
pub use invocation::{
    ArtifactReference, CancellationAcknowledgement, CancellationRequest, ErrorClass,
    InputReference, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationRequest,
    InvocationTerminal, InvocationValueReference, TerminalStatus, UsageObservation,
};
