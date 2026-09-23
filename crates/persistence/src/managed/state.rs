use super::ManagedUse;
use crate::PersistenceError;
use milkdrift_authority::AuthorityDecisionSnapshot;
use milkdrift_capability::managed::{
    MANAGED_SCHEMA_VERSION, MAX_MANAGED_RESOURCES, MAX_MANAGED_USES, ManagedName, ManagedRequest,
    ManagedResourceView, ManagedResponse, RecipeReference,
};
use milkdrift_capability::{BoundedJson, CapabilityDescriptor};
use serde::{Deserialize, Serialize};

/// Fully compiled approved configuration; only the selected platform adapter interprets its bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedSetup {
    /// Protected target contract, absent for ordinary development resources.
    pub protection: Option<ProtectedDeployment>,
    /// Immutable approved input reference.
    pub recipe: RecipeReference,
    /// Closed adapter protocol identifier, distinct from a general deployment language.
    pub mechanism: String,
    /// Exact deployment configuration retained for recovery and removal.
    pub configuration: BoundedJson,
    /// Platform/account/store identity; restored copies must not silently adopt another owner.
    pub platform_owner: String,
    /// Lifetime ownership token, persisted before any physical creation.
    pub ownership: String,
    /// Every owned, attached and shared resource with an explicit preservation rule.
    pub resources: Vec<ManagedResourceView>,
    /// Exact candidate descriptors, published only after platform verification.
    pub capabilities: Vec<CapabilityDescriptor>,
}

impl ApprovedSetup {
    /// Validate portable bounds; the platform additionally validates typed mechanism configuration.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if let Some(protection) = &self.protection {
            protection.validate()?;
        }
        if self.resources.is_empty()
            || self.resources.len() > MAX_MANAGED_RESOURCES
            || self.capabilities.len() > 8
            || self.mechanism.len() > 64
            || self.mechanism.is_empty()
            || self.platform_owner.len() > 256
            || self.platform_owner.is_empty()
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.ownership)
            || !milkdrift_contracts::is_canonical_blake3_digest(&self.recipe.digest)
            || self
                .resources
                .iter()
                .any(|r| r.identity.is_empty() || r.identity.len() > 4096)
            || self
                .resources
                .iter()
                .map(|r| &r.name)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.resources.len()
        {
            return Err(PersistenceError::InvalidDocument(
                "invalid approved managed setup".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Finite platform boundaries; each intent is durable before the corresponding external action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedStep {
    /// Verify rootless/session/storage prerequisites and immutable inputs.
    Prerequisites,
    /// Create exact owned working/staging/data identities if absent; never adopt a name match.
    PrepareStorage,
    /// Drain/stop the exact old owned service, preserving its data.
    StopService,
    /// Install exact generated definitions after checking existing ownership.
    Configure,
    /// Start the declared owned service through the external supervisor.
    StartService,
    /// Verify effective deployed identity, protection and service state.
    Verify,
    /// Remove exact obsolete owned definitions after stop proof.
    RemoveConfiguration,
    /// Apply the saved preservation plan to verified owned storage.
    RemoveStorage,
}

/// Latest observation is separate from approved desired state and immutable transition receipts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedObservation {
    /// Bounded exact evidence digest from adapter observation.
    pub digest: String,
    /// Human-readable finite result; never raw tool output or secret values.
    pub summary: String,
    /// Whether the declared service is currently running.
    pub running: bool,
}

/// Intended change supplied to the store after the semantic owner authorizes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedChange {
    /// Exact accepted authority, rechecked for protected service entry and recovery.
    pub entry_authorization: Option<AuthorityDecisionSnapshot>,
    /// Exact deterministic transition identity.
    pub identity: String,
    /// Complete candidate; the previous setup remains retained until verified completion.
    pub candidate: ApprovedSetup,
    /// Candidate generation, unchanged for stop/start/reapply.
    pub generation: u64,
    /// Closed bounded sequence; no arbitrary executable instructions.
    pub steps: Vec<ManagedStep>,
    /// Whether completion retires the installation namespace permanently.
    pub removing: bool,
    /// Requested final service state.
    pub running: bool,
}

/// Operational disposition of an accepted transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedChangePhase {
    /// The indexed next step is durably intended and may already have happened.
    Pending {},
    /// Recovery must inspect the same identity; no candidate is advertised healthy.
    Uncertain {
        /// Bounded redacted reason.
        diagnostic: String,
    },
    /// All boundaries and publication committed.
    Complete {},
}

