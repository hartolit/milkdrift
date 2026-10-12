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
    ArtifactReadAuthority, ArtifactStore, PersistenceError, TimestampMillis, WorkflowCommit,
    WorkflowSnapshot, WorkflowStore,
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

fn context(
    request: &InvocationRequest,
    budget: WorkspaceBudget,
) -> TestResult<AdapterExecutionContext> {
    use milkdrift_authority::{
        AuthorityBudget, AuthorityDecisionSnapshot, AuthorityExecutionProvenance,
        AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId, DecisionReasonCode,
        ExecutionAuthorityBasis, GrantDigest, GrantId, PolicyId, RequestedResourceFacts,
    };
    use milkdrift_capability::{
        CapabilityDescriptorDocument, ResolvedCapabilitySnapshot, SideEffectClass,
    };
    let revision: RevisionId = serde_json::from_value(json!(format!("rev_{}", "1".repeat(64))))?;
    let workflow = WorkflowId::new("run-materialization")?;
    let run = RunId::new(workflow.as_str())?;
    let decision = AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("materialization-test")?,
        1,
        AuthorityRequest {
            decision: DecisionId::new("materialization-start")?,
            actor: ActorRef::new("actor-materialization")?,
            grant: GrantId::new("materialization-grant")?,
            grant_revision: 1,
            grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
            revocation_generation: 0,
            operation: AuthorityOperation::StartRun,
            resources: RequestedResourceFacts::empty(),
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(10),
            provenance: AuthorityExecutionProvenance::default(),
        },
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::Unknown,
    )?;
    let basis = ExecutionAuthorityBasis::from_start_decision(
        &decision,
        workflow,
        run.clone(),
        revision.clone(),
    )?;
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let body = descriptor
        .get_mut("descriptor")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor fixture body missing")?;
    for (field, value) in [
        ("identity", json!(request.capability())),
        ("provider_profile", json!(request.provider_profile())),
        ("category", json!({"type":"process"})),
    ] {
        *body
            .get_mut(field)
            .ok_or("descriptor fixture field missing")? = value;
    }
    let operations = body
        .get_mut("operations")
        .ok_or("descriptor fixture operations missing")?;
    let operation = operations
        .get_mut("model.generate")
        .ok_or("descriptor fixture model operation missing")?
        .take();
    *operations = json!({request.operation().as_str(): operation});
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?;
    let resolution =
        ResolvedCapabilitySnapshot::from_descriptor(descriptor.body(), request.operation())?;
    // This trusted fixture restores the same immutable dispatch representation as durable recovery.
    let dispatch: milkdrift_runtime::ExecutionDispatch = serde_json::from_value(json!({
        "run":run,"revision":revision,"node":NodeId::new("node-materialization")?,
        "execution":"execution-materialization","attempt":"attempt-materialization",
        "lease":"lease-materialization","lease_expires_at":1000,
        "resolution":resolution,"execution_authority":basis,
        "resolution_authorization":decision,"entry_authorization":decision,"request":request
    }))?;
    Ok(AdapterExecutionContext::from_dispatch(
        &dispatch.for_task(budget, Vec::new()),
    )?)
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

