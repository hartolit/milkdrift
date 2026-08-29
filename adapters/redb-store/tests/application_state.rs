//! Application receipt, layout, proposal-index, audit, restart, and fault contracts.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_blueprint::{RevisionId, WorkflowId};
use milkdrift_persistence::{
    ApplicationCommandCommit, ApplicationCommandCommitOutcome, ApplicationCommandEffect,
    ApplicationCommandReceipt, ApplicationCommandResult, ApplicationCommandStore,
    ApplicationEffectReference, ApplicationLayoutStore, ApplicationLayoutUpdate,
    ApplicationPageQuery, CommandId, IntegrityDigest, IntegrityScanRequest, PageSize,
    PersistenceError, ProposalIndexEntry, ProposalIndexStore, SecurityAuditEntry,
    SecurityAuditStore, StorageAdmin, TimestampMillis,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::RunId;
use redb::{Database, TableDefinition};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn receipts_layouts_proposals_and_audit_are_incremental_and_restart_durable() -> TestResult {
    let directory = tempfile::tempdir()?;
    let workflow = WorkflowId::new("workflow-application")?;
    let revision = revision_id('1')?;
    let layout_one = b"layout-generation-one".to_vec();
    let layout_one_digest = IntegrityDigest::hash(&layout_one);
    let layout_reference = ApplicationEffectReference::Layout {
        workflow: workflow.clone(),
        revision: revision.clone(),
        generation: 1,
        digest: layout_one_digest.clone(),
    };
    let first = receipt(
        "actor:application",
        "command-layout-one",
        b"put-layout-one",
        Some(layout_reference),
    )?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_application_limits(16, 2),
    )?;
    assert_eq!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: first.clone(),
            effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                layout_schema_version: 1,
                workflow: workflow.clone(),
                revision: revision.clone(),
                generation: 1,
                digest: layout_one_digest,
                author: first.actor().clone(),
                updated_at: TimestampMillis::new(10),
                document: layout_one.clone(),
            }),
        })?,
        ApplicationCommandCommitOutcome::Committed
    );
    let replay = store.commit_application_command(&ApplicationCommandCommit {
        receipt: first.clone(),
        effect: ApplicationCommandEffect::None,
    })?;
    assert!(matches!(replay, ApplicationCommandCommitOutcome::Replayed(value) if *value == first));

    let conflicting = receipt(
        "actor:application",
        "command-layout-one",
        b"different-command-content",
        None,
    )?;
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: conflicting,
            effect: ApplicationCommandEffect::None,
        }),
        Err(PersistenceError::ExternalCommandIdempotencyConflict { .. })
    ));

    let stored = store
        .application_layout(&workflow, &revision)?
        .ok_or("layout was not committed")?;
    assert_eq!(stored.document(), layout_one);
    assert_eq!(stored.generation(), 1);
    assert_eq!(stored.created_at(), TimestampMillis::new(10));

    let layout_two = b"layout-generation-two".to_vec();
    let layout_two_digest = IntegrityDigest::hash(&layout_two);
    let second = receipt(
        "actor:application",
        "command-layout-two",
        b"put-layout-two",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 2,
            digest: layout_two_digest.clone(),
        }),
    )?;
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: second,
        effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
            layout_schema_version: 1,
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 2,
            digest: layout_two_digest,
            author: ActorRef::new("actor:application")?,
            updated_at: TimestampMillis::new(20),
            document: layout_two.clone(),
        }),
    })?;
    let updated = store
        .application_layout(&workflow, &revision)?
        .ok_or("updated layout disappeared")?;
    assert_eq!(updated.document(), layout_two);
    assert_eq!(updated.created_at(), TimestampMillis::new(10));
    assert_eq!(updated.updated_at(), TimestampMillis::new(20));

    let run = RunId::new("run-application")?;
    let proposal_revision = revision_id('2')?;
    let proposal_receipt = receipt(
        "actor:application",
        "command-proposal",
        b"submit-proposal",
        Some(ApplicationEffectReference::Proposal {
            run: run.clone(),
            proposal: "proposal-application".to_owned(),
            proposed_revision: proposal_revision.clone(),
        }),
    )?;
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: proposal_receipt.clone(),
        effect: ApplicationCommandEffect::IndexProposal(ProposalIndexEntry {
            run: run.clone(),
            proposal: "proposal-application".to_owned(),
            proposed_revision: proposal_revision,
            receipt_actor: proposal_receipt.actor().clone(),
            receipt_command: proposal_receipt.command().clone(),
            created_at: TimestampMillis::new(10),
        }),
    })?;
    let proposals = store.proposal_index(&run, &page(10)?)?;
    assert_eq!(proposals.items.len(), 1);
    assert_eq!(proposals.items[0].proposal, "proposal-application");
    assert_eq!(store.rebuild_proposal_index()?, 1);

    for index in 0..3 {
        store.append_security_audit(&SecurityAuditEntry {
            evaluated_at: TimestampMillis::new(30 + index),
            actor: ActorRef::new("actor:application")?,
            grant: GrantId::new("grant-application")?,
            grant_revision: 1,
            grant_digest: digest('a')?,
            operation: "read_artifact_content".to_owned(),
            resource_digest: IntegrityDigest::hash(format!("resource-{index}").as_bytes()),
            decision_digest: IntegrityDigest::hash(format!("decision-{index}").as_bytes())
                .to_string(),
            outcome: "allowed".to_owned(),
            reason_codes: vec!["allowed".to_owned()],
        })?;
    }
    let audit = store.security_audit(&page(10)?)?;
    assert_eq!(audit.items.len(), 2);
    assert_eq!(audit.items[0].sequence, 2);
    assert_eq!(audit.items[1].sequence, 3);

    let mut cursor = None;
    loop {
        let scan = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(1)?,
            verify_artifact_content: false,
            cursor,
        })?;
        assert!(
            scan.failures.is_empty(),
            "integrity failures: {:?}",
            scan.failures
        );
        cursor = scan.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let receipt_page = store.application_command_receipts(&page(2)?)?;
    assert_eq!(receipt_page.items.len(), 2);
    assert!(receipt_page.next.is_some());
    let layout_page = store.application_layouts(&page(1)?)?;
    assert_eq!(layout_page.items.len(), 1);

    drop(store);
    let reopened = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_application_limits(16, 2),
    )?;
    assert_eq!(
        reopened
            .application_layout(&workflow, &revision)?
            .ok_or("layout did not survive reopen")?
            .document(),
        layout_two
    );
    assert_eq!(reopened.proposal_index(&run, &page(10)?)?.items.len(), 1);
    assert_eq!(reopened.security_audit(&page(10)?)?.items.len(), 2);
    Ok(())
}

