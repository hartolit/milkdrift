//! Current compact-projection lookup into the canonical attempt read shape.

use super::{LocatedAttempt, Owner};
use crate::host::{ActorSession, PublicFailure};

impl Owner {
    pub(super) fn current_attempt_read(
        &self,
        session: &ActorSession,
        run: &str,
        attempt: &str,
    ) -> Result<Option<LocatedAttempt>, PublicFailure> {
        Ok(self
            .run_read(session, run)?
            .nodes
            .into_iter()
            .find(|node| {
                node.latest_attempt
                    .as_ref()
                    .is_some_and(|value| value.attempt_id == attempt)
            })
            .and_then(|node| {
                node.latest_attempt.map(|value| LocatedAttempt {
                    node_id: node.node_id,
                    revision_id: node.revision_id,
                    value,
                })
            }))
    }
}