/// Durable transition evidence retained independently of invoking operation history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTransition {
    /// Original accepted intent and exact identities.
    pub change: ManagedChange,
    /// Number of verified completed steps; the next step is already intended.
    pub next_step: u32,
    /// One bounded observation for each completed boundary.
    pub evidence: Vec<ManagedObservation>,
    /// Whether continuation is automatic or needs an authorized recovery request.
    pub phase: ManagedChangePhase,
}

/// Mutable inventory whose version changes with lifecycle and use claims, never with old receipts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationRecord {
    /// Exact durable schema.
    pub schema_version: u32,
    /// Never-reused installation namespace.
    pub name: ManagedName,
    /// Optimistic concurrency guard.
    pub version: u64,
    /// Last verified implementation/configuration generation.
    pub generation: u64,
    /// Currently approved and verified setup; absent before initial verification.
    pub current: Option<ApprovedSetup>,
    /// Pending replacement retains all candidate removal/recovery facts.
    pub pending: Option<ManagedTransition>,
    /// Last committed desired service state, independent of platform observations.
    pub desired_running: bool,
    /// Most recent verified observation, independent of intent.
    pub observation: Option<ManagedObservation>,
    /// Admission is closed during maintenance and after removal.
    pub admission_open: bool,
    /// Removed namespace remains a tombstone; identities are never silently recreated.
    pub removed: bool,
    /// Unresolved uses, bounded at accepted-work admission.
    pub uses: Vec<ManagedUse>,
}

