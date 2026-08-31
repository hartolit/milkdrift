//! Durable bridge path, budget, publication, and escape evidence.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CapabilityId, InputReference, InvocationId, InvocationRequest,
    InvocationValueReference, OperationId,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, InputMaterialization, InvocationDataAccess, MaterializationLimits,
    StoreInvocationDataAccess,
};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactStore, AtomicRunCommitRequest, AttemptId, CommandDisposition,
    CommandId, CommandReceipt, CommandResultDocument, EventId, EvidenceId, IndexedRunState,
    NodeExecutionId, PersistenceError, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal,
    RunSequence, RunSummaryIndex, TimestampMillis, WorkspaceAccounting,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, ContentDigest, MediaType, RunId, WorkspaceBudget, WorkspaceUsage,
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

fn context() -> TestResult<AdapterExecutionContext> {
    let revision: RevisionId = serde_json::from_value(json!(format!("rev_{}", "1".repeat(64))))?;
    Ok(AdapterExecutionContext::new(
        RunId::new("run-materialization")?,
        revision,
        NodeId::new("node-materialization")?,
        NodeExecutionId::new("execution-materialization")?,
        AttemptId::new("attempt-materialization")?,
    ))
}

fn request() -> TestResult<InvocationRequest> {
    Ok(InvocationRequest::new(
        InvocationId::new("invocation-materialization")?,
        CapabilityId::new("capability-materialization")?,
        OperationId::new("process.execute")?,
        None,
        None,
        vec![InputReference::new(
            "prompt",
            InvocationValueReference::Inline {
                value: BoundedJson::new(json!({"instruction": "safe"}))?,
            },
        )?],
        BTreeMap::new(),
    )?)
}

fn two_input_request() -> TestResult<InvocationRequest> {
    Ok(InvocationRequest::new(
        InvocationId::new("invocation-materialization-two-inputs")?,
        CapabilityId::new("capability-materialization")?,
        OperationId::new("process.execute")?,
        None,
        None,
        vec![
            InputReference::new(
                "first",
                InvocationValueReference::Inline {
                    value: BoundedJson::new(json!("aa"))?,
                },
            )?,
            InputReference::new(
                "second",
                InvocationValueReference::Inline {
                    value: BoundedJson::new(json!("bb"))?,
                },
            )?,
        ],
        BTreeMap::new(),
    )?)
}

fn expected_process_artifact(
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
    output_name: &str,
    media_type: &str,
    bytes: &[u8],
) -> TestResult<ArtifactReference> {
    let digest = ContentDigest::for_bytes(bytes);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.process-output-publication.v1\0");
    for component in [
        context.run().as_str().as_bytes(),
        context.revision().as_str().as_bytes(),
        context.node().as_str().as_bytes(),
        context.execution().as_str().as_bytes(),
        context.attempt().as_str().as_bytes(),
        request.invocation().as_str().as_bytes(),
        output_name.as_bytes(),
        digest.as_bytes(),
    ] {
        hasher.update(&(component.len() as u64).to_be_bytes());
        hasher.update(component);
    }
    Ok(ArtifactReference::new(
        ArtifactId::new(format!("process:{}", hasher.finalize().to_hex()))?,
        digest,
        MediaType::new(media_type)?,
        bytes.len() as u64,
    ))
}

fn limits() -> MaterializationLimits {
    MaterializationLimits {
        max_files: 8,
        max_file_bytes: 1024,
        max_total_bytes: 4096,
        max_path_bytes: 256,
        max_directory_depth: 8,
        chunk_bytes: 256,
    }
}

fn seed_invocation(
    store: &RedbStore,
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
    budget: WorkspaceBudget,
) -> TestResult {
    let command = CommandId::new("command-materialization")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        context.run().clone(),
        ActorRef::new("actor-materialization")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"fixture"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new("event-materialization")?,
        context.run().clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::NodeScheduled {
            node: context.node().clone(),
            execution: context.execution().clone(),
            attempt: context.attempt().clone(),
            invocation: request.invocation().clone(),
            idempotency_key: None,
            request: request.clone(),
        },
    )?;
    let result = CommandResultDocument::new(
        command,
        context.run().clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let commit = AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: budget.clone(),
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        }),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run: context.run().clone(),
                workflow: WorkflowId::new("workflow-materialization")?,
                revision: context.revision().clone(),
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?;
    let _ = store.commit_command(&commit)?;
    Ok(())
}

#[test]
fn isolated_inputs_and_outputs_use_the_durable_artifact_protocol() -> TestResult {
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let budget = WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?;
    let access = StoreInvocationDataAccess::new(
        store.clone(),
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
        budget.clone(),
    )?;
    let context = context()?;
    let request = request()?;
    seed_invocation(store.as_ref(), &context, &request, budget)?;
    let workspace = access.materialize(
        &context,
        &request,
        &[InputMaterialization::new("prompt", "inputs/prompt.json")?],
        limits(),
    )?;
    let input = workspace.input_path("prompt").ok_or("missing input path")?;
    assert!(input.starts_with(workspace.root()));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(input)?)?,
        json!({"instruction": "safe"})
    );
    fs::create_dir(workspace.root().join("out"))?;
    fs::write(workspace.root().join("out/result.txt"), b"published")?;
    let reference = access.publish_file(
        &context,
        &request,
        workspace.as_ref(),
        "result",
        Path::new("out/result.txt"),
        "text/plain",
        limits(),
    )?;
    let durable = milkdrift_workspace::ArtifactReference::new(
        milkdrift_workspace::ArtifactId::new(reference.identity())?,
        milkdrift_workspace::ContentDigest::from_hex(reference.digest())?,
        milkdrift_workspace::MediaType::new(reference.media_type().ok_or("missing media")?)?,
        reference.size_bytes().ok_or("missing size")?,
    );
    assert!(store.is_committed(&durable)?);
    Ok(())
}

