//! A bounded current view for operators; history remains available through exact reads.

use crate::{ArtifactMetadataRead, RunRead};
use serde::{Deserialize, Serialize};

/// Current run facts, separately authorized result evidence, and available control operations.
/// An offered action is a current permission hint; its command still checks all guards and rules.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunResultRead {
    /// Current frontier, enriched with authorized attempt details for the returned nodes.
    pub run: RunRead,
    /// Saved display name, absent when revision inspection is unavailable.
    pub workflow_name: Option<String>,
    /// Saved lineage version, independent of the run sequence.
    pub version: Option<u64>,
    /// This view omitted frontier nodes or terminal outputs to stay within its bounds.
    pub truncated: bool,
    /// Only outputs recorded by a successful workflow terminal, never an intermediate draft.
    pub outputs: Vec<RunOutputRead>,
    /// At least one terminal output was withheld by the caller's read scope.
    pub outputs_restricted: bool,
    /// Current lifecycle-compatible operations allowed by this caller's grant.
    pub actions: Vec<String>,
}

/// One declared terminal output with independently authorized metadata and text preview.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutputRead {
    /// Declared workflow output field.
    pub name: String,
    /// Exact immutable download identity.
    pub artifact: ArtifactMetadataRead,
    /// UTF-8 text when content is readable and textual. Treat it as untrusted terminal text.
    pub preview: Option<String>,
    /// Preview is only a prefix; downloading retrieves the complete verified artifact.
    pub preview_truncated: bool,
}