#[test]
fn application_transaction_faults_distinguish_before_from_after_commit() -> TestResult {
    for (point, committed) in [
        (FaultPoint::BeforeApplicationCommit, false),
        (FaultPoint::AfterApplicationCommit, true),
    ] {
        let directory = tempfile::tempdir()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_application_limits(8, 8)
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let receipt = receipt("actor:fault", "command-fault", b"fault-boundary", None)?;
        assert!(
            store
                .commit_application_command(&ApplicationCommandCommit {
                    receipt: receipt.clone(),
                    effect: ApplicationCommandEffect::None,
                })
                .is_err()
        );
        let observed = store.application_command_receipt(receipt.actor(), receipt.command())?;
        assert_eq!(observed.is_some(), committed, "fault point {point:?}");
        if committed {
            assert!(matches!(
                store.commit_application_command(&ApplicationCommandCommit {
                    receipt,
                    effect: ApplicationCommandEffect::None,
                })?,
                ApplicationCommandCommitOutcome::Replayed(_)
            ));
        }
    }
    Ok(())
}

#[test]
fn malformed_application_rows_surface_typed_corruption() -> TestResult {
    const LAYOUTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.application.layouts");
    let directory = tempfile::tempdir()?;
    let workflow = WorkflowId::new("workflow-corrupt-application")?;
    let revision = revision_id('3')?;
    let document = b"layout-before-corruption".to_vec();
    let digest = IntegrityDigest::hash(&document);
    let receipt = receipt(
        "actor:corrupt-application",
        "command-corrupt-layout",
        b"put-layout-corrupt",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 1,
            digest: digest.clone(),
        }),
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: receipt.clone(),
            effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                layout_schema_version: 1,
                workflow: workflow.clone(),
                revision: revision.clone(),
                generation: 1,
                digest,
                author: receipt.actor().clone(),
                updated_at: TimestampMillis::new(10),
                document,
            }),
        })?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut layouts = write.open_table(LAYOUTS)?;
        let key = compound_key(&[workflow.as_str(), revision.as_str()]);
        layouts.insert(key.as_slice(), b"not-json".as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let result = store.application_layout(&workflow, &revision);
    assert!(
        matches!(result, Err(PersistenceError::Corruption(_))),
        "expected typed application corruption, got {result:?}"
    );
    Ok(())
}

fn receipt(
    actor: &str,
    command: &str,
    canonical_command: &[u8],
    effect: Option<ApplicationEffectReference>,
) -> Result<ApplicationCommandReceipt, PersistenceError> {
    ApplicationCommandReceipt::new(
        ActorRef::new(actor).map_err(authority_error)?,
        CommandId::new(command)?,
        1,
        IntegrityDigest::hash(canonical_command),
        GrantId::new("grant-application").map_err(authority_error)?,
        1,
        digest('a').map_err(authority_error)?,
        Some(IntegrityDigest::hash(b"authority-decision").to_string()),
        TimestampMillis::new(10),
        TimestampMillis::new(10),
        ApplicationCommandResult::Accepted {
            document: br#"{"accepted":true}"#.to_vec(),
            effect,
        },
    )
}

fn page(limit: u32) -> Result<ApplicationPageQuery, PersistenceError> {
    Ok(ApplicationPageQuery {
        after: None,
        limit: PageSize::new(limit)?,
    })
}

fn revision_id(hex: char) -> Result<RevisionId, serde_json::Error> {
    serde_json::from_value(serde_json::Value::String(format!(
        "rev_{}",
        hex.to_string().repeat(64)
    )))
}

fn digest(hex: char) -> Result<GrantDigest, milkdrift_authority::AuthorityError> {
    GrantDigest::new(format!("b3_{}", hex.to_string().repeat(64)))
}

fn authority_error(error: milkdrift_authority::AuthorityError) -> PersistenceError {
    PersistenceError::InvalidDocument(error.to_string())
}

fn compound_key(components: &[&str]) -> Vec<u8> {
    let mut key = Vec::new();
    for component in components {
        key.extend_from_slice(&(component.len() as u32).to_be_bytes());
        key.extend_from_slice(component.as_bytes());
    }
    key
}

struct FailOnce {
    point: FaultPoint,
    fired: AtomicBool,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            fired: AtomicBool::new(false),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        if point == self.point && !self.fired.swap(true, Ordering::SeqCst) {
            return Err(injected_failure(point));
        }
        Ok(())
    }
}