impl InstallationRecord {
    /// Validate durable bounds and transition consistency before accepting stored state.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.schema_version != MANAGED_SCHEMA_VERSION
            || self.version == 0
            || self.uses.len() > MAX_MANAGED_USES
            || (self.admission_open
                && (self.removed || self.pending.is_some() || self.current.is_none()))
        {
            return Err(PersistenceError::InvalidDocument(
                "invalid managed inventory".to_owned(),
            ));
        }
        if let Some(current) = &self.current {
            current.validate()?;
        }
        for usage in &self.uses {
            usage.validate()?;
            if usage.binding.installation != self.name
                || usage.binding.generation != self.generation
            {
                return Err(PersistenceError::InvalidDocument(
                    "use names a different installation generation".to_owned(),
                ));
            }
        }
        if self
            .uses
            .iter()
            .map(|u| &u.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != self.uses.len()
        {
            return Err(PersistenceError::InvalidDocument(
                "duplicated managed use".to_owned(),
            ));
        }
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        if let Some(pending) = &self.pending {
            pending.change.candidate.validate()?;
            for observation in &pending.evidence {
                observation.validate()?;
            }
            if pending.change.steps.is_empty()
                || !milkdrift_contracts::is_canonical_blake3_digest(&pending.change.identity)
                || pending.change.generation == 0
                || matches!(&pending.phase, ManagedChangePhase::Uncertain { diagnostic } if diagnostic.len() > 512)
                || pending.change.steps.len() > 16
                || pending.next_step as usize != pending.evidence.len()
                || pending.next_step as usize > pending.change.steps.len()
            {
                return Err(PersistenceError::InvalidDocument(
                    "invalid managed transition".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Authorized caller projection, with no executable configuration or credential values.
    #[must_use]
    pub fn view(&self) -> ManagedResponse {
        let setup = self
            .current
            .as_ref()
            .or_else(|| self.pending.as_ref().map(|p| &p.change.candidate));
        ManagedResponse {
            accepted_evaluation: self
                .current
                .as_ref()
                .and_then(|s| s.protection.as_ref())
                .and_then(|p| p.evidence.as_ref())
                .map(|e| e.identity.clone()),
            evaluation: None,
            schema_version: MANAGED_SCHEMA_VERSION,
            installation: self.name.clone(),
            version: self.version,
            generation: self.generation,
            state: if self.removed {
                "removed"
            } else if self.pending.is_some() {
                "pending"
            } else if self.observation.as_ref().is_some_and(|o| o.running) {
                "running"
            } else {
                "stopped"
            }
            .to_owned(),
            desired_running: Some(self.desired_running),
            observed_running: self.observation.as_ref().map(|o| o.running),
            recipe: setup.map(|s| s.recipe.clone()),
            resources: setup.map(|s| s.resources.clone()).unwrap_or_default(),
            blockers: self.uses.iter().map(ManagedUse::view).collect(),
            pending: self.pending.as_ref().map(|p| p.change.identity.clone()),
            capabilities: if self.admission_open {
                self.current
                    .as_ref()
                    .map(|s| {
                        s.capabilities
                            .iter()
                            .map(|d| d.identity().clone())
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
            diagnostics: self
                .pending
                .as_ref()
                .and_then(|p| match &p.phase {
                    ManagedChangePhase::Uncertain { diagnostic } => Some(vec![diagnostic.clone()]),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    self.observation
                        .as_ref()
                        .map(|o| vec![o.summary.clone()])
                        .unwrap_or_default()
                }),
        }
    }
}

/// Immutable command acceptance, replayed exactly even after resource state advances or is removed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReceipt {
    /// Exact supported schema.
    pub schema_version: u32,
    /// Canonical complete caller request.
    pub request: ManagedRequest,
    /// Durable acceptance authority, independent of later disclosure authorization.
    pub authorization: AuthorityDecisionSnapshot,
    /// Exact immutable acceptance response. Inspect reads subsequent platform progress.
    pub response: ManagedResponse,
}

impl ManagedObservation {
    /// Verify bounded evidence before it becomes durable inventory.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if !milkdrift_contracts::is_canonical_blake3_digest(&self.digest)
            || self.summary.len() > 512
        {
            return Err(PersistenceError::InvalidDocument(
                "invalid managed observation".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Immutable protection attached to a resource's durable inventory. Raw lifecycle changes cannot
/// remove it; only an accepted publication can replace its candidate/evidence pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedDeployment {
    /// Exact accepted agreement; changing requirements requires a distinct target/run.
    pub agreement: String,
    /// Operator-owned check and producer policy.
    pub policy: milkdrift_authority::ProtectedEffectPolicy,
    /// Most recently published exact candidate, if any.
    pub evidence: Option<milkdrift_workspace::CandidateEvaluation>,
}
impl ProtectedDeployment {
    /// Validate policy and evidence identities before retaining or recovering configuration.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.policy
            .validate()
            .map_err(|e| PersistenceError::InvalidDocument(e.to_string()))?;
        if !milkdrift_contracts::is_canonical_blake3_digest(&self.agreement) {
            return Err(PersistenceError::InvalidDocument(
                "invalid protected agreement".to_owned(),
            ));
        }
        if let Some(e) = &self.evidence {
            e.validate()
                .map_err(|e| PersistenceError::InvalidDocument(e.to_string()))?;
            if e.subject.agreement != self.agreement
                || e.subject.policy
                    != self
                        .policy
                        .digest()
                        .map_err(|e| PersistenceError::InvalidDocument(e.to_string()))?
            {
                return Err(PersistenceError::InvalidDocument(
                    "protected evidence binding differs".to_owned(),
                ));
            }
        }
        Ok(())
    }
}
