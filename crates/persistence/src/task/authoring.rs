//! Latest authoring checkpoints are mutable content, not executable tasks or command receipts.

use std::collections::BTreeMap;

use milkdrift_blueprint::WorkflowId;
use serde::{Deserialize, Serialize};

/// Maximum canonical JSON bytes in one saved authoring document.
pub const MAX_WORKFLOW_AUTHORING_BYTES: usize =
    milkdrift_blueprint::authoring::MAX_AUTHORING_DOCUMENT_BYTES;
/// Maximum independently saved card positions in one workflow.
pub const MAX_WORKFLOW_POSITIONS: usize = 600;
/// Maximum artifact sources protected by one latest authoring document.
pub const MAX_WORKFLOW_PROTECTED_SOURCES: usize = 400;

/// A latest snapshot whose semantic document is constructed and validated by shared control.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowAuthoringSnapshot {
    /// Existing daemon-owned workflow, never a client-local destination.
    pub workflow: WorkflowId,
    /// Independent authoring generation; zero means no checkpoint has been accepted.
    pub generation: u64,
    /// Storage-calculated BLAKE3 digest of the canonical current document.
    pub digest: String,
    /// Bounded non-executable authoring content, including remaining candidates and accepted IDs.
    pub document: serde_json::Value,
    /// Artifact identities resolved under authority and retained by this latest document.
    pub protected_sources: Vec<String>,
}

/// One conditional replacement; it carries no idempotency key or historical receipt promise.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowAuthoringCommit {
    /// Current saved generation observed by the author; zero starts an absent document.
    pub expected_generation: u64,
    /// Complete replacement content validated by the semantic authoring owner.
    pub document: serde_json::Value,
    /// Current artifact identities to protect, derived from the authorized document sources.
    pub protected_sources: Vec<String>,
}

/// A finite presentation coordinate; position never changes readiness or executable meaning.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPosition {
    /// Horizontal canvas coordinate.
    pub x: f64,
    /// Vertical canvas coordinate.
    pub y: f64,
}

/// Independently versioned latest positions. Camera and selection remain client preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPositionsSnapshot {
    /// Existing workflow whose cards are arranged.
    pub workflow: WorkflowId,
    /// Position-only generation, independent of saved semantic content and runtime activity.
    pub generation: u64,
    /// Storage-calculated BLAKE3 digest of the canonical positions map.
    pub digest: String,
    /// Stable card identities and finite coordinates, bounded by MAX_WORKFLOW_POSITIONS.
    pub positions: BTreeMap<String, WorkflowPosition>,
}

/// A complete conditional position replacement, with no execution or authoring side effects.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowPositionsCommit {
    /// Last observed position-only generation.
    pub expected_generation: u64,
    /// Complete latest positions; omission deliberately removes a saved position.
    pub positions: BTreeMap<String, WorkflowPosition>,
}
