//! Independent ordering is explicit; retained arrays preserve their earlier combined meaning.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{TaskId, TaskInputBinding};

/// What must happen before an independently ordered task may start.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOrderCondition {
    /// The predecessor must succeed, or have an explicitly adopted successful correction.
    Success,
    /// Success, failure or proven stop is enough; uncertainty never satisfies this condition.
    Settled,
}

/// New definitions carry independent order conditions; input bindings supply their own readiness.
/// Arrays exist only to read retained definitions without rewriting their historical predicates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskDependencies {
    /// Historical merged dependencies, whose predicate depends on that definition's input bindings.
    Legacy(BTreeSet<TaskId>),
    /// Independent conditions; removing a data binding cannot remove or weaken one.
    Ordered(BTreeMap<TaskId, TaskOrderCondition>),
}

impl Default for TaskDependencies {
    fn default() -> Self {
        Self::Ordered(BTreeMap::new())
    }
}

impl TaskDependencies {
    /// Whether this is retained historical meaning rather than a new authoring representation.
    #[must_use]
    pub const fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    /// Every predecessor named by independent or historical ordering.
    pub fn references(&self) -> impl Iterator<Item = &TaskId> {
        let (legacy, ordered) = match self {
            Self::Legacy(values) => (Some(values), None),
            Self::Ordered(values) => (None, Some(values)),
        };
        legacy
            .into_iter()
            .flat_map(BTreeSet::iter)
            .chain(ordered.into_iter().flat_map(BTreeMap::keys))
    }

    /// Exposes the exact retained ordering for inspection or an explicit future revision.
    /// Use the original definition's bindings before changing them; guessing the intent of a
    /// historical merged edge would silently change the earlier success/settlement predicate.
    #[must_use]
    pub fn conditions(
        &self,
        inputs: &BTreeMap<String, TaskInputBinding>,
    ) -> BTreeMap<TaskId, TaskOrderCondition> {
        match self {
            Self::Ordered(conditions) => conditions.clone(),
            Self::Legacy(dependencies) => dependencies
                .iter()
                .map(|task| {
                    let settled = inputs.values().any(|binding| {
                        matches!(binding, TaskInputBinding::SettledOutcome { task: source } if source == task)
                            || matches!(binding, TaskInputBinding::TaskOutput { task: source, optional: true, .. } if source == task)
                    });
                    (task.clone(), if settled { TaskOrderCondition::Settled } else { TaskOrderCondition::Success })
                })
                .collect(),
        }
    }
}
