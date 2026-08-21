//! Pure, deterministic projection of one run's authoritative event history.
//!
//! This module deliberately has no persistence, clock, ID-generation, registry, or
//! executor dependency. It accepts already-decoded event envelopes and either
//! projects every fact or rejects the history at the first point where doing so
//! would require guessing.

pub(super) mod serde_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord,
        V: Serialize,
        S: Serializer,
    {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let entries = Vec::<(K, V)>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            if map.insert(key, value).is_some() {
                return Err(D::Error::custom(
                    "projection snapshot map contains a duplicate key",
                ));
            }
        }
        Ok(map)
    }
}

mod apply_core;
mod apply_reconciliation;
mod apply_structured;
mod helpers;
mod node;
mod reconciliation;
mod replay;
mod run;
mod structured;

pub use node::{
    AttemptState, AttemptTerminal, CapabilityResolution, DeterministicNodeTerminalProjection,
    ExternalOutcomeObligation, LateTerminalEvidence, LeaseProjection, LeaseState,
    NodeAttemptProjection, NodeExecutionCancellationProjection, NodeExecutionProjection,
    NodeExecutionState, ProgressObservation, PublishedNodeOutput, RetainedExternalOutcome,
    RetryProjection, RetryState, SideEffectClassification, TimerCancellationProjection,
    TimerProjection, TimerPurpose, TimerState,
};
pub use reconciliation::{
    ReconciliationCancellationProjection, ReconciliationDecision, ReconciliationPlanProjection,
    ReconciliationProjection, ReconciliationRemediationProjection, ReconciliationRequestProjection,
    ReconciliationRequestState, RecoveryDecision, RecoveryObservation, RecoveryProjection,
    RemediationProjection,
};
pub use run::{
    ResourceUsage, RevisionPin, RunCancellation, RunLifecycle, RunProjection,
    RunTerminalProjection, RunTerminationIntent,
};
pub use structured::{
    BranchProjection, BranchState, IterationProjection, IterationState, JoinProjection,
    RepeatContinuationDecisionProjection, RepeatContinuationProjection,
    RepeatContinuationRequestProjection, RepeatTermination, SignalProjection,
    SubworkflowOutputImport, SubworkflowProjection, SubworkflowState, WaitCancellationProjection,
    WaitProjection,
};

#[cfg(test)]
mod tests;
