//! Real-store artifact, clock, capacity and managed protection contracts.
#[path = "contracts/artifact.rs"]
mod artifact;
#[path = "contracts/capacity.rs"]
mod capacity;
#[path = "contracts/clock.rs"]
mod clock;
#[path = "contracts/fault_reopen.rs"]
mod fault_reopen;
#[path = "contracts/managed.rs"]
mod managed;
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
    RequestedResourceFacts,
};
use milkdrift_blueprint::{TaskId, WorkflowId};
use milkdrift_capability::{BoundedJson, CapabilityId, OperationId, SideEffectClass};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    BeginArtifactOutcome, BeginArtifactPublication, ClockWatermarkObservation, ClockWatermarkStore,
    IntegrityScanRequest, OrphanCleanupRequest, PageSize, PersistenceError, StorageAdmin,
    StorageFailureClass, TimestampMillis, WorkflowCommit, WorkflowSnapshot, WorkflowStore,
};
use milkdrift_redb_store::{FaultPoint, RedbStore, RedbStoreConfig, StoreClock};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactOwner, ArtifactProvenance, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
};
use redb::{Database, TableDefinition};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;
fn usage(store: &RedbStore, run: &RunId) -> Result<WorkspaceUsage, PersistenceError> {
    store.artifact_usage(&ArtifactOwner::from(run))
}

fn artifact_metadata(
    id: &str,
    bytes: &[u8],
    sensitivity: ArtifactSensitivity,
) -> Result<ArtifactMetadata, Box<dyn std::error::Error>> {
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(id)?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("application/octet-stream")?,
        bytes.len() as u64,
    );
    Ok(ArtifactMetadata::new(
        reference,
        sensitivity,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("test-source")?,
            },
            Vec::new(),
        )?,
    )?)
}

#[path = "support/fault.rs"]
mod fault;
use fault::FailOnce;
