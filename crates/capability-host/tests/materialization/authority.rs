//! Capability execution permission does not authorize reading every selected artifact.

use super::*;
use milkdrift_authority::{
    ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator, AuthorityExecutionProvenance,
    AuthorityGrantBuilder, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
    CapabilityAuthorityScope, DecisionId, GrantId, GrantSetEvaluator, PolicyId,
    RequestedResourceFacts, Selection,
};
use milkdrift_capability::SideEffectClass;
use milkdrift_persistence::{ArtifactPublicationId, BeginArtifactPublication, EvidenceId};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity, CausalId,
    CausalReference,
};
use std::collections::BTreeSet;

fn source(
    store: &dyn ArtifactStore,
    bytes: &[u8],
) -> TestResult<milkdrift_capability::ArtifactReference> {
    let reference = ArtifactReference::new(
        ArtifactId::new("read-scope-source")?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        u64::try_from(bytes.len())?,
    );
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("read-scope-fixture")?,
            },
            vec![],
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new("read-scope-publication")?,
        RunId::new("read-scope-producer")?,
        metadata,
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&publication)?;
    if !bytes.is_empty() {
        store.write_chunk(publication.publication(), 0, bytes)?;
    }
    store.commit_publication(publication.publication())?;
    Ok(milkdrift_capability::ArtifactReference::new(
        reference.artifact().as_str(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().into()),
        Some(reference.size_bytes()),
    )?)
}

#[test]
fn task_materialization_requires_read_permission_beyond_valid_execution_authority() -> TestResult {
    for (case, can_read, scope, byte_limit, allowed) in [
        (
            "missing-operation",
            false,
            ArtifactAuthorityScope::dangerous_all(),
            1024,
            false,
        ),
        (
            "empty-missing-operation",
            false,
            ArtifactAuthorityScope::dangerous_all(),
            1024,
            false,
        ),
        (
            "wrong-identity",
            true,
            ArtifactAuthorityScope::new(
                Selection::only(BTreeSet::from([ArtifactId::new("other-source")?]))?,
                BTreeSet::from([ArtifactSensitivity::Restricted]),
            )?,
            1024,
            false,
        ),
        (
            "wrong-sensitivity",
            true,
            ArtifactAuthorityScope::new(
                Selection::any(),
                BTreeSet::from([ArtifactSensitivity::Public]),
            )?,
            1024,
            false,
        ),
        (
            "byte-budget",
            true,
            ArtifactAuthorityScope::dangerous_all(),
            2,
            false,
        ),
        (
            "allowed",
            true,
            ArtifactAuthorityScope::dangerous_all(),
            1024,
            true,
        ),
        (
            "expired",
            true,
            ArtifactAuthorityScope::dangerous_all(),
            1024,
            false,
        ),
    ] {
        let owner = tempfile::tempdir()?;
        let execution = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(owner.path())?);
        let bytes: &[u8] = if case == "empty-missing-operation" {
            b""
        } else {
            b"private result"
        };
        let reference = source(store.as_ref(), bytes)?;
        let base = request()?;
        let input = InputReference::new(
            "result",
            InvocationValueReference::Artifact {
                reference: reference.clone(),
            },
        )?;
        let request = InvocationRequest::new(
            base.invocation().clone(),
            base.capability().clone(),
            base.operation().clone(),
            None,
            None,
            vec![input.clone()],
            BTreeMap::new(),
        )?;
        let mut operations = BTreeSet::from([
            AuthorityOperation::StartRun,
            AuthorityOperation::InvokeCapability,
            AuthorityOperation::ReadArtifactMetadata,
        ]);
        if can_read {
            operations.insert(AuthorityOperation::ReadArtifactContent);
        }
        let bare = AuthorityGrantBuilder::new(
            GrantId::new("read-scope-grant")?,
            1,
            ActorRef::new("read-scope-actor")?,
        )
        .operations(operations)
        .build()?;
        let mut resources = bare.resources().clone();
        resources.capability = CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown);
        resources.artifacts = scope;
        let grant = AuthorityGrantBuilder::new(bare.identity().clone(), 1, bare.actor().clone())
            .operations(bare.operations().clone())
            .resources(resources)
            .budget(AuthorityBudget {
                artifact_bytes: Some(byte_limit),
                ..AuthorityBudget::default()
            })
            .validity(BoundaryTimeMillis::new(0), BoundaryTimeMillis::new(20))
            .build()?;
        let authority = Arc::new(GrantSetEvaluator::new(
            PolicyId::new("read-scope-policy")?,
            1,
            [grant.clone()],
            BTreeMap::new(),
        )?);
        let mut claim = AuthorityRequest {
            decision: DecisionId::new("read-scope-start")?,
            actor: grant.actor().clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant.digest()?,
            revocation_generation: grant.revocation_generation(),
            operation: AuthorityOperation::StartRun,
            resources: RequestedResourceFacts::empty(),
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(10),
            provenance: AuthorityExecutionProvenance::default(),
        };
        let start = authority.evaluate(&claim)?;
        assert!(start.is_allowed(), "{case}");
        claim.operation = AuthorityOperation::InvokeCapability;
        let entry = authority.evaluate(&claim)?;
        assert!(entry.is_allowed(), "{case} lacks execution permission");
        let context = context_with_authorization(
            &request,
            WorkspaceBudget::new(16, 1024, 4096, 16, 1024, 4096)?,
            &start,
            &entry,
        )?;
        let access = StoreInvocationDataAccess::new(
            store,
            execution.path(),
            ArtifactReadAuthority::Authorized {
                actor: ActorRef::new("service:daemon-runtime")?,
                evidence: EvidenceId::new("daemon-materialization")?,
            },
        )?
        .with_task_authority(
            authority.clone(),
            Arc::new(milkdrift_runtime::ManualClock::new(if case == "expired" {
                30
            } else {
                10
            })),
        );
        let read = access.read_input_bytes(&context, &input, limits());
        assert_eq!(read.is_ok(), allowed, "{case}");
        if allowed {
            assert_eq!(read?, bytes);
        }
        assert_eq!(
            access
                .read_artifact_bytes(&context, &reference, limits())
                .is_ok(),
            allowed,
            "{case}"
        );
        let direct = AdapterExecutionContext::direct(
            milkdrift_capability_host::DirectInputSelection::new(&request, 1024)?,
        );
        assert_eq!(
            access.read_input_bytes(&direct, &input, limits())?,
            bytes,
            "separately authorized direct reads changed for {case}"
        );
        assert_eq!(
            access
                .materialize(
                    &context,
                    &request,
                    &[InputMaterialization::new("result", "result.txt")?],
                    limits()
                )
                .is_ok(),
            allowed,
            "{case}"
        );
    }
    Ok(())
}
