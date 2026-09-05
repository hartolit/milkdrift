//! Independent reconciliation event fact validation.
use super::{
    super::MAX_RECONCILIATION_PLAN_ITEMS, super::kind::RunEventKind,
    super::model::ReconciliationClassification, ReferenceContext,
};
use crate::PersistenceError;

pub(super) fn validate(
    event: &RunEventKind,
    _context: &ReferenceContext<'_>,
) -> Result<(), PersistenceError> {
    if let RunEventKind::ReconciliationPlanRecorded { items, .. } = event {
        if items.len() > MAX_RECONCILIATION_PLAN_ITEMS {
            return Err(PersistenceError::Bounds {
                location: "event.reconciliation.items",
                reason: format!(
                    "at most {MAX_RECONCILIATION_PLAN_ITEMS} items are allowed so actions, application, and revision pin fit one atomic commit"
                ),
            });
        }
        let unique: std::collections::BTreeSet<_> = items
            .iter()
            .map(|item| (item.node.as_ref(), item.execution.as_ref()))
            .collect();
        let invalid_identity = items.iter().any(|item| {
            item.node.is_none()
                && item.execution.is_none()
                && item.classification
                    != ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
        });
        if unique.len() != items.len() || invalid_identity {
            return Err(PersistenceError::InvalidDocument(
                        "reconciliation items must be distinct; only workflow-interface incompatibilities may omit both node and execution"
                            .to_owned(),
                    ));
        }
    }
    Ok(())
}
