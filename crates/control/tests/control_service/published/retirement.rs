//! Retired definitions outlive their registry adapters and still replay after complete reopen.
use super::*;

fn retire(
    fixture: &Fixture,
    generation: u64,
) -> TestResult<milkdrift_persistence::published::PublishedMethodRecord> {
    let evaluator = GrantSetEvaluator::new(
        PolicyId::new("test.publication")?,
        1,
        [publication_grant("human:caller", "grant:caller")?],
        BTreeMap::new(),
    )?;
    let mut request = fixture.decision.request().clone();
    request.resources.capability_operation = Some(OperationId::new("method.retire")?);
    Ok(fixture.published.retire(
        fixture.method.descriptor.identity(),
        generation,
        1,
        &evaluator.evaluate(&request)?,
        &milkdrift_persistence::IntegrityDigest::hash(format!("retire-{generation}").as_bytes()),
    )?)
}

fn generations(fixture: &Fixture) -> TestResult<Vec<milkdrift_capability_host::GenerationView>> {
    Ok(fixture
        .host
        .generations(
            &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
            NOW,
        )?
        .into_iter()
        .filter(|g| g.capability == *fixture.method.descriptor.identity())
        .collect())
}

#[test]
fn retired_generations_turn_over_beyond_registry_capacity_and_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let mut accepted = Vec::new();
    for number in 1..=6 {
        let fixture = fixture(directory.path(), &format!("turnover-{number}"))?;
        let mut method = fixture.method.clone();
        let mut descriptor = serde_json::to_value(&method.descriptor)?;
        descriptor["descriptor_revision"] = serde_json::json!(number);
        method.descriptor = serde_json::from_value(descriptor)?;
        let request = milkdrift_persistence::IntegrityDigest::hash(&serde_json::to_vec(&method)?);
        let expected = (number > 1).then_some(2);
        let published =
            fixture
                .published
                .publish(method.clone(), expected, &fixture.decision, &request)?;
        let run = start_outer(&fixture, &format!("run:turnover-{number}"))?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop()
            .ok_or("pending plan absent")?;
        assert_eq!(plan.generation, number);
        let retired = retire(&fixture, number)?;
        fixture.published.maintain_retirement()?;
        assert_eq!(generations(&fixture)?.len(), 1);
        assert_eq!(generations(&fixture)?[0].pending_workflows, 1);
        drop(fixture);

        let fixture = super::fixture(directory.path(), &format!("settle-{number}"))?;
        assert_eq!(generations(&fixture)?.len(), 1);
        assert!(generations(&fixture)?[0].draining);
        for _ in 0..64 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            fixture.published.maintain_retirement()?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert_eq!(
            fixture.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        assert!(generations(&fixture)?.is_empty());
        assert_eq!(
            fixture.store.published_invocation(&plan.source)?.as_ref(),
            Some(&plan)
        );
        assert_eq!(retire(&fixture, number)?, retired);
        accepted.push((method, expected, request, published, retired, plan));
    }
    let fixture = fixture(directory.path(), "turnover-history")?;
    assert!(generations(&fixture)?.is_empty());
    for (method, expected, request, published, retired, plan) in accepted {
        assert_eq!(
            fixture
                .published
                .publish(method, expected, &fixture.decision, &request)?,
            published
        );
        assert_eq!(
            fixture
                .store
                .published_method(&plan.capability, plan.generation)?,
            Some(retired)
        );
        assert_eq!(
            fixture.store.published_invocation(&plan.source)?,
            Some(plan)
        );
    }
    assert!(generations(&fixture)?.is_empty());
    assert_eq!(fixture.process.entries(), 0);
    Ok(())
}
