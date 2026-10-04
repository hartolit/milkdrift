//! Failure-only public reads. Retained bundles deliberately exclude payloads and raw stores.
use super::support::*;
use milkdrift_control_protocol::{AttemptRead, RunRead};
use milkdrift_peer_protocol::{ObservationPage, PeerExecutionId};
use serde_json::{Value, json};
use std::{future::Future, io::Write as _, path::Path, time::Instant};

const COLLECTION_TIME: Duration = Duration::from_secs(3);
const READ_TIME: Duration = Duration::from_millis(250);
const MAX_READS: usize = 16;
const MAX_ITEMS: usize = 8;
const MAX_BUNDLE_BYTES: usize = 65_536;

pub(super) struct FailureSnapshot {
    started: Instant,
    deadline: Duration,
    test: String,
    last: Value,
    last_progress_ms: Option<u128>,
    reads: usize,
    collected: Vec<Value>,
}

impl FailureSnapshot {
    pub(super) fn new(deadline: Duration) -> Self {
        Self {
            started: Instant::now(),
            deadline,
            test: std::thread::current()
                .name()
                .unwrap_or("unnamed integration test")
                .into(),
            last: json!({"unavailable":"no observation yet"}),
            last_progress_ms: None,
            reads: 0,
            collected: vec![],
        }
    }

    pub(super) fn run(&mut self, state: &RunRead) {
        self.observe(run_summary(state));
    }

    pub(super) fn invocation(&mut self, page: &ObservationPage) {
        self.observe(json!({"execution":page.execution,"status":page.status,
            "sequence":page.next_sequence,"observations":page.observations.len(),
            "terminal":page.terminal,"closed":page.closed}));
    }

    fn observe(&mut self, value: Value) {
        if self.last != value {
            self.last_progress_ms = Some(self.started.elapsed().as_millis());
        }
        self.last = value;
    }

    // ControlClient bounds each HTTP response while streaming, before JSON decoding. Limiting
    // reads also bounds total received bytes, including refused/oversized/error responses.
    async fn read<T>(
        &mut self,
        end: tokio::time::Instant,
        label: &str,
        future: impl Future<Output = Result<T, ClientError>>,
    ) -> Option<T> {
        if self.reads >= MAX_READS || tokio::time::Instant::now() >= end {
            self.collected
                .push(json!({"read":label,"unavailable":"collection limit"}));
            return None;
        }
        self.reads += 1;
        let read_end = end.min(tokio::time::Instant::now() + READ_TIME);
        let kind = match tokio::time::timeout_at(read_end, future).await {
            Ok(Ok(value)) => return Some(value),
            Err(_) | Ok(Err(ClientError::Timeout)) => "read deadline",
            Ok(Err(ClientError::Api(_))) => "public read refused",
            Ok(Err(_)) => "transport or decoding failure",
        };
        self.collected
            .push(json!({"read":label,"unavailable":kind}));
        None
    }

    pub(super) async fn capture(
        mut self,
        client: &ControlClient,
        run: Option<&str>,
        workflow: Option<&str>,
        execution: Option<&PeerExecutionId>,
        fixture_requests: Option<usize>,
        reason: &str,
    ) -> String {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/control-plane-failures");
        match self
            .collect(client, run, workflow, execution, fixture_requests, reason)
            .await
            .and_then(|document| retain(&root, document))
        {
            Ok(path) => format!("{reason}; diagnostics: {}", path.display()),
            Err(error) => format!("{reason}; diagnostic retention failed: {error}"),
        }
    }

