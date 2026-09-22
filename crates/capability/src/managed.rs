//! Portable requests and inspection for installations whose resources outlive an invocation.
//!
//! A recipe reference selects exact operator-approved configuration. Callers cannot submit engine
//! arguments, mounts or unit text. The host owns authorization and lifecycle decisions; a platform
//! adapter interprets the approved recipe. Version guards protect a resource independently of the
//! operation receipt, so replaying an old command never reapplies its effects.

use crate::{CapabilityId, ContractError};
use serde::{Deserialize, Serialize};

/// Current installation command and inspection document version.
pub const MANAGED_SCHEMA_VERSION: u32 = 1;
/// Maximum resources in one installation and one coherent use acquisition.
pub const MAX_MANAGED_RESOURCES: usize = 16;
/// Maximum simultaneous unresolved uses of one installation.
pub const MAX_MANAGED_USES: usize = 128;
/// Descriptor extension binding exact managed generations to accepted work.
pub const MANAGED_BINDING_EXTENSION: &str = "org.milkdrift/managed-resources";

milkdrift_contracts::validated_string_type! {
    /// Short path-independent installation, recipe, resource or command name.
    pub struct ManagedName;
    error = ContractError;
    validate = |value: &str, kind: &'static str| {
        if value.is_empty() || value.len() > 64 || !value.as_bytes()[0].is_ascii_lowercase()
            || !value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            Err(ContractError::InvalidIdentity { type_name: kind, reason: "expected 1..=64 lowercase letters, digits or hyphens, starting with a letter".to_owned() })
        } else { Ok(()) }
    };
}

/// An approved immutable recipe; its digest includes all effective non-secret configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeReference {
    /// Operator-configured recipe identity.
    pub name: ManagedName,
    /// Canonical BLAKE3 digest of the approved recipe.
    pub digest: String,
}

/// Retained data disposition chosen before any owned storage is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataDisposition {
    /// Removal leaves data and its ownership evidence for deliberate later handling.
    Preserve,
    /// Removal may delete only this installation's verified owned storage after draining.
    DeleteOnRemoval,
}

/// Maintenance uses fail-fast busy refusal; no hidden or unbounded wait queue exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedAction {
    /// Preview the exact setup and prerequisite diagnosis without accepting effects.
    Prepare {
        /// Approved recipe to inspect.
        recipe: RecipeReference,
    },
    /// Create or identically reapply a setup, preserving existing mutable content.
    Apply {
        /// Approved recipe to apply.
        recipe: RecipeReference,
    },
    /// Inspect current state, pending work and blockers.
    Inspect {},
    /// Restore the declared running service state.
    Start {},
    /// Drain and stop only owned services.
    Stop {},
    /// Replace an approved generation after draining all its users.
    Update {
        /// Exact approved replacement.
        recipe: RecipeReference,
        /// Explicitly allow an interruption when old and new services cannot coexist.
        allow_interruption: bool,
    },
    /// Change future data disposition; this operation itself deletes no data.
    Preserve {
        /// New explicit disposition.
        disposition: DataDisposition,
    },
    /// Remove owned service/configuration and apply the saved data disposition.
    Remove {},
    /// Resume the exact recorded pending change after inspecting physical identities.
    Recover {},
    /// Transfer exclusive editing after exact accepted lineage and physical quiescence.
    Handoff {
        /// Exact inspected parent and child claims.
        transfer: EditingHandoff,
    },
    /// Settle a proven stopped child and optionally return parent editing eligibility.
    Return {
        /// Exact inspected claims after handoff.
        transfer: EditingHandoff,
        /// Current authority and cancellation must still permit resumption.
        resume_parent: bool,
    },
    /// Fence an exact retained use, keeping its execution outcome uncertain.
    Resolve {
        /// Use identity from authorized inspection.
        use_id: String,
        /// Exact claim revision inspected by the operator.
        expected_claim: u64,
    },
}

/// The same typed document is accepted by administration and capability invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRequest {
    /// Exact supported document version.
    pub schema_version: u32,
    /// Actor-scoped idempotency key; changed bytes under this key conflict permanently.
    pub command: ManagedName,
    /// Installation to operate on.
    pub installation: ManagedName,
    /// Zero only for a new installation; mutations compare the durable version.
    pub expected_version: u64,
    /// One closed lifecycle operation.
    pub action: ManagedAction,
}

