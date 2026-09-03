//! Immutable contracts for Milkdrift's durable, scoped workspace.
//!
//! A workspace is a set of immutable value versions. Every value reference names an
//! exact run, scope, key, and version; there is no shared mutable map hidden behind
//! these types. [`ScopeLineage`] makes the visibility rule explicit: a scope can read
//! exact values from its ancestors, while new versions belong only to its leaf scope.
//! Sibling branches therefore cannot observe or overwrite one another's local values.
//!
//! Large values are represented by [`ArtifactReference`] rather than embedded bytes.
//! [`ArtifactMetadata`] records a BLAKE3 digest, exact size, media type, sensitivity,
//! retention, and causal provenance. This crate defines those portable facts and
//! budget calculations only. It deliberately owns no storage, filesystem paths,
//! scheduler, clock, asynchronous runtime, or artifact I/O.
//!
//! ```
//! use milkdrift_capability::BoundedJson;
//! use milkdrift_workspace::{
//!     BranchId, RunId, ScopeId, ScopeLineage, ValueKey, WorkspaceScope,
//!     WorkspaceValue, WorkspaceValueEntry,
//! };
//! use serde_json::json;
//!
//! let root = WorkspaceScope::run_root(RunId::new("run-1")?, ScopeId::new("root")?);
//! let branch = WorkspaceScope::branch(
//!     ScopeId::new("scope-a")?,
//!     &root,
//!     BranchId::new("branch-a")?,
//! )?;
//! let lineage = ScopeLineage::new(vec![root, branch])?;
//! let value = WorkspaceValueEntry::initial(
//!     lineage.leaf().reference().clone(),
//!     ValueKey::new("answer")?,
//!     WorkspaceValue::Json(BoundedJson::new(json!(42))?),
//! );
//! assert!(lineage.owns_value_stream(value.reference()));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod artifact;
mod budget;
mod error;
mod identity;
mod scope;
mod value;

pub use artifact::{
    ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalReference, ContentDigest, MAX_MEDIA_TYPE_BYTES, MediaType,
    RetentionDeadline,
};
pub use budget::{WorkspaceBudget, WorkspaceUsage};
pub use error::WorkspaceError;
pub use identity::{
    ArtifactId, BranchId, CausalId, IterationId, MAX_EXTENDED_ID_BYTES, MAX_STANDARD_ID_BYTES,
    RunId, ScopeId, SubworkflowId, ValueKey, ValueVersion,
};
pub use scope::{MAX_SCOPE_DEPTH, ScopeKind, ScopeLineage, ScopeReference, WorkspaceScope};
pub use value::{ValueOrigin, WorkspaceValue, WorkspaceValueEntry, WorkspaceValueReference};