#[test]
fn configured_process_input_can_materialize_the_exact_context_manifest() -> TestResult {
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let budget = WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?;
    let context = context()?;
    let base_request = request()?;
    seed_invocation(store.as_ref(), &context, &base_request, budget.clone())?;
    let authorized = ArtifactReadAuthority::Authorized {
        actor: ActorRef::new("actor-materialization")?,
        evidence: EvidenceId::new("evidence-materialization")?,
    };
    let access = StoreInvocationDataAccess::new(store, execution_owner.path(), authorized, budget)?;
    let manifest_bytes = br#"{"schema_version":2,"selection":"frozen"}"#;
    let manifest = access.publish_bytes(
        &context,
        &base_request,
        "context-manifest",
        "application/vnd.milkdrift.context-manifest.v2+json",
        manifest_bytes,
        limits(),
    )?;
    let request = request()?.with_context_manifest(manifest)?;
    let workspace = access.materialize(
        &context,
        &request,
        &[InputMaterialization::new(
            milkdrift_capability::CONTEXT_MANIFEST_INPUT_NAME,
            "context/manifest.json",
        )?],
        limits(),
    )?;
    let path = workspace
        .input_path(milkdrift_capability::CONTEXT_MANIFEST_INPUT_NAME)
        .ok_or("missing context manifest path")?;
    assert_eq!(fs::read(path)?, manifest_bytes);
    Ok(())
}

#[test]
fn traversal_symlink_special_file_and_budget_escapes_are_rejected() -> TestResult {
    assert!(InputMaterialization::new("prompt", "../escape").is_err());
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let access = StoreInvocationDataAccess::new(
        store,
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
        WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
    )?;
    let context = context()?;
    let request = request()?;
    let workspace = access.materialize(
        &context,
        &request,
        &[InputMaterialization::new("prompt", "prompt.json")?],
        limits(),
    )?;
    fs::create_dir(workspace.root().join("directory-output"))?;
    assert!(
        access
            .publish_file(
                &context,
                &request,
                workspace.as_ref(),
                "directory",
                Path::new("directory-output"),
                "application/octet-stream",
                limits(),
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("prompt.json", workspace.root().join("linked-output"))?;
        assert!(
            access
                .publish_file(
                    &context,
                    &request,
                    workspace.as_ref(),
                    "linked",
                    Path::new("linked-output"),
                    "application/octet-stream",
                    limits(),
                )
                .is_err()
        );

        fs::hard_link(
            workspace.root().join("prompt.json"),
            workspace.root().join("hard-linked-output"),
        )?;
        assert!(
            access
                .publish_file(
                    &context,
                    &request,
                    workspace.as_ref(),
                    "hard-linked",
                    Path::new("hard-linked-output"),
                    "application/octet-stream",
                    limits(),
                )
                .is_err()
        );

        let _socket =
            std::os::unix::net::UnixListener::bind(workspace.root().join("special-output.socket"))?;
        assert!(
            access
                .publish_file(
                    &context,
                    &request,
                    workspace.as_ref(),
                    "special",
                    Path::new("special-output.socket"),
                    "application/octet-stream",
                    limits(),
                )
                .is_err()
        );
    }
    let mut tiny = limits();
    tiny.max_file_bytes = 2;
    tiny.max_total_bytes = 2;
    assert!(
        access
            .materialize(
                &context,
                &request,
                &[InputMaterialization::new("prompt", "prompt.json")?],
                tiny,
            )
            .is_err()
    );

    let two_inputs = two_input_request()?;
    let specifications = [
        InputMaterialization::new("first", "first.json")?,
        InputMaterialization::new("second", "second.json")?,
    ];
    let mut one_file = limits();
    one_file.max_files = 1;
    assert!(
        access
            .materialize(&context, &two_inputs, &specifications, one_file)
            .is_err()
    );
    let mut shallow = limits();
    shallow.max_directory_depth = 2;
    assert!(
        access
            .materialize(
                &context,
                &request,
                &[InputMaterialization::new("prompt", "one/two/prompt.json")?],
                shallow,
            )
            .is_err()
    );
    let mut aggregate = limits();
    aggregate.max_file_bytes = 4;
    aggregate.max_total_bytes = 6;
    assert!(
        access
            .materialize(&context, &two_inputs, &specifications, aggregate)
            .is_err()
    );
    Ok(())
}

#[test]
fn failed_publication_is_aborted_without_a_committed_reference() -> TestResult {
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(store_owner.path()).with_fault_injector(Arc::new(FailOnce::new(
            FaultPoint::BeforeArtifactChunkWrite,
        ))),
    )?);
    let budget = WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?;
    let access = StoreInvocationDataAccess::new(
        store.clone(),
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
        budget.clone(),
    )?;
    let context = context()?;
    let request = request()?;
    seed_invocation(store.as_ref(), &context, &request, budget)?;
    let bytes = b"publication must be atomic";
    let expected =
        expected_process_artifact(&context, &request, "faulted-output", "text/plain", bytes)?;

    assert!(
        access
            .publish_bytes(
                &context,
                &request,
                "faulted-output",
                "text/plain",
                bytes,
                limits(),
            )
            .is_err()
    );
    assert!(!store.is_committed(&expected)?);

    let recovered = access.publish_bytes(
        &context,
        &request,
        "faulted-output",
        "text/plain",
        bytes,
        limits(),
    )?;
    assert_eq!(recovered.identity(), expected.artifact().as_str());
    assert!(store.is_committed(&expected)?);
    Ok(())
}