impl ManagedRequest {
    /// Checks finite version and digest semantics before any authorization or platform effect.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != MANAGED_SCHEMA_VERSION {
            return Err(invalid("unsupported managed request version"));
        }
        let recipe = match &self.action {
            ManagedAction::Prepare { recipe }
            | ManagedAction::Apply { recipe }
            | ManagedAction::Update { recipe, .. } => Some(recipe),
            _ => None,
        };
        if recipe.is_some_and(|r| !milkdrift_contracts::is_canonical_blake3_digest(&r.digest)) {
            return Err(invalid("recipe digest must be a canonical BLAKE3 digest"));
        }
        if let ManagedAction::Resolve {
            use_id,
            expected_claim,
        } = &self.action
            && (use_id.len() != 64
                || !milkdrift_contracts::is_canonical_blake3_digest(&format!("b3_{use_id}"))
                || *expected_claim == 0)
        {
            return Err(invalid(
                "resolution requires an exact use and nonzero claim",
            ));
        }
        if let ManagedAction::Handoff { transfer } | ManagedAction::Return { transfer, .. } =
            &self.action
            && (transfer.installation != self.installation
                || transfer.generation == 0
                || transfer.parent_claim == 0
                || transfer.child_claim == 0
                || transfer.parent == transfer.child
                || !milkdrift_contracts::is_canonical_blake3_digest(&format!(
                    "b3_{}",
                    transfer.parent
                ))
                || !milkdrift_contracts::is_canonical_blake3_digest(&format!(
                    "b3_{}",
                    transfer.child
                ))
                || transfer.association.is_empty()
                || transfer.association.len() > 256)
        {
            return Err(invalid("invalid exact editing transfer"));
        }
        Ok(())
    }

    /// Stable operation selector used by the ordinary grant evaluator for every caller.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self.action {
            ManagedAction::Prepare { .. } => "resource.prepare",
            ManagedAction::Apply { .. } => "resource.apply",
            ManagedAction::Inspect {} => "resource.inspect",
            ManagedAction::Start {} => "resource.start",
            ManagedAction::Stop {} => "resource.stop",
            ManagedAction::Update { .. } => "resource.update",
            ManagedAction::Preserve { .. } => "resource.preserve",
            ManagedAction::Remove {} => "resource.remove",
            ManagedAction::Recover {} => "resource.recover",
            ManagedAction::Resolve { .. } => "resource.resolve",
            ManagedAction::Handoff { .. } => "resource.handoff",
            ManagedAction::Return { .. } => "resource.return",
        }
    }
}

/// Ownership controls removal, independently of where a resource happens to be located.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOwnership {
    /// Created with this installation's exact ownership marker.
    Owned,
    /// Shared prerequisite; never deleted by installation removal.
    Shared,
    /// Endpoint or storage owned elsewhere; never stopped or deleted here.
    Attached,
}

/// Purpose determines use conflicts, without exposing a platform mechanism to the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedResourceKind {
    /// Mutable source, documentation and experimental tools.
    WorkingArea,
    /// Candidate construction/output storage.
    Staging,
    /// Persistent application data.
    Data,
    /// Independently supervised endpoint.
    Service,
    /// Exact shared implementation or model input.
    Prerequisite,
}

/// Exact dependency retained by a published capability generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequirement {
    /// Resource within the installation.
    pub resource: ManagedName,
    /// Whether the operation needs the one exclusive editing claim.
    pub mutation: bool,
}

/// Descriptor-owned resource dependency. Accepted snapshots carry this unchanged across restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedBinding {
    /// Exact installation identity.
    pub installation: ManagedName,
    /// Immutable deployed configuration generation.
    pub generation: u64,
    /// Exact approved effective configuration, including implementation bytes.
    pub recipe_digest: String,
    /// All dependencies are acquired atomically in resource-name order.
    pub resources: Vec<ResourceRequirement>,
}

impl ManagedBinding {
    /// Validate finite, unique dependencies before use acquisition or durable decoding.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.generation == 0
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.recipe_digest)
            || self.resources.is_empty()
            || self.resources.len() > MAX_MANAGED_RESOURCES
            || self
                .resources
                .iter()
                .map(|r| &r.resource)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.resources.len()
        {
            return Err(invalid("invalid managed generation dependencies"));
        }
        Ok(())
    }
}

