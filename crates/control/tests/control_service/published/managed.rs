//! Deterministic resource-owner integration. Physical Linux qualification uses the binary lane.
use super::*;
use milkdrift_capability::managed::*;
use milkdrift_persistence::managed::*;

fn digest() -> String {
    format!("b3_{}", "1".repeat(64))
}
fn name(value: &str) -> TestResult<ManagedName> {
    Ok(ManagedName::new(value)?)
}

pub(super) fn install(
    store: &RedbStore,
    descriptor: milkdrift_capability::CapabilityDescriptor,
) -> TestResult<milkdrift_capability::CapabilityDescriptor> {
    let binding = ManagedBinding {
        installation: name("publication-test")?,
        generation: 1,
        recipe_digest: digest(),
        resources: vec![ResourceRequirement {
            resource: name("working")?,
            mutation: true,
        }],
    };
    let mut value = serde_json::to_value(&descriptor)?;
    value["extensions"][MANAGED_BINDING_EXTENSION] = serde_json::to_value(binding)?;
    let descriptor: milkdrift_capability::CapabilityDescriptor = serde_json::from_value(value)?;
    if store
        .managed_installation(&name("publication-test")?)?
        .is_some()
    {
        return Ok(descriptor);
    }
    let recipe = RecipeReference {
        name: name("publication-test")?,
        digest: digest(),
    };
    let request = ManagedRequest {
        schema_version: MANAGED_SCHEMA_VERSION,
        command: name("install-test")?,
        installation: name("publication-test")?,
        expected_version: 0,
        action: ManagedAction::Apply {
            recipe: recipe.clone(),
        },
    };
    let grant = publication_grant("service:deployment", "grant:deployment")?;
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?;
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(CapabilityId::new("managed.publication-test")?);
    resources.capability_operation = Some(OperationId::new(request.operation())?);
    let authorization = evaluator.evaluate(&AuthorityRequest {
        decision: DecisionId::new("decision:installation")?,
        actor: grant.actor().clone(),
        grant: grant.identity().clone(),
        grant_revision: 1,
        grant_digest: grant.digest()?,
        revocation_generation: 0,
        operation: AuthorityOperation::AdministerCapabilities,
        resources,
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(NOW),
        provenance: Default::default(),
    })?;
    let setup = ApprovedSetup {
        protection: None,
        recipe,
        mechanism: "deterministic-test".to_owned(),
        configuration: BoundedJson::new(serde_json::json!({}))?,
        platform_owner: "publication-test".to_owned(),
        ownership: digest(),
        resources: vec![ManagedResourceView {
            name: name("working")?,
            kind: ManagedResourceKind::WorkingArea,
            ownership: ResourceOwnership::Owned,
            identity: "test-working-area".to_owned(),
            disposition: DataDisposition::Preserve,
        }],
        capabilities: vec![descriptor.clone()],
    };
    store.begin_managed_change(
        &request,
        &authorization,
        &ManagedChange {
            entry_authorization: None,
            identity: digest(),
            candidate: setup,
            generation: 1,
            steps: vec![ManagedStep::Verify],
            removing: false,
            running: false,
        },
    )?;
    store.advance_managed_change(
        &request.installation,
        &digest(),
        0,
        ManagedObservation {
            digest: digest(),
            summary: "test mechanism verified".to_owned(),
            running: false,
        },
    )?;
    Ok(descriptor)
}

