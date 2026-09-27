//! Removal may consume only the identity and preservation policy this scenario observed.
use milkdrift_capability::managed::{DataDisposition, ManagedResponse};
use milkdrift_evidence::{EvidenceResult, application::ensure};
use serde_json::Value;

pub(super) fn unchanged(current: &Value, expected: &Value) -> EvidenceResult<ManagedResponse> {
    let current: ManagedResponse = serde_json::from_value(current.clone())?;
    let expected: ManagedResponse = serde_json::from_value(expected.clone())?;
    ensure(
        current.installation == expected.installation
            && current.generation == expected.generation
            && current.recipe == expected.recipe
            && current.accepted_evaluation == expected.accepted_evaluation
            && current.resources == expected.resources
            && !current.resources.is_empty()
            && current
                .resources
                .iter()
                .all(|r| r.disposition == DataDisposition::Preserve),
        &format!(
            "study installation {} changed identity or preservation; refuse automatic cleanup",
            current.installation
        ),
    )?;
    ensure(
        current.pending.is_none() && current.blockers.is_empty(),
        &format!(
            "study installation {} has unresolved use; inspect it before cleanup",
            current.installation
        ),
    )?;
    Ok(current)
}