/// One resource projected from the durable inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedResourceView {
    /// Local resource name.
    pub name: ManagedName,
    /// Semantic purpose.
    pub kind: ManagedResourceKind,
    /// Removal authority classification.
    pub ownership: ResourceOwnership,
    /// Exact planned platform identity; not a reusable PID.
    pub identity: String,
    /// Removal disposition for mutable contents.
    pub disposition: DataDisposition,
}

/// A use that prevents maintenance or conflicts with an editing request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedBlocker {
    /// Stable use identity used for guarded resolution.
    pub use_id: String,
    /// Local attempt or serving execution reference, not an invented second journal.
    pub execution: String,
    /// Exact generation protected by the lifetime hold.
    pub generation: u64,
    /// Monotonic editing claim revision.
    pub claim: u64,
    /// Resource names for which this use currently owns mutation.
    pub editing: Vec<ManagedName>,
    /// Durable stage, including suspended or physically stopped states.
    pub state: String,
}

/// Bounded diagnosis and current resource inventory returned through both caller paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedResponse {
    /// Current response schema.
    pub schema_version: u32,
    /// Requested installation.
    pub installation: ManagedName,
    /// Version to use for the next mutation.
    pub version: u64,
    /// Last verified deployed generation; candidates do not replace it early.
    pub generation: u64,
    /// Lifecycle or preview result.
    pub state: String,
    /// Last committed desired service state; absent for a preparation preview.
    pub desired_running: Option<bool>,
    /// Latest observed service state; absence is not a stopped-service claim.
    pub observed_running: Option<bool>,
    /// Exact approved recipe, if one is retained.
    pub recipe: Option<RecipeReference>,
    /// Owned, attached and shared identities.
    pub resources: Vec<ManagedResourceView>,
    /// Every unresolved use, bounded by admission.
    pub blockers: Vec<ManagedBlocker>,
    /// Interrupted operation identity, if any.
    pub pending: Option<String>,
    /// Verified descriptor candidates eligible for registry projection; consult capability health for availability.
    pub capabilities: Vec<CapabilityId>,
    /// Bounded platform observations and refusal diagnostics without secrets.
    pub diagnostics: Vec<String>,
}

fn invalid(message: &str) -> ContractError {
    ContractError::InvalidIdentity {
        type_name: "managed resource contract",
        reason: message.to_owned(),
    }
}

/// Exact accepted parent/child relationship, checked against authoritative execution facts in store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditingHandoff {
    /// Resource owner.
    pub installation: ManagedName,
    /// Lifetime generation both users require.
    pub generation: u64,
    /// Already accepted parent invocation.
    pub parent: String,
    /// Already accepted child invocation.
    pub child: String,
    /// Parent claim expected at transfer (or return after its increment).
    pub parent_claim: u64,
    /// Child claim expected at transfer/return.
    pub child_claim: u64,
    /// Exact runtime-owned parent/child association evidence; no actor-name reentrancy.
    pub association: String,
}

impl ManagedResponse {
    /// Check bounded response facts and target correlation before a client exposes the result.
    pub fn validate_for(&self, request: &ManagedRequest) -> Result<(), ContractError> {
        if self.schema_version != MANAGED_SCHEMA_VERSION
            || self.installation != request.installation
            || !matches!(
                self.state.as_str(),
                "prepared" | "running" | "stopped" | "pending" | "removed" | "drift"
            )
            || self.resources.len() > MAX_MANAGED_RESOURCES
            || self.blockers.len() > MAX_MANAGED_USES
            || self.capabilities.len() > 8
            || self.diagnostics.len() > 32
            || self.diagnostics.iter().any(|d| d.len() > 512)
            || self
                .resources
                .iter()
                .any(|r| r.identity.is_empty() || r.identity.len() > 4096)
            || self
                .pending
                .as_ref()
                .is_some_and(|d| !milkdrift_contracts::is_canonical_blake3_digest(d))
            || self
                .recipe
                .as_ref()
                .is_some_and(|r| !milkdrift_contracts::is_canonical_blake3_digest(&r.digest))
            || self.blockers.iter().any(|b| {
                b.claim == 0
                    || b.generation == 0
                    || b.execution.len() > 512
                    || !milkdrift_contracts::is_canonical_blake3_digest(&format!("b3_{}", b.use_id))
                    || b.editing.len() > MAX_MANAGED_RESOURCES
                    || b.state.len() > 32
            })
        {
            return Err(invalid("invalid or mismatched managed response"));
        }
        Ok(())
    }
}