pub(super) struct ManagedProcessAdapter {
    pub(super) store: Arc<RedbStore>,
    pub(super) process: Arc<CountingProcessAdapter>,
    pub(super) uncertain_entry: Arc<std::sync::atomic::AtomicBool>,
}
impl CapabilityAdapter for ManagedProcessAdapter {
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.process.admission_envelope(invocation)
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let run = || -> TestResult {
            let id = managed_use_id(invocation.request().invocation());
            let usage = self
                .store
                .managed_use(&id)?
                .ok_or("child resource hold absent")?;
            let parent_id = usage
                .parent
                .as_ref()
                .ok_or("published writer did not receive a guarded parent claim")?;
            let parent = self
                .store
                .managed_use(parent_id)?
                .ok_or("parent lifetime hold absent")?;
            assert!(matches!(parent.phase, ManagedUsePhase::Suspended { .. }));
            assert!(parent.editing.is_empty());
            assert_eq!(usage.editing, vec![name("working")?]);
            assert!(
                self.store
                    .enter_managed_use(parent_id, parent.claim, "forbidden-parent-writer")
                    .is_err()
            );
            self.store
                .enter_managed_use(&id, usage.claim, &format!("fixture:{id}"))?;
            self.store.verify_managed_integrity()?;
            if self.uncertain_entry.swap(false, Ordering::SeqCst) {
                return Err("lost worker outcome after physical entry".into());
            }
            self.store.quiesce_managed_use(
                &id,
                usage.claim,
                &QuiescenceEvidence::PhysicalStop {
                    physical_identity: format!("fixture:{id}"),
                    observation_digest: digest(),
                    disrupted: false,
                },
            )?;
            Ok(())
        };
        run().map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.process.execute(invocation, reporter)
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.process.cancel(request)
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        self.process.health(now)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[test]
fn publication_hands_editing_to_exact_internal_writers_and_releases_every_hold() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture_with_resources(directory.path(), "managed", true)?;
    let run = start_outer(&fixture, "run:managed-publication")?;
    let mut parent_resumed = false;
    for _ in 0..64 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
        if !parent_resumed && fixture.process.entries() == 1 {
            let inventory = fixture
                .store
                .managed_installation(&name("publication-test")?)?
                .ok_or("inventory absent between children")?;
            assert_eq!(
                inventory.uses.len(),
                1,
                "the first child must release its hold"
            );
            let parent = &inventory.uses[0];
            assert!(matches!(
                parent.phase,
                ManagedUsePhase::Quiescent {
                    evidence: QuiescenceEvidence::NoExternalEntry { .. }
                }
            ));
            assert_eq!(parent.editing, vec![name("working")?]);
            assert!(
                parent.claim > 1,
                "return must advance the exact parent claim"
            );
            parent_resumed = true;
        }
        if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    assert_eq!(
        fixture.runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    assert_eq!(fixture.process.entries(), 2);
    assert!(
        parent_resumed,
        "editing must return before the next child can acquire it"
    );
    let inventory = fixture
        .store
        .managed_installation(&name("publication-test")?)?
        .ok_or("inventory absent")?;
    assert!(
        inventory.uses.is_empty(),
        "all parent and child holds must settle: {:?}",
        inventory.uses
    );
    fixture.store.verify_managed_integrity()?;
    Ok(())
}

#[test]
fn lost_child_stop_proof_survives_cancel_and_reopen_without_returning_editing() -> TestResult {
    let directory = TempDir::new()?;
    let run;
    let source;
    let held;
    {
        let fixture = fixture_with_resources(directory.path(), "lost-stop", true)?;
        fixture.uncertain_entry.store(true, Ordering::SeqCst);
        run = start_outer(&fixture, "run:lost-child-stop")?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop()
            .ok_or("link absent")?;
        source = plan.source.clone();
        for _ in 0..32 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
        }
        let inventory = fixture
            .store
            .managed_installation(&name("publication-test")?)?
            .ok_or("inventory absent")?;
        assert_eq!(inventory.uses.len(), 2);
        let parent = inventory
            .uses
            .iter()
            .find(|usage| usage.parent.is_none())
            .ok_or("parent absent")?;
        assert!(matches!(parent.phase, ManagedUsePhase::Suspended { .. }));
        assert!(parent.editing.is_empty());
        let child = inventory
            .uses
            .iter()
            .find(|usage| usage.parent.is_some())
            .ok_or("child absent")?;
        assert!(matches!(child.phase, ManagedUsePhase::Entered { .. }));
        assert_eq!(child.editing, vec![name("working")?]);
        fixture
            .runtime
            .cancel_published_run(&plan, fixture.store.as_ref())?;
        for _ in 0..8 {
            runtime_tick(&fixture.runtime)?;
        }
        held = fixture
            .store
            .managed_installation(&name("publication-test")?)?
            .ok_or("inventory absent")?
            .uses;
        assert!(!fixture.runtime.projection(&run)?.lifecycle().is_completed());
        assert_eq!(fixture.process.entries(), 0);
    }
    let reopened = fixture_with_resources(directory.path(), "lost-stop-reopen", true)?;
    for _ in 0..8 {
        runtime_tick(&reopened.runtime)?;
    }
    let inventory = reopened
        .store
        .managed_installation(&name("publication-test")?)?
        .ok_or("inventory absent")?;
    assert_eq!(
        inventory.uses, held,
        "restart or cancellation acknowledgement cannot establish physical stop"
    );
    assert!(reopened.store.published_local_pending(&source)?);
    assert!(
        !reopened
            .runtime
            .projection(&run)?
            .lifecycle()
            .is_completed()
    );
    let parent = inventory
        .uses
        .iter()
        .find(|usage| usage.parent.is_none())
        .ok_or("parent absent")?;
    assert!(
        reopened
            .store
            .enter_managed_use(&parent.id, parent.claim, "unsafe-parent-resumption")
            .is_err()
    );
    assert_eq!(reopened.process.entries(), 0);
    reopened.store.verify_managed_integrity()?;
    Ok(())
}
