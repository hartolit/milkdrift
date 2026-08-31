use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use milkdrift_blueprint::NodeId;
use milkdrift_capability::{
    ArtifactReference, CapabilityObservation, InvocationEvent, InvocationId, InvocationRequest,
    ResolvedCapabilitySnapshot, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterExecutionContext, AdapterReporter, CapabilityHost,
    CapabilitySelectionPolicy, HostConfig, InMemorySecretResolver, InputMaterialization,
    InvocationDataAccess, InvocationDataError, MaterializationLimits, MaterializedExecution,
};
use milkdrift_local_process::{LocalProcessAdapter, PlatformSupport, ProcessProfileDocument};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;

use crate::{EvidenceResult, ScenarioMeasurement};

/// Exercises the production artifact publication and bounded range-read path.
pub fn artifact_range_read() -> EvidenceResult<ScenarioMeasurement> {
    crate::persistence::artifact_publication()
}

/// Exercises both production model-provider bounded stream parsers with fixed fixtures.
pub fn model_stream_parsers() -> EvidenceResult<ScenarioMeasurement> {
    let evidence =
        milkdrift_model_provider::exercise_stream_fixtures(1_024).map_err(std::io::Error::other)?;
    let encoded = serde_json::to_vec(&evidence)?;
    Ok(ScenarioMeasurement::new(
        "adapters/model_stream_parsers_2048_responses",
        evidence.responses,
        evidence.input_bytes,
        &encoded,
    ))
}

/// Executes a byte-pinned local-process fixture and drains bounded stdout/stderr streams.
pub fn local_process_stream_drain() -> EvidenceResult<ScenarioMeasurement> {
    let executable = std::env::var_os("MILKDRIFT_EVIDENCE_PROCESS_HELPER")
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::other(
                "MILKDRIFT_EVIDENCE_PROCESS_HELPER must name the built evidence-process-helper",
            )
        })?;
    let executable = executable.canonicalize()?;
    let executable_bytes = fs::read(&executable)?;
    let data = Arc::new(EvidenceDataAccess::new()?);
    let executable_root = executable
        .parent()
        .ok_or_else(|| std::io::Error::other("process helper has no parent"))?;
    let profile_value = serde_json::json!({
        "schema_version": 2,
        "profile": {
            "profile_id": "operational-evidence-process",
            "revision": 1,
            "capability": "local-process-operational-evidence",
            "descriptor_revision": 1,
            "provider_profile": null,
            "operation": "process.execute",
            "side_effect": "read_only",
            "idempotency": "unsupported",
            "cancellation": "best_effort",
            "trust_class": "trusted_host_process",
            "executable": executable,
            "implementation": {
                "content_digest": format!("b3_{}", blake3::hash(&executable_bytes)),
                "size_bytes": executable_bytes.len(),
                "package_revision": "operational-evidence-helper-v1",
                "documentation_reference": "urn:milkdrift:operational-evidence-helper-v1"
            },
            "arguments": ["emit"],
            "substitutions": {},
            "working_directory": { "type": "isolated_root" },
            "filesystem_roots": [
                { "path": executable_root, "access": "execute" },
                { "path": data.root(), "access": "read_write" }
            ],
            "inputs": [],
            "environment": {
                "allowed_non_secret": [],
                "secrets": {},
                "max_value_bytes": 4096
            },
            "stdin": { "type": "disabled" },
            "stdout": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 32,
                "overflow_action": "continue_truncated",
                "artifact_name": "stdout"
            },
            "stderr": {
                "max_capture_bytes": 1048576,
                "stream_progress": true,
                "max_progress_events": 32,
                "overflow_action": "continue_truncated",
                "artifact_name": "stderr"
            },
            "outputs": [],
            "limits": {
                "max_argv_entries": 64,
                "max_argv_bytes": 65536,
                "max_children_observed": 32,
                "max_files": 64,
                "max_file_bytes": 2097152,
                "max_total_materialized_bytes": 4194304,
                "max_path_bytes": 4096,
                "max_directory_depth": 32,
                "artifact_chunk_bytes": 65536,
                "max_output_files": 64,
                "max_total_output_bytes": 4194304,
                "wall_timeout_ms": 10000,
                "graceful_termination_ms": 200,
                "forced_termination_ms": 200,
                "heartbeat_interval_ms": 1000
            },
            "restart": "retain_uncertain",
            "platform": PlatformSupport::current(),
            "max_concurrent": 4,
            "extensions": {}
        }
    });
    let profile =
        ProcessProfileDocument::from_json(&serde_json::to_vec(&profile_value)?)?.into_profile();
    let operation = profile.operation().clone();
    let adapter = Arc::new(LocalProcessAdapter::new(
        profile.clone(),
        Arc::clone(&data) as Arc<dyn InvocationDataAccess>,
        Arc::new(InMemorySecretResolver::new()),
    )?);
    let descriptor = adapter.descriptor().clone();
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 2,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    host.register(
        descriptor.clone(),
        adapter,
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            1,
            true,
            0,
            "evidence fixture ready",
        )?),
    )?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-operational-evidence")?,
        profile.capability().clone(),
        operation,
        None,
        None,
        Vec::new(),
        BTreeMap::new(),
    )?;
    let reporter = EvidenceReporter::default();
    let context = AdapterExecutionContext::new(
        RunId::new("run-operational-evidence")?,
        serde_json::from_value(serde_json::json!(format!("rev_{}", "0".repeat(64))))?,
        NodeId::new("node-operational-evidence")?,
        NodeExecutionId::new("execution-operational-evidence")?,
        AttemptId::new("attempt-operational-evidence")?,
    );
    host.execute_exact_with_context(&snapshot, &request, &context, &reporter)?;
    let events = reporter.events()?;
    let terminal = events.iter().find_map(|event| {
        event
            .kind()
            .terminal()
            .map(milkdrift_capability::InvocationTerminal::status)
    });
    if terminal != Some(TerminalStatus::Success) {
        return Err(std::io::Error::other("local process fixture did not succeed").into());
    }
    let stdout = data
        .output("stdout")?
        .ok_or_else(|| std::io::Error::other("stdout missing"))?;
    let stderr = data
        .output("stderr")?
        .ok_or_else(|| std::io::Error::other("stderr missing"))?;
    if stdout.len() != 262_144 || stderr.len() != 262_144 {
        return Err(std::io::Error::other("local process stream capture was incomplete").into());
    }
    let encoded = serde_json::to_vec(&(
        events.clone(),
        blake3::hash(&stdout).to_hex().to_string(),
        blake3::hash(&stderr).to_hex().to_string(),
    ))?;
    host.shutdown()?;
    Ok(ScenarioMeasurement::new(
        "adapters/local_process_stdout_stderr_524288_bytes",
        u64::try_from(events.len())?,
        u64::try_from(stdout.len().saturating_add(stderr.len()))?,
        &encoded,
    ))
}

