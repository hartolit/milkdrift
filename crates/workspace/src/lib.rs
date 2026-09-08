//! Pass exact values between tasks while keeping branch-owned changes separate.
//!
//! A workspace is logical state, independent of a process's files. Each
//! [`WorkspaceValueReference`] names a run, scope, key, and immutable version.
//! [`ScopeLineage`] checks which ancestor values a branch can read. A branch starts
//! its own stream to change an inherited value, leaving the parent version available
//! to other branches. Runtime and persistence enforce these relationships on use.
//!
//! [`WorkspaceValue`] holds small JSON or an [`ArtifactReference`] to separately stored
//! content. [`ArtifactMetadata`] adds sensitivity, retention, and provenance; persistence
//! owns publication and reads. [`WorkspaceBudget`] computes usage for the owner to commit
//! with each accepted change. Constructing these values performs no storage or file I/O.
//!
//! Here a branch inherits an input and advances its own stream. The root still names
//! the original input, and a sibling lineage cannot read the branch's new version.
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
//! let sibling = WorkspaceScope::branch(
//!     ScopeId::new("scope-b")?, &root, BranchId::new("branch-b")?,
//! )?;
//! let lineage = ScopeLineage::new(vec![root.clone(), branch])?;
//! let sibling_lineage = ScopeLineage::new(vec![root.clone(), sibling])?;
//! let input = WorkspaceValueEntry::initial(
//!     root.reference().clone(), ValueKey::new("request")?,
//!     WorkspaceValue::Json(BoundedJson::new(json!("review this change"))?),
//! );
//! assert!(lineage.can_read(input.reference()));
//! let local = WorkspaceValueEntry::inherited(
//!     lineage.leaf().reference().clone(), ValueKey::new("request")?,
//!     input.reference().clone(), input.value().clone(),
//! )?;
//! let revised = WorkspaceValueEntry::successor(
//!     local.reference().clone(),
//!     WorkspaceValue::Json(BoundedJson::new(json!("also check the failure path"))?),
//! )?;
//! assert_eq!(revised.reference().version().get(), 2);
//! assert!(lineage.owns_value_stream(revised.reference()));
//! assert!(!sibling_lineage.can_read(revised.reference()));
//! assert!(sibling_lineage.can_read(input.reference()));
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
