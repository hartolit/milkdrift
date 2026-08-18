//! Integration evidence for durable workspace contracts and hostile input.

use milkdrift_capability::{BoundedJson, InvocationId};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, BranchId, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    ScopeId, ScopeLineage, ScopeReference, ValueKey, ValueOrigin, ValueVersion, WorkspaceBudget,
    WorkspaceError, WorkspaceScope, WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use proptest::prelude::*;
use serde_json::{Value, json};

fn bounded(value: Value) -> Result<BoundedJson, Box<dyn std::error::Error>> {
    Ok(BoundedJson::new(value)?)
}

fn artifact(content: &[u8]) -> Result<ArtifactMetadata, Box<dyn std::error::Error>> {
    let size = u64::try_from(content.len())?;
    let reference = ArtifactReference::new(
        ArtifactId::new("artifact/report")?,
        ContentDigest::for_bytes(content),
        MediaType::new("Application/JSON")?,
        size,
    );
    let provenance = ArtifactProvenance::new(
        CausalReference::Invocation {
            invocation: InvocationId::new("invocation-7")?,
        },
        vec![CausalReference::External {
            source: CausalId::new("import/source-1")?,
        }],
    )?;
    Ok(ArtifactMetadata::new(
        reference,
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        provenance,
    )?)
}

#[test]
fn structured_lineages_isolate_sibling_branch_value_streams()
-> Result<(), Box<dyn std::error::Error>> {
    let root = WorkspaceScope::run_root(RunId::new("run-1")?, ScopeId::new("root")?);
    let branch_a =
        WorkspaceScope::branch(ScopeId::new("scope-a")?, &root, BranchId::new("branch-a")?)?;
    let branch_b =
        WorkspaceScope::branch(ScopeId::new("scope-b")?, &root, BranchId::new("branch-b")?)?;
    let lineage_a = ScopeLineage::new(vec![root.clone(), branch_a.clone()])?;
    let lineage_b = ScopeLineage::new(vec![root.clone(), branch_b])?;

    let input = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("request")?,
        WorkspaceValue::Json(bounded(json!({"prompt": "hello"}))?),
    );
    assert!(lineage_a.can_read(input.reference()));
    assert!(lineage_b.can_read(input.reference()));

    let local_a = WorkspaceValueEntry::inherited(
        branch_a.reference().clone(),
        ValueKey::new("request")?,
        input.reference().clone(),
        input.value().clone(),
    )?;
    assert!(lineage_a.owns_value_stream(local_a.reference()));
    assert!(!lineage_b.can_read(local_a.reference()));
    assert!(!lineage_b.owns_value_stream(local_a.reference()));

    let next_a = WorkspaceValueEntry::successor(
        local_a.reference().clone(),
        WorkspaceValue::Json(bounded(json!({"prompt": "branch-a"}))?),
    )?;
    assert_eq!(next_a.reference().version().get(), 2);
    assert!(matches!(next_a.origin(), ValueOrigin::Successor { .. }));

    let encoded = serde_json::to_vec(&next_a)?;
    let decoded: WorkspaceValueEntry = serde_json::from_slice(&encoded)?;
    assert_eq!(decoded, next_a);
    Ok(())
}

#[test]
fn cross_run_imports_begin_a_new_parent_local_stream() -> Result<(), Box<dyn std::error::Error>> {
    let child = WorkspaceValueReference::new(
        ScopeReference::new(RunId::new("child-run")?, ScopeId::new("child-root")?),
        ValueKey::new("result")?,
        ValueVersion::FIRST,
    );
    let parent_scope = ScopeReference::new(RunId::new("parent-run")?, ScopeId::new("subworkflow")?);
    let imported = WorkspaceValueEntry::imported(
        parent_scope.clone(),
        ValueKey::new("child-result")?,
        child.clone(),
        WorkspaceValue::Json(bounded(json!({"ok": true}))?),
    )?;
    assert_eq!(imported.reference().version(), ValueVersion::FIRST);
    assert!(matches!(
        imported.origin(),
        ValueOrigin::Imported { source } if source == &child
    ));
    assert!(
        WorkspaceValueEntry::imported(
            parent_scope.clone(),
            ValueKey::new("invalid")?,
            WorkspaceValueReference::new(
                parent_scope,
                ValueKey::new("source")?,
                ValueVersion::FIRST,
            ),
            WorkspaceValue::Json(bounded(json!(null))?),
        )
        .is_err()
    );

    let mut wire = serde_json::to_value(&imported)?;
    wire["reference"]["version"] = json!(2);
    assert!(serde_json::from_value::<WorkspaceValueEntry>(wire).is_err());
    Ok(())
}

#[test]
fn scope_and_value_deserialization_rejects_forged_parentage_and_versions()
-> Result<(), Box<dyn std::error::Error>> {
    let parented_root = json!({
        "reference": {"run": "run-1", "scope": "root"},
        "kind": {"type": "run_root"},
        "parent": {"run": "run-1", "scope": "other"}
    });
    assert!(serde_json::from_value::<WorkspaceScope>(parented_root).is_err());

    let parentless_branch = json!({
        "reference": {"run": "run-1", "scope": "branch"},
        "kind": {"type": "branch", "branch": "left"},
        "parent": null
    });
    assert!(serde_json::from_value::<WorkspaceScope>(parentless_branch).is_err());

    let zero_version = json!({
        "scope": {"run": "run-1", "scope": "root"},
        "key": "answer",
        "version": 0
    });
    assert!(
        serde_json::from_value::<milkdrift_workspace::WorkspaceValueReference>(zero_version)
            .is_err()
    );

    let skipped_successor = json!({
        "reference": {
            "scope": {"run": "run-1", "scope": "root"},
            "key": "answer",
            "version": 3
        },
        "value": {"type": "json", "value": 42},
        "origin": {
            "type": "successor",
            "previous": {
                "scope": {"run": "run-1", "scope": "root"},
                "key": "answer",
                "version": 1
            }
        }
    });
    assert!(serde_json::from_value::<WorkspaceValueEntry>(skipped_successor).is_err());
    Ok(())
}