pub(crate) fn peer_storage_turnover(
    executions: u32,
) -> EvidenceResult<crate::peer::PeerTurnoverEvidence> {
    crate::peer::peer_storage_turnover(executions)
}

#[derive(Default)]
struct EvidenceReporter {
    events: Mutex<Vec<InvocationEvent>>,
}

impl EvidenceReporter {
    fn events(&self) -> EvidenceResult<Vec<InvocationEvent>> {
        Ok(self
            .events
            .lock()
            .map_err(|_| std::io::Error::other("reporter lock poisoned"))?
            .clone())
    }
}

impl AdapterReporter for EvidenceReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.events
            .lock()
            .map_err(|_| AdapterError::external_failure("reporter lock poisoned"))?
            .push(event);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

struct EvidenceWorkspace {
    _owner: tempfile::TempDir,
    root: PathBuf,
}

impl MaterializedExecution for EvidenceWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    fn input_path(&self, _input_name: &str) -> Option<&Path> {
        None
    }
}

struct EvidenceDataAccess {
    _owner: tempfile::TempDir,
    root: PathBuf,
    outputs: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl EvidenceDataAccess {
    fn new() -> EvidenceResult<Self> {
        let owner = tempfile::tempdir()?;
        let root = owner.path().canonicalize()?;
        Ok(Self {
            _owner: owner,
            root,
            outputs: Mutex::new(BTreeMap::new()),
        })
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn output(&self, name: &str) -> EvidenceResult<Option<Vec<u8>>> {
        Ok(self
            .outputs
            .lock()
            .map_err(|_| std::io::Error::other("output lock poisoned"))?
            .get(name)
            .cloned())
    }

    fn publish(&self, name: &str, bytes: &[u8]) -> Result<ArtifactReference, InvocationDataError> {
        self.outputs
            .lock()
            .map_err(|_| InvocationDataError::Publication("output lock poisoned".to_owned()))?
            .insert(name.to_owned(), bytes.to_vec());
        ArtifactReference::new(
            format!("evidence-{name}"),
            blake3::hash(bytes).to_hex().to_string(),
            Some("application/octet-stream".to_owned()),
            Some(bytes.len() as u64),
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))
    }
}

impl InvocationDataAccess for EvidenceDataAccess {
    fn materialize(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        _inputs: &[InputMaterialization],
        _limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError> {
        let owner = tempfile::tempdir_in(&self.root)
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        let root = owner
            .path()
            .canonicalize()
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        Ok(Box::new(EvidenceWorkspace {
            _owner: owner,
            root,
        }))
    }

    fn publish_file(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        workspace: &dyn MaterializedExecution,
        output_name: &str,
        relative_path: &Path,
        _media_type: &str,
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        let bytes = fs::read(workspace.root().join(relative_path))
            .map_err(|error| InvocationDataError::Filesystem(error.to_string()))?;
        self.publish(output_name, &bytes)
    }

    fn publish_bytes(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        output_name: &str,
        _media_type: &str,
        bytes: &[u8],
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        self.publish(output_name, bytes)
    }
}