    async fn collect(
        &mut self,
        client: &ControlClient,
        run: Option<&str>,
        workflow: Option<&str>,
        execution: Option<&PeerExecutionId>,
        fixture_requests: Option<usize>,
        reason: &str,
    ) -> TestResult<Value> {
        let collecting = Instant::now();
        let end = tokio::time::Instant::now() + COLLECTION_TIME;
        if let Some(health) = self.read(end, "health", client.health()).await {
            self.collected.push(json!({"health":{
                "state":health.state,"ready":health.ready,"draining":health.draining,
                "queued_requests":health.queued_requests,"request_queue_capacity":health.request_queue_capacity,
                "active_effects":health.active_effects,"serving_active":health.peer_executions.active_count,
                "serving_queued":health.peer_executions.dispatch_queued}}));
        }
        if let Some(execution) = execution
            && let Some(page) = self
                .read(
                    end,
                    "invocation",
                    client.invocation_observations(execution, 0, 8),
                )
                .await
        {
            self.collected.push(json!({"invocation":{"execution":page.execution,"status":page.status,
                    "sequence":page.next_sequence,"observations":page.observations.len(),"terminal":page.terminal}}));
        }
        let mut runs = run.map(str::to_owned).into_iter().collect::<Vec<_>>();
        if (workflow.is_some() || run.is_none())
            && let Some(page) = self
                .read(
                    end,
                    "runs",
                    client.runs(
                        None,
                        workflow,
                        &PageRequest {
                            limit: 8,
                            cursor: None,
                        },
                    ),
                )
                .await
        {
            self.collected
                .push(json!({"run_page_truncated":page.next_cursor.is_some()}));
            runs.extend(page.items.into_iter().map(|item| item.run_id));
        }
        runs.sort();
        runs.dedup();
        for run in runs.iter().take(MAX_ITEMS) {
            if let Some(state) = self.read(end, "run", client.run(run)).await {
                self.collected.push(run_summary(&state));
                for node in state.nodes.iter().take(MAX_ITEMS) {
                    if let Some(attempt) = &node.latest_attempt_id
                        && let Some(attempt) = self
                            .read(end, "attempt", client.attempt(run, attempt))
                            .await
                    {
                        self.collected
                            .push(json!({"run":run,"attempt":attempt_summary(&attempt)}));
                    }
                }
            }
        }
        let binary = std::env::current_exe().ok().and_then(|path| {
            let meta = fs::metadata(&path).ok()?;
            Some(json!({"name":path.file_name()?.to_str(),"bytes":meta.len(),
                "modified_unix_ms":meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis()}))
        });
        let source = env!("MILKDRIFT_SOURCE_REVISION");
        let document = json!({"test":self.test,"failure_class":reason,"elapsed_ms":self.started.elapsed().as_millis(),
            "deadline_ms":self.deadline.as_millis(),"last":self.last,"last_progress_ms":self.last_progress_ms,"fixture_requests":fixture_requests,
            "source":source,"binary":binary,"source_note":"local source and binary hashes belong to the invoking campaign log",
            "collection_ms":collecting.elapsed().as_millis(),"limits":{"time_ms":COLLECTION_TIME.as_millis(),
                "per_read_ms":READ_TIME.as_millis(),"reads":MAX_READS,"items_per_run":MAX_ITEMS,
                "response_bytes_per_read":milkdrift_control_protocol::MAX_DOCUMENT_BYTES,"bundle_bytes":MAX_BUNDLE_BYTES},
            "reads":self.reads,"runs_truncated":runs.len()>MAX_ITEMS,"collected":self.collected,
            "child_logs":"unavailable: in-process fixture; payload logs and stores are never exported"});
        Ok(document)
    }
}

fn retain(root: &Path, mut document: Value) -> TestResult<std::path::PathBuf> {
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    if bytes.len() > MAX_BUNDLE_BYTES {
        document
            .as_object_mut()
            .ok_or("diagnostic document is not an object")?
            .insert("collected".into(), json!({"truncated":"bundle byte limit"}));
        document
            .as_object_mut()
            .ok_or("diagnostic document is not an object")?
            .insert("last".into(), json!({"truncated":"bundle byte limit"}));
        bytes = serde_json::to_vec_pretty(&document)?;
    }
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err("diagnostic metadata exceeds byte limit".into());
    }
    fs::create_dir_all(root)?;
    let mut file = tempfile::Builder::new()
        .prefix("failure-")
        .suffix(".json")
        .tempfile_in(root)?;
    file.write_all(&bytes)?;
    let path = file.path().to_owned();
    file.persist(&path)?;
    Ok(path)
}

fn run_summary(state: &RunRead) -> Value {
    json!({"run":state.run_id,"workflow":state.workflow_id,"revision":state.revision_id,
        "sequence":state.sequence,"lifecycle":state.lifecycle,"terminal":state.terminal,
        "parent":state.published_source.as_ref().map(|source| json!({"type":source.get("type"),
            "run":source.get("run"),"attempt":source.get("attempt"),"execution":source.get("execution")})),
        "uncertainty_count":state.uncertainty_count,"nodes_truncated":state.nodes.len()>MAX_ITEMS,
        "nodes":state.nodes.iter().take(MAX_ITEMS).map(|node| json!({"node":node.node_id,
            "execution":node.execution_id,"state":node.state,"attempt":node.latest_attempt_id,
            "attempts":node.attempt_count})).collect::<Vec<_>>()})
}

fn attempt_summary(attempt: &AttemptRead) -> Value {
    json!({"attempt":attempt.attempt_id,"invocation":attempt.invocation_id,"state":attempt.state,
        "capability":attempt.capability_id,"operation":attempt.operation_contract.as_ref().map(|c| &c.operation),
        "entry_authorized":attempt.entry_authorization.as_ref().map(|a| a.allowed),
        "progress_observations":attempt.progress_observations,"progress_bytes":attempt.progress_bytes,
        "outputs":attempt.outputs.len(),"terminal":attempt.terminal,"uncertain":attempt.uncertain})
}

#[cfg(test)]
#[path = "diagnostics/tests.rs"]
mod tests;