#[test]
fn artifacts_have_exact_content_facts_and_default_deny_export()
-> Result<(), Box<dyn std::error::Error>> {
    let content = br#"{"result":42}"#;
    let metadata = artifact(content)?;
    assert!(metadata.verifies(content));
    assert!(!metadata.verifies(br#"{"result":41}"#));
    assert_eq!(
        metadata.reference().media_type().as_str(),
        "application/json"
    );
    assert!(!metadata.sensitivity().permits_unauthorized_export());

    let mut wire = serde_json::to_value(&metadata)?;
    let object = wire
        .as_object_mut()
        .ok_or("metadata must encode as an object")?;
    object.remove("sensitivity");
    let defaulted: ArtifactMetadata = serde_json::from_value(wire)?;
    assert_eq!(defaulted.sensitivity(), ArtifactSensitivity::Restricted);

    let upper_digest = json!({
        "artifact": "artifact/report",
        "digest": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "media_type": "application/json",
        "size_bytes": 1
    });
    assert!(serde_json::from_value::<ArtifactReference>(upper_digest).is_err());
    assert!(MediaType::new("text/plain; charset=utf-8").is_err());
    assert!(MediaType::new("*/json").is_err());
    Ok(())
}

#[test]
fn provenance_is_bounded_unique_and_not_directly_self_referential()
-> Result<(), Box<dyn std::error::Error>> {
    let metadata = artifact(b"hello")?;
    let cause = CausalReference::External {
        source: CausalId::new("source")?,
    };
    assert!(ArtifactProvenance::new(cause.clone(), vec![cause.clone(), cause]).is_err());

    let self_reference = metadata.reference().clone();
    let self_provenance = ArtifactProvenance::new(
        CausalReference::Artifact {
            reference: self_reference.clone(),
        },
        Vec::new(),
    )?;
    assert!(
        ArtifactMetadata::new(
            self_reference,
            ArtifactSensitivity::Public,
            ArtifactRetention::Indefinite,
            self_provenance,
        )
        .is_err()
    );

    let causes: Vec<_> = (0..129)
        .map(|index| {
            Ok(CausalReference::External {
                source: CausalId::new(format!("source-{index}"))?,
            })
        })
        .collect::<Result<_, WorkspaceError>>()?;
    assert!(
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("producer")?,
            },
            causes,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn budgets_apply_per_item_and_aggregate_limits_without_wrapping()
-> Result<(), Box<dyn std::error::Error>> {
    let value = WorkspaceValue::Json(bounded(json!({"a": 1}))?);
    let encoded_size = u64::try_from(serde_json::to_vec(value.as_json().ok_or("json")?)?.len())?;
    let artifact = artifact(b"12345")?;
    let budget = WorkspaceBudget::new(2, encoded_size, encoded_size, 1, 5, 5)?;

    let after_value = budget.admit_value(&WorkspaceUsage::EMPTY, &value)?;
    assert_eq!(after_value.value_versions(), 1);
    assert_eq!(after_value.inline_bytes(), encoded_size);
    assert!(budget.admit_value(&after_value, &value).is_err());

    let after_artifact = budget.admit_artifact(&after_value, &artifact)?;
    assert_eq!(after_artifact.artifacts(), 1);
    assert_eq!(after_artifact.artifact_bytes(), 5);
    assert!(budget.admit_artifact(&after_artifact, &artifact).is_err());

    let unbounded_counts = WorkspaceBudget::new(u64::MAX, 0, 0, u64::MAX, 0, 0)?;
    let saturated = WorkspaceUsage::new(u64::MAX, 0, 0, 0);
    assert!(matches!(
        unbounded_counts.admit_value(
            &saturated,
            &WorkspaceValue::Artifact(artifact.reference().clone())
        ),
        Err(WorkspaceError::AccountingOverflow("value-version count"))
    ));
    Ok(())
}

#[test]
fn unknown_fields_and_oversized_identities_do_not_deserialize()
-> Result<(), Box<dyn std::error::Error>> {
    let unknown_scope_field = json!({
        "run": "run-1",
        "scope": "root",
        "surprise": true
    });
    assert!(serde_json::from_value::<ScopeReference>(unknown_scope_field).is_err());
    assert!(RunId::new("x".repeat(129)).is_err());
    assert!(ValueVersion::new(0).is_err());
    Ok(())
}

proptest! {
    #[test]
    fn arbitrary_identity_text_never_bypasses_validation(text in ".{0,300}") {
        if let Ok(identity) = RunId::new(text) {
            prop_assert!(!identity.as_str().is_empty());
            prop_assert!(identity.as_str().len() <= 128);
            prop_assert!(identity.as_str().is_ascii());
            prop_assert!(identity.as_str().as_bytes()[0].is_ascii_alphanumeric());
        }
    }

    #[test]
    fn digest_wire_form_round_trips(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let digest = ContentDigest::for_bytes(&bytes);
        let text = digest.to_string();
        prop_assert_eq!(text.len(), 64);
        prop_assert_eq!(ContentDigest::from_hex(&text)?, digest);
    }
}
