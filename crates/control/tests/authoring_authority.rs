//! Saved review applies the same descriptor scope as the capability catalog before exposing ports.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    CapabilityAuthorityScope, CapabilityAuthorityScopeBuilder, GrantId, GrantSetEvaluator,
    PolicyId, WorkflowRunScope,
};
use milkdrift_blueprint::authoring::{
    AuthoringCapability, AuthoringDefinition, AuthoringDocument, AuthoringTask,
};
use milkdrift_blueprint::{TaskExecution, WorkflowCreate};
use milkdrift_capability::{
    CapabilityCategory, CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityId,
    CapabilityRequirement, Locality, OperationId, ProviderProfileRef, SideEffectClass,
    TaskAuthoringContract, TaskAuthoringPort, TaskInputMedia, TrustZone,
};
use milkdrift_control::{ActorAuthorityContext, AuthoringScope, TaskControlService};
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ExecutionDispatch, ExecutorError,
    PreparedExecution, ResolvedCapability, SequentialIdGenerator, SystemBoundaryClock,
    TaskExecutor, TaskRuntime,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Executor {
    descriptor: CapabilityDescriptor,
    inner: DeterministicExecutor,
}
impl TaskExecutor for Executor {
    fn task_authoring(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<Option<(CapabilityDescriptor, TaskAuthoringContract)>, ExecutorError> {
        if requirement.exact_capability() != Some(self.descriptor.identity()) {
            return Ok(None);
        }
        Ok(Some((
            self.descriptor.clone(),
            TaskAuthoringContract {
                label: "Unexposed private adapter label".into(),
                input_media: TaskInputMedia::Any,
                inputs: vec![TaskAuthoringPort {
                    name: "private_input".into(),
                    label: "Private operations evidence".into(),
                    required: true,
                    accepts_artifacts: true,
                    media_type: Some("text/plain".into()),
                    max_bytes: 1024,
                }],
                allows_additional_inputs: false,
                outputs: vec![],
                default_output: None,
            },
        )))
    }
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.inner.resolve(requirement, now)
    }
    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        self.inner.prepare_exact_entry(dispatch)
    }
    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, ExecutorError> {
        self.inner.cancel(request)
    }
}

fn grant(
    name: &str,
    scope: CapabilityAuthorityScope,
    missing: Option<AuthorityOperation>,
) -> TestResult<AuthorityGrant> {
    let actor = ActorRef::new(format!("human:{name}"))?;
    let template =
        AuthorityGrantBuilder::new(GrantId::new(format!("grant:{name}"))?, 1, actor.clone())
            .operations(BTreeSet::from([AuthorityOperation::InspectRun]))
            .build()?;
    let mut resources = template.resources().clone();
    resources.workflow_run = WorkflowRunScope::Any;
    resources.capability = scope;
    let mut operations = BTreeSet::from([
        AuthorityOperation::CreateRun,
        AuthorityOperation::StartRun,
        AuthorityOperation::Apply,
        AuthorityOperation::InspectRun,
        AuthorityOperation::ListCapabilities,
        AuthorityOperation::InspectCapabilityHealth,
        AuthorityOperation::InspectProviderProfile,
    ]);
    if let Some(missing) = missing {
        operations.remove(&missing);
    }
    Ok(
        AuthorityGrantBuilder::new(template.identity().clone(), 1, actor)
            .operations(operations)
            .resources(resources)
            .budget(AuthorityBudget::default())
            .build()?,
    )
}

fn context(grant: &AuthorityGrant) -> TestResult<ActorAuthorityContext> {
    Ok(ActorAuthorityContext::new(
        grant.actor().clone(),
        CommandAuthorityClaim::new(
            grant.identity().clone(),
            grant.revision(),
            grant.digest()?,
            0,
        )?,
    ))
}

fn document(capability: &str) -> AuthoringDocument {
    AuthoringDocument {
        tasks: vec![AuthoringTask {
            id: "candidate".into(),
            version: None,
            title_edited: true,
            contract_generation: Some(3),
            definition: AuthoringDefinition {
                title: "Review supplied evidence".into(),
                description: String::new(),
                instructions: "Read the supplied evidence".into(),
                capability: AuthoringCapability {
                    id: capability.into(),
                    operation: "model.generate".into(),
                },
                inputs: BTreeMap::new(),
                outputs: BTreeMap::new(),
                dependencies: Default::default(),
                required: true,
                corrective_of: None,
                paths: vec![],
                tools: vec![],
                actions: None,
                execution: TaskExecution::Legacy,
            },
        }],
        ..AuthoringDocument::default()
    }
}

