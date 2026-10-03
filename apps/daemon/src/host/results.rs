//! Compose an operator view from the existing run, attempt, revision and artifact owners.

use super::read_model::{internal, public_control, public_persistence};
use super::{ActorSession, Owner, PublicFailure};
use milkdrift_authority::{AuthorityOperation, RequestedResourceFacts};
use milkdrift_control_protocol::{ErrorCode, RunOutputRead, RunResultRead};
use milkdrift_persistence::{RunOutcome, WorkspaceStore};
use milkdrift_workspace::RunId;

// A compact operator read, not a lifetime history query. Exact reads retain the omitted detail.
const FRONTIER_ITEMS: usize = 32;
const PREVIEW_BYTES: u32 = 4096;

fn optional<T>(value: Result<T, PublicFailure>) -> Result<Option<T>, PublicFailure> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error) if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::NotFound) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

impl Owner {
    pub(super) fn run_result(
        &mut self,
        session: &ActorSession,
        run: &str,
    ) -> Result<RunResultRead, PublicFailure> {
        let mut read = self.run_read(session, run)?;
        let mut truncated = read.nodes.len() > FRONTIER_ITEMS;
        // Keep active work first without claiming that absent settled occurrences never existed.
        read.nodes
            .sort_by_key(|node| node.state.starts_with("terminal"));
        read.nodes.truncate(FRONTIER_ITEMS);
        for node in &mut read.nodes {
            if let Some(attempt) = &node.latest_attempt_id {
                node.latest_attempt = optional(self.attempt_read(session, run, attempt))?;
            }
        }
        let definition = read
            .revision_id
            .as_ref()
            .map(|revision| optional(self.revision(session, revision)))
            .transpose()?
            .flatten();
        let (workflow_name, version) = if let Some(definition) = definition {
            let (_, revision) = milkdrift_blueprint::BlueprintRevisionDocument::from_json(
                &serde_json::to_vec(&definition.document).map_err(|_| internal())?,
            )
            .map_err(|_| internal())?;
            (
                Some(revision.semantic().metadata().name().to_owned()),
                Some(definition.summary.lineage_sequence),
            )
        } else {
            (None, None)
        };
        let run_id = RunId::new(run).map_err(|_| internal())?;
        let projection = self
            .workflow()?
            .runtime
            .projection(&run_id)
            .map_err(|error| public_control(error.into()))?;
        let mut outputs = Vec::new();
        let mut outputs_restricted = false;
        if let Some(terminal) = projection
            .terminal()
            .filter(|terminal| terminal.outcome() == RunOutcome::Succeeded)
        {
            truncated |= terminal.outputs().len() > FRONTIER_ITEMS;
            for reference in terminal.outputs().iter().take(FRONTIER_ITEMS) {
                let mut resources = RequestedResourceFacts::empty();
                resources.run = Some(run_id.clone());
                resources.workflow = projection.workflow().cloned();
                resources.workspace_scope = Some(reference.scope().scope().clone());
                if optional(self.authorize(
                    session,
                    AuthorityOperation::ReadWorkspaceValue,
                    resources,
                    "read:run-result",
                ))?
                .is_none()
                {
                    outputs_restricted = true;
                    continue;
                }
                let entry = self
                    .store
                    .value(reference)
                    .map_err(public_persistence)?
                    .ok_or_else(internal)?;
                let Some(artifact) = entry.value().as_artifact() else {
                    continue;
                };
                let Some(metadata) =
                    optional(self.artifact_metadata(session, artifact.artifact().as_str()))?
                else {
                    outputs_restricted = true;
                    continue;
                };
                if metadata.digest != artifact.digest().to_hex()
                    || metadata.size != artifact.size_bytes()
                {
                    return Err(internal());
                }
                let mut preview = None;
                let mut preview_truncated = false;
                if metadata.content_type.starts_with("text/")
                    && let Some(chunk) = optional(self.artifact_range(
                        session,
                        &metadata.artifact_id,
                        0,
                        PREVIEW_BYTES,
                        "result-preview",
                    ))?
                {
                    preview_truncated = !chunk.end;
                    let end = match std::str::from_utf8(&chunk.bytes) {
                        Ok(_) => chunk.bytes.len(),
                        Err(error) if error.error_len().is_none() && !chunk.end => {
                            error.valid_up_to()
                        }
                        Err(_) => 0,
                    };
                    preview = std::str::from_utf8(&chunk.bytes[..end])
                        .ok()
                        .map(str::to_owned);
                }
                outputs.push(RunOutputRead {
                    name: reference.key().as_str().to_owned(),
                    artifact: metadata,
                    preview,
                    preview_truncated,
                });
            }
        }
        let mut actions = Vec::new();
        let candidates = match read.lifecycle.as_str() {
            "running" => vec![
                ("pause", AuthorityOperation::Pause),
                ("cancel", AuthorityOperation::Cancel),
            ],
            "paused" => vec![
                ("resume", AuthorityOperation::Resume),
                ("propose", AuthorityOperation::Propose),
                ("signal", AuthorityOperation::DeliverSignal),
                ("cancel", AuthorityOperation::Cancel),
            ],
            _ => vec![],
        };
        for (name, operation) in candidates {
            if optional(self.authorize_run_read(session, run, operation, "read:permitted-action"))?
                .is_some()
            {
                actions.push(name.to_owned());
            }
        }
        if read.terminal.is_some() {
            let mut resources = RequestedResourceFacts::empty();
            resources.workflow = projection.workflow().cloned();
            if optional(self.authorize(
                session,
                AuthorityOperation::CreateRun,
                resources.clone(),
                "read:linked-run",
            ))?
            .is_some()
                && optional(self.authorize(
                    session,
                    AuthorityOperation::StartRun,
                    resources,
                    "read:linked-run",
                ))?
                .is_some()
            {
                actions.push("start_linked_run".into());
            }
        }
        Ok(RunResultRead {
            run: read,
            workflow_name,
            version,
            truncated,
            outputs,
            outputs_restricted,
            actions,
        })
    }
}
