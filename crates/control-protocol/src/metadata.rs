//! Shared installation naming transport. Validation and durable acceptance belong to its owner.
use serde::{Deserialize, Serialize};

/// Current daemon name and the authenticated actor's permission to change it.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DaemonMetadataDto {
    /// Stable identity, unaffected by renaming.
    pub daemon_id: String,
    /// Shared human-readable name supplied by the daemon.
    pub name: String,
    /// Guard for the next rename, not a workflow generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
    /// Current authorization to rename this daemon.
    pub can_rename: bool,
}

/// Exact generation-guarded rename. Retain every field when recovering a lost reply.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RenameDaemonRequest {
    /// Actor-scoped immutable mutation identity.
    pub request_id: String,
    /// Previously observed daemon name generation.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub expected_generation: u64,
    /// Proposed name, normalized and validated by the daemon's metadata owner.
    pub name: String,
}

/// Accepted name and generation for one exact rename, retained independently of workflows.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[cfg_attr(feature = "web-codegen", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RenameDaemonReceipt {
    /// Actor-scoped accepted mutation identity.
    pub request_id: String,
    /// Stable identity of the daemon that accepted the request.
    pub daemon_id: String,
    /// Accepted normalized name, which a later rename may supersede.
    pub name: String,
    /// Generation established by this request.
    #[cfg_attr(feature = "web-codegen", ts(type = "number"))]
    pub generation: u64,
}