#[test]
fn direct_selection_materializes_only_frozen_inputs_and_cannot_publish_before_entry() -> TestResult
{
    use milkdrift_capability_host::DirectInputSelection;
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let access = StoreInvocationDataAccess::new(
        store,
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let request = request()?;
    let selection = DirectInputSelection::new(&request, 1_024)?;
    assert!(DirectInputSelection::new(&request, 1).is_err());
    assert!(!selection.canonical_json().contains("instruction"));
    let context = AdapterExecutionContext::direct(selection);
    assert!(context.workflow().is_none());
    let workspace = access.materialize(
        &context,
        &request,
        &[InputMaterialization::new("prompt", "prompt.json")?],
        limits(),
    )?;
    assert_eq!(
        fs::read(workspace.input_path("prompt").ok_or("input missing")?)?,
        br#"{"instruction":"safe"}"#
    );
    let foreign = InputReference::new(
        "other",
        InvocationValueReference::Inline {
            value: BoundedJson::new(json!("secret"))?,
        },
    )?;
    assert!(
        access
            .read_input_bytes(&context, &foreign, limits())
            .is_err()
    );
    assert!(
        access
            .materialize(&context, &two_input_request()?, &[], limits())
            .is_err()
    );
    let missing = milkdrift_capability::ArtifactReference::new(
        "forbidden",
        "1".repeat(64),
        Some("text/plain".to_owned()),
        Some(4),
    )?;
    let error = access
        .read_artifact_bytes(&context, &missing, limits())
        .err()
        .ok_or("foreign read accepted")?;
    assert!(
        error
            .to_string()
            .contains("outside the frozen direct selection")
    );
    assert!(
        access
            .publish_bytes(
                &context,
                &request,
                "output",
                "text/plain",
                b"result",
                limits()
            )
            .is_err()
    );
    drop(workspace);
    assert_eq!(fs::read_dir(execution_owner.path())?.count(), 0);
    Ok(())
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
        context
            .workflow()
            .ok_or("workflow context missing")?
            .run()
            .as_str()
            .as_bytes(),
        context
            .workflow()
            .ok_or("workflow context missing")?
            .revision()
            .as_str()
            .as_bytes(),
        context
            .workflow()
            .ok_or("workflow context missing")?
            .node()
            .as_str()
            .as_bytes(),
        context
            .workflow()
            .ok_or("workflow context missing")?
            .execution()
            .as_str()
            .as_bytes(),
        context
            .workflow()
            .ok_or("workflow context missing")?
            .attempt()
            .as_str()
            .as_bytes(),
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
) -> TestResult {
    let workflow = WorkflowId::new(
        context
            .workflow()
            .ok_or("workflow context missing")?
            .run()
            .as_str(),
    )?;
    store.commit_workflow(WorkflowCommit {
        expected_generation: None,
        snapshot: WorkflowSnapshot {
            workflow,
            generation: 1,
            title: "materialization fixture".into(),
            description: String::new(),
            owner_actor: "actor-materialization".into(),
            updated_at: TimestampMillis::new(10),
            document: json!({"fixture":"accepted task entry"}),
            entered_invocations: vec![request.invocation().clone()],
            managed_executions: Vec::new(),
            artifact_capacity: 4096,
            reserved_bytes: 0,
            reserved_receipts: 0,
        },
        command: None,
    })?;
    Ok(())
}

#[test]
fn isolated_inputs_and_outputs_use_the_durable_artifact_protocol() -> TestResult {
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let budget = WorkspaceBudget::new(16, 1024, 4096, 16, 32, 4096)?;
    let access = StoreInvocationDataAccess::new(
        store.clone(),
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let request = request()?;
    let context = context(&request, budget)?;
    seed_invocation(store.as_ref(), &context, &request)?;
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
    // The adapter's materialization limit permits this, but the run's accepted
    // per-artifact ceiling is smaller. Publication must use the durable run budget.
    assert!(
        access
            .publish_bytes(
                &context,
                &request,
                "over-run-budget",
                "text/plain",
                &[0; 64],
                limits(),
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn output_publication_preserves_aliased_inputs_as_one_exact_causal_parent() -> TestResult {
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(store_owner.path())?);
    let source = milkdrift_capability::ArtifactReference::new(
        "evidence:accepted-decision",
        ContentDigest::for_bytes(b"accepted").to_hex(),
        Some("text/plain".to_owned()),
        Some(8),
    )?;
    let durable = ArtifactReference::new(
        ArtifactId::new(source.identity())?,
        ContentDigest::for_bytes(b"accepted"),
        MediaType::new("text/plain")?,
        8,
    );
    let metadata = milkdrift_workspace::ArtifactMetadata::new(
        durable,
        milkdrift_workspace::ArtifactSensitivity::Public,
        milkdrift_workspace::ArtifactRetention::WhileReferenced,
        milkdrift_workspace::ArtifactProvenance::new(
            milkdrift_workspace::CausalReference::External {
                source: milkdrift_workspace::CausalId::new("acceptance-fixture")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = milkdrift_persistence::BeginArtifactPublication::new(
        milkdrift_persistence::ArtifactPublicationId::new("publication:acceptance-fixture")?,
        RunId::new("run-input-fixture")?,
        metadata,
        WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, b"accepted")?;
    store.commit_publication(publication.publication())?;
    let base = request()?;
    let request = InvocationRequest::new(
        base.invocation().clone(),
        base.capability().clone(),
        base.operation().clone(),
        None,
        None,
        ["acceptance_result", "accepted_result"]
            .into_iter()
            .map(|name| {
                InputReference::new(
                    name,
                    InvocationValueReference::Artifact {
                        reference: source.clone(),
                    },
                )
            })
            .collect::<Result<_, _>>()?,
        BTreeMap::new(),
    )?;
    let context = context(
        &request,
        WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
    )?;
    seed_invocation(store.as_ref(), &context, &request)?;
    let access = StoreInvocationDataAccess::new(
        store.clone(),
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let workspace = access.materialize(
        &context,
        &request,
        &[
            InputMaterialization::new("acceptance_result", "first.txt")?,
            InputMaterialization::new("accepted_result", "second.txt")?,
        ],
        limits(),
    )?;
    assert_eq!(
        fs::read(
            workspace
                .input_path("acceptance_result")
                .ok_or("first input missing")?
        )?,
        b"accepted"
    );
    assert_eq!(
        fs::read(
            workspace
                .input_path("accepted_result")
                .ok_or("second input missing")?
        )?,
        b"accepted"
    );
    let first = access.publish_bytes(
        &context,
        &request,
        "next-result",
        "text/plain",
        b"derived",
        limits(),
    )?;
    let metadata = store
        .metadata(&ArtifactId::new(first.identity())?)?
        .ok_or("output metadata absent")?;
    assert_eq!(request.inputs().len(), 2, "named bindings remain distinct");
    assert_eq!(metadata.provenance().causes().iter().filter(|cause| matches!(cause,
        milkdrift_workspace::CausalReference::Artifact { reference } if reference.artifact().as_str() == source.identity()
    )).count(), 1);
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
    )?;
    let request = request()?;
    let context = context(
        &request,
        WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
    )?;
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

        // Bind outside the longer materialization path to respect macOS's Unix
        // socket address limit, then move the same special file into the workspace.
        let socket_owner = tempfile::tempdir()?;
        let socket_path = socket_owner.path().join("s");
        let _socket = std::os::unix::net::UnixListener::bind(&socket_path)?;
        fs::rename(socket_path, workspace.root().join("special-output.socket"))?;
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
    let two_context = context_for_request(&two_inputs)?;
    let specifications = [
        InputMaterialization::new("first", "first.json")?,
        InputMaterialization::new("second", "second.json")?,
    ];
    let mut one_file = limits();
    one_file.max_files = 1;
    assert!(
        access
            .materialize(&two_context, &two_inputs, &specifications, one_file)
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
    aggregate.max_file_bytes = 3;
    aggregate.max_total_bytes = 3;
    assert!(
        access
            .materialize(&two_context, &two_inputs, &specifications, aggregate)
            .is_err()
    );
    Ok(())
}

#[test]
fn publication_failure_reports_abort_failure_and_remains_recoverable() -> TestResult {
    struct FailWriteAndAbort {
        write: FailOnce,
        abort: FailOnce,
    }
    impl FaultInjector for FailWriteAndAbort {
        fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
            self.write.check(point)?;
            self.abort.check(point)
        }
    }
    let store_owner = tempfile::tempdir()?;
    let execution_owner = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open_with_config(
        RedbStoreConfig::new(store_owner.path()).with_fault_injector(Arc::new(FailWriteAndAbort {
            write: FailOnce::new(FaultPoint::BeforeArtifactChunkWrite),
            abort: FailOnce::new(FaultPoint::BeforeArtifactAbortCommit),
        })),
    )?);
    let access = StoreInvocationDataAccess::new(
        store.clone(),
        execution_owner.path(),
        ArtifactReadAuthority::PublicOnly,
    )?;
    let request = request()?;
    let context = context(
        &request,
        WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
    )?;
    seed_invocation(store.as_ref(), &context, &request)?;
    let bytes = b"retained failed output";
    let expected = expected_process_artifact(
        &context,
        &request,
        "cleanup-faulted-output",
        "text/plain",
        bytes,
    )?;
    let result = access.publish_bytes(
        &context,
        &request,
        "cleanup-faulted-output",
        "text/plain",
        bytes,
        limits(),
    );
    let error = result.err().ok_or("publication unexpectedly succeeded")?;
    assert!(
        error
            .to_string()
            .contains("publication cleanup also failed and the session may remain")
    );
    assert!(!store.is_committed(&expected)?);
    let recovered = access.publish_bytes(
        &context,
        &request,
        "cleanup-faulted-output",
        "text/plain",
        bytes,
        limits(),
    )?;
    assert_eq!(recovered.identity(), expected.artifact().as_str());
    assert!(store.is_committed(&expected)?);
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
    )?;
    let request = request()?;
    let context = context(&request, budget)?;
    seed_invocation(store.as_ref(), &context, &request)?;
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

fn context_for_request(request: &InvocationRequest) -> TestResult<AdapterExecutionContext> {
    context(
        request,
        WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
    )
}
