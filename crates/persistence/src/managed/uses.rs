use crate::{AttemptId, NodeExecutionId, RunSequence};
use milkdrift_capability::InvocationId;
use milkdrift_capability::managed::{ManagedBinding, ManagedBlocker, ManagedName};
use milkdrift_peer_protocol::PeerExecutionId;
use milkdrift_workspace::RunId;
use serde::{Deserialize, Serialize};

/// The existing owner of an accepted operation. This link is not another execution ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedExecution {
    /// One runtime-owned local attempt.
    Local {
        /// Owning run.
        run: RunId,
        /// Logical execution.
        execution: NodeExecutionId,
        /// Exact attempt.
        attempt: AttemptId,
        /// Invocation identity used by the adapter.
        invocation: InvocationId,
        /// Journal acceptance boundary.
        accepted_at: RunSequence,
    },
    /// One durable direct or delegated serving acceptance.
    Serving {
        /// Serving operation.
        execution: PeerExecutionId,
        /// Rewritten serving invocation identity.
        invocation: InvocationId,
    },
}

impl ManagedExecution {
    /// Exact executor-facing correlation identity.
    #[must_use]
    pub const fn invocation(&self) -> &InvocationId {
        match self {
            Self::Local { invocation, .. } | Self::Serving { invocation, .. } => invocation,
        }
    }
    /// Stable use key derived from the accepted invocation, without PID or lease reuse.
    #[must_use]
    pub fn use_id(&self) -> String {
        managed_use_id(self.invocation())
    }
}

/// Identical derivation at durable acceptance and adapter entry.
#[must_use]
pub fn managed_use_id(invocation: &InvocationId) -> String {
    blake3::hash(format!("milkdrift.managed-use.v1:{}", invocation.as_str()).as_bytes())
        .to_hex()
        .to_string()
}

/// Physical use status, separate from whether an operation returned a terminal result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedUsePhase {
    /// Accepted but no external entry intent has committed.
    Reserved {},
    /// Durable entry intent; platform creation may or may not have happened.
    Entered {
        /// Exact intended task/supervisor identity.
        physical_identity: String,
    },
    /// Authorized reclamation excludes late entry before any physical fence is attempted.
    Fencing {
        /// Exact intended task identity retained across interrupted resolution.
        physical_identity: String,
    },
    /// Adapter proved the writer stopped; its lifetime hold is still retained until settlement.
    Quiescent {
        /// Verified physical evidence.
        evidence: QuiescenceEvidence,
    },
    /// Parent cannot write while its exact child owns editing.
    Suspended {
        /// Exact child use identity.
        child: String,
        /// Physical proof preceding transfer.
        evidence: QuiescenceEvidence,
    },
}

/// Bound proof from an enforcing adapter, never from a caller's cancellation acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuiescenceEvidence {
    /// Exact physical identity from entry intent.
    pub physical_identity: String,
    /// Digest of inspected identity/absence and supervisor state.
    pub observation_digest: String,
    /// True when authorized fencing stopped resource use without establishing its outcome.
    pub disrupted: bool,
}

/// A lifetime hold and zero or more exclusive editing claims acquired coherently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedUse {
    /// Stable exact accepted invocation key.
    pub id: String,
    /// Existing authoritative execution link.
    pub execution: ManagedExecution,
    /// Exact frozen generation and complete resource set.
    pub binding: ManagedBinding,
    /// Monotonic claim generation, compared by every entry, transfer and return.
    pub claim: u64,
    /// Current editing claims; lifetime dependencies remain in `binding` while suspended.
    pub editing: Vec<ManagedName>,
    /// The owning execution committed final entry; only its no-entry refusal may release an unentered reservation.
    pub entry_committed: bool,
    /// Owning execution settled or became terminal; resource uncertainty may still be retained.
    pub execution_terminal: bool,
    /// Physical use state.
    pub phase: ManagedUsePhase,
    /// Exact parent use when mutation was inherited by guarded handoff.
    pub parent: Option<String>,
}

impl ManagedUse {
    /// Validate the exact accepted identity, finite claims and physical evidence.
    pub fn validate(&self) -> Result<(), crate::PersistenceError> {
        let invalid = || crate::PersistenceError::InvalidDocument("invalid managed use".to_owned());
        self.binding.validate().map_err(|_| invalid())?;
        if self.id != self.execution.use_id()
            || self.claim == 0
            || self.editing.len() > self.binding.resources.len()
            || self
                .editing
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.editing.len()
            || self.editing.iter().any(|name| {
                !self
                    .binding
                    .resources
                    .iter()
                    .any(|r| &r.resource == name && r.mutation)
            })
            || self
                .parent
                .as_ref()
                .is_some_and(|p| p == &self.id || !valid_use_id(p))
        {
            return Err(invalid());
        }
        let evidence = match &self.phase {
            ManagedUsePhase::Reserved {} => None,
            ManagedUsePhase::Entered { physical_identity }
            | ManagedUsePhase::Fencing { physical_identity } => {
                if physical_identity.is_empty() || physical_identity.len() > 4096 {
                    return Err(invalid());
                }
                None
            }
            ManagedUsePhase::Quiescent { evidence } => Some(evidence),
            ManagedUsePhase::Suspended { child, evidence } => {
                if !valid_use_id(child) || child == &self.id {
                    return Err(invalid());
                }
                Some(evidence)
            }
        };
        if evidence.is_some_and(|e| {
            e.physical_identity.is_empty()
                || e.physical_identity.len() > 4096
                || !milkdrift_contracts::is_canonical_blake3_digest(&e.observation_digest)
        }) {
            return Err(invalid());
        }
        Ok(())
    }

    /// Bounded blocker projection used by ordinary authorized inspection.
    #[must_use]
    pub fn view(&self) -> ManagedBlocker {
        ManagedBlocker {
            use_id: self.id.clone(),
            execution: match &self.execution {
                ManagedExecution::Local { run, attempt, .. } => {
                    format!("run:{run}/attempt:{attempt}")
                }
                ManagedExecution::Serving { execution, .. } => format!("serving:{execution}"),
            },
            generation: self.binding.generation,
            claim: self.claim,
            editing: self.editing.clone(),
            state: match self.phase {
                ManagedUsePhase::Reserved {} => "reserved",
                ManagedUsePhase::Entered { .. } => "entered",
                ManagedUsePhase::Fencing { .. } => "fencing",
                ManagedUsePhase::Quiescent { .. } => "quiescent",
                ManagedUsePhase::Suspended { .. } => "suspended",
            }
            .to_owned(),
        }
    }
}

fn valid_use_id(value: &str) -> bool {
    value.len() == 64 && milkdrift_contracts::is_canonical_blake3_digest(&format!("b3_{value}"))
}