#[test]
fn review_hides_denied_ports_and_allows_scoped_metadata_without_invocation_permission() -> TestResult
{
    let any = || CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly);
    let builder = || CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly);
    let mut grants = vec![grant("allowed", any(), None)?];
    for (name, scope) in [
        (
            "identity",
            builder()
                .only_capabilities(BTreeSet::from([CapabilityId::new("other")?]))?
                .build(),
        ),
        (
            "category",
            builder()
                .only_categories(BTreeSet::from([CapabilityCategory::Process]))?
                .build(),
        ),
        (
            "operation",
            builder()
                .only_operations(BTreeSet::from([OperationId::new("process.execute")?]))?
                .build(),
        ),
        (
            "profile",
            builder()
                .only_provider_profiles(BTreeSet::from([ProviderProfileRef::new("other")?]))?
                .build(),
        ),
        (
            "trust",
            builder()
                .only_trust_zones(BTreeSet::from([TrustZone::new("other")?]))?
                .build(),
        ),
        (
            "locality",
            builder()
                .only_localities(BTreeSet::from([Locality::Local]))?
                .build(),
        ),
        (
            "effect",
            CapabilityAuthorityScope::allow_any(SideEffectClass::None),
        ),
    ] {
        grants.push(grant(name, scope, None)?);
    }
    for (name, missing) in [
        ("no-list", AuthorityOperation::ListCapabilities),
        ("no-health", AuthorityOperation::InspectCapabilityHealth),
        ("no-profile", AuthorityOperation::InspectProviderProfile),
    ] {
        grants.push(grant(name, any(), Some(missing))?);
    }
    assert!(grants.iter().all(|grant| {
        !grant
            .operations()
            .contains(&AuthorityOperation::InvokeCapability)
    }));
    let allowed = context(grants.first().ok_or("allowed grant absent")?)?;
    let denied = grants
        .iter()
        .skip(1)
        .map(context)
        .collect::<TestResult<Vec<_>>>()?;
    let evaluator = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("authoring-read-scope")?,
        1,
        grants,
        BTreeMap::new(),
    )?);
    let root = tempfile::tempdir()?;
    let store = Arc::new(milkdrift_redb_store::RedbStore::open(root.path())?);
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    *descriptor
        .pointer_mut("/descriptor/operations/model.generate/side_effect")
        .ok_or("operation side effect absent")? = serde_json::json!("read_only");
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?
        .body()
        .clone();
    let executor = Arc::new(Executor {
        inner: DeterministicExecutor::new(descriptor.clone()),
        descriptor,
    });
    let clock = Arc::new(SystemBoundaryClock);
    let runtime = Arc::new(TaskRuntime::new(
        store.clone(),
        executor,
        evaluator.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("authoring-authority", 1)?),
    ));
    let control = TaskControlService::new(runtime, evaluator, clock, store.clone())
        .with_authoring_store(store);
    let create: WorkflowCreate =
        serde_json::from_value(serde_json::json!({"title":"Unfinished scoped work","tasks":[]}))?;
    let workflow = control.create(&allowed, "empty", create)?.workflow;
    let saved =
        control.checkpoint_authoring(&allowed, &workflow, 0, document("anthropic-primary"))?;
    let scope = AuthoringScope {
        tasks: vec!["candidate".into()],
        changes: vec![],
        use_current: vec![],
    };
    let allowed_review = control.review_authoring(&allowed, &workflow, saved.generation, &scope)?;
    assert!(
        allowed_review
            .issues
            .iter()
            .any(|issue| issue.message.contains("Private operations evidence")),
        "catalog readers can inspect their allowed inputs without invocation permission"
    );
    let mut denied_reviews = Vec::new();
    for actor in &denied {
        let review = control.review_authoring(actor, &workflow, saved.generation, &scope)?;
        assert!(!serde_json::to_string(&review)?.contains("Private"));
        assert!(review.issues.iter().any(|issue| issue.message
            == "This model or agent is unavailable. Choose a current connection"));
        denied_reviews.push(review.issues);
    }
    let missing = control.checkpoint_authoring(
        &allowed,
        &workflow,
        saved.generation,
        document("unregistered"),
    )?;
    let absent = control.review_authoring(&allowed, &workflow, missing.generation, &scope)?;
    assert!(
        denied_reviews.iter().all(|issues| issues == &absent.issues),
        "denied and unavailable selections expose the same diagnostics"
    );
    Ok(())
}
