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
    ApplicationPageQuery, ApplicationReceiptArchiveRequest, CommandId, IntegrityDigest,
    IntegrityScanRequest, PageSize, PersistenceError, ProposalIndexEntry, ProposalIndexStore,
    SecurityAuditEntry, SecurityAuditStore, StorageAdmin, StorageFailureClass, TimestampMillis,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::RunId;
use redb::{Database, ReadableTable as _, TableDefinition};

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
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(16, 2)
            .with_security_audit_limit(2),
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

    let same_layout_receipt = receipt(
        "actor:application",
        "command-layout-same",
        b"put-layout-same",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 1,
            digest: stored.digest().clone(),
        }),
    )?;
    assert_eq!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: same_layout_receipt.clone(),
            effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                layout_schema_version: 1,
                workflow: workflow.clone(),
                revision: revision.clone(),
                generation: 1,
                digest: stored.digest().clone(),
                author: same_layout_receipt.actor().clone(),
                updated_at: TimestampMillis::new(11),
                document: layout_one.clone(),
            }),
        })?,
        ApplicationCommandCommitOutcome::Committed
    );
    let wrong_generation_receipt = receipt(
        "actor:application",
        "command-layout-same-digest-wrong-generation",
        b"put-layout-same-digest-wrong-generation",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 2,
            digest: stored.digest().clone(),
        }),
    )?;
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: wrong_generation_receipt.clone(),
            effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                layout_schema_version: 1,
                workflow: workflow.clone(),
                revision: revision.clone(),
                generation: 2,
                digest: stored.digest().clone(),
                author: wrong_generation_receipt.actor().clone(),
                updated_at: TimestampMillis::new(12),
                document: layout_one.clone(),
            }),
        }),
        Err(PersistenceError::Corruption(_))
    ));
    let dishonest_layout_receipt = receipt(
        "actor:application",
        "command-layout-same-digest-different-document",
        b"put-layout-same-digest-different-document",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 1,
            digest: stored.digest().clone(),
        }),
    )?;
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: dishonest_layout_receipt.clone(),
            effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                layout_schema_version: 1,
                workflow: workflow.clone(),
                revision: revision.clone(),
                generation: 1,
                digest: stored.digest().clone(),
                author: dishonest_layout_receipt.actor().clone(),
                updated_at: TimestampMillis::new(12),
                document: b"different bytes under the old digest".to_vec(),
            }),
        }),
        Err(PersistenceError::Corruption(_))
    ));

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
            proposed_revision: proposal_revision.clone(),
            receipt_actor: proposal_receipt.actor().clone(),
            receipt_command: proposal_receipt.command().clone(),
            created_at: TimestampMillis::new(10),
        }),
    })?;
    let duplicate_proposal_receipt = receipt(
        "actor:application",
        "command-proposal-duplicate-index",
        b"submit-proposal-duplicate-index",
        Some(ApplicationEffectReference::Proposal {
            run: run.clone(),
            proposal: "proposal-application".to_owned(),
            proposed_revision: proposal_revision.clone(),
        }),
    )?;
    assert_eq!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: duplicate_proposal_receipt.clone(),
            effect: ApplicationCommandEffect::IndexProposal(ProposalIndexEntry {
                run: run.clone(),
                proposal: "proposal-application".to_owned(),
                proposed_revision: proposal_revision,
                receipt_actor: duplicate_proposal_receipt.actor().clone(),
                receipt_command: duplicate_proposal_receipt.command().clone(),
                created_at: TimestampMillis::new(11),
            }),
        })?,
        ApplicationCommandCommitOutcome::Committed
    );
    let conflicting_proposal_revision = revision_id('4')?;
    let conflicting_proposal_receipt = receipt(
        "actor:application",
        "command-proposal-conflicting-revision",
        b"submit-proposal-conflicting-revision",
        Some(ApplicationEffectReference::Proposal {
            run: run.clone(),
            proposal: "proposal-application".to_owned(),
            proposed_revision: conflicting_proposal_revision.clone(),
        }),
    )?;
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: conflicting_proposal_receipt.clone(),
            effect: ApplicationCommandEffect::IndexProposal(ProposalIndexEntry {
                run: run.clone(),
                proposal: "proposal-application".to_owned(),
                proposed_revision: conflicting_proposal_revision,
                receipt_actor: conflicting_proposal_receipt.actor().clone(),
                receipt_command: conflicting_proposal_receipt.command().clone(),
                created_at: TimestampMillis::new(12),
            }),
        }),
        Err(PersistenceError::Corruption(_))
    ));
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
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(16, 2)
            .with_security_audit_limit(2),
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
fn hot_receipt_capacity_reclaims_and_exact_cold_replay_remains_lifetime_durable() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(1, 1)
            .with_security_audit_limit(1),
    )?;
    let first = receipt(
        "actor:capacity",
        "command-capacity",
        b"fills-the-only-slot",
        None,
    )?;
    assert_eq!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: first.clone(),
            effect: ApplicationCommandEffect::None,
        })?,
        ApplicationCommandCommitOutcome::Committed
    );
    for ordinal in 0..32 {
        let next = receipt(
            "actor:capacity",
            &format!("command-capacity-{ordinal:02}"),
            format!("capacity-command-{ordinal}").as_bytes(),
            None,
        )?;
        assert_eq!(
            store.commit_application_command(&ApplicationCommandCommit {
                receipt: next,
                effect: ApplicationCommandEffect::None,
            })?,
            ApplicationCommandCommitOutcome::Committed
        );
    }
    let status = store.application_receipt_status()?;
    assert_eq!(status.hot_count, 1);
    assert_eq!(status.cold_count, 32);
    assert!(status.archive_generation >= 32);
    let replay = store.commit_application_command(&ApplicationCommandCommit {
        receipt: first.clone(),
        effect: ApplicationCommandEffect::None,
    })?;
    assert!(matches!(replay, ApplicationCommandCommitOutcome::Replayed(value) if *value == first));
    let conflicting = receipt(
        "actor:capacity",
        "command-capacity",
        b"different-cold-command",
        None,
    )?;
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: conflicting,
            effect: ApplicationCommandEffect::None,
        }),
        Err(PersistenceError::ExternalCommandIdempotencyConflict { .. })
    ));
    Ok(())
}

#[test]
fn reopen_reestablishes_smaller_receipt_and_audit_bounds_before_ready() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut receipts = Vec::new();
    {
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_application_receipt_lifecycle(8, 2)
                .with_security_audit_limit(8),
        )?;
        for ordinal in 0..6 {
            let value = receipt(
                "actor:downsized",
                &format!("command-downsized-{ordinal}"),
                format!("downsized-{ordinal}").as_bytes(),
                None,
            )?;
            store.commit_application_command(&ApplicationCommandCommit {
                receipt: value.clone(),
                effect: ApplicationCommandEffect::None,
            })?;
            receipts.push(value);
            store.append_security_audit(&SecurityAuditEntry {
                evaluated_at: TimestampMillis::new(100 + ordinal),
                actor: ActorRef::new("actor:downsized")?,
                grant: GrantId::new("grant-downsized")?,
                grant_revision: 1,
                grant_digest: digest('d')?,
                operation: "inspect".to_owned(),
                resource_digest: IntegrityDigest::hash(format!("resource-{ordinal}").as_bytes()),
                decision_digest: IntegrityDigest::hash(format!("decision-{ordinal}").as_bytes())
                    .to_string(),
                outcome: "allowed".to_owned(),
                reason_codes: vec!["allowed".to_owned()],
            })?;
        }
    }

    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(2, 1)
            .with_security_audit_limit(2),
    )?;
    let status = store.application_receipt_status()?;
    assert_eq!(status.hot_count, 2);
    assert_eq!(status.cold_count, 4);
    let audit = store.security_audit(&page(10)?)?;
    assert_eq!(
        audit
            .items
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        [5, 6]
    );

    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: receipts[0].clone(),
            effect: ApplicationCommandEffect::None,
        })?,
        ApplicationCommandCommitOutcome::Replayed(_)
    ));
    store.append_security_audit(&SecurityAuditEntry {
        evaluated_at: TimestampMillis::new(200),
        actor: ActorRef::new("actor:downsized")?,
        grant: GrantId::new("grant-downsized")?,
        grant_revision: 1,
        grant_digest: digest('d')?,
        operation: "inspect".to_owned(),
        resource_digest: IntegrityDigest::hash(b"resource-new"),
        decision_digest: IntegrityDigest::hash(b"decision-new").to_string(),
        outcome: "allowed".to_owned(),
        reason_codes: vec!["allowed".to_owned()],
    })?;
    let audit = store.security_audit(&page(10)?)?;
    assert_eq!(
        audit
            .items
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        [6, 7]
    );
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
                .with_application_receipt_lifecycle(8, 2)
                .with_security_audit_limit(8)
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
fn explicit_archival_preserves_rejected_results_proposals_and_stale_generation_truth() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(4, 1)
            .with_security_audit_limit(4),
    )?;
    let run = RunId::new("run-cold-proposal")?;
    let proposed_revision = revision_id('4')?;
    let proposal = receipt(
        "actor:archive",
        "command-a-proposal",
        b"proposal-command",
        Some(ApplicationEffectReference::Proposal {
            run: run.clone(),
            proposal: "proposal-cold".to_owned(),
            proposed_revision: proposed_revision.clone(),
        }),
    )?;
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: proposal.clone(),
        effect: ApplicationCommandEffect::IndexProposal(ProposalIndexEntry {
            run: run.clone(),
            proposal: "proposal-cold".to_owned(),
            proposed_revision,
            receipt_actor: proposal.actor().clone(),
            receipt_command: proposal.command().clone(),
            created_at: proposal.completed_at(),
        }),
    })?;
    let rejected = rejected_receipt("actor:archive", "command-z-rejected", b"rejected-command")?;
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: rejected.clone(),
        effect: ApplicationCommandEffect::None,
    })?;

    let before = store.application_receipt_status()?;
    let outcome = store.archive_application_command_receipts(ApplicationReceiptArchiveRequest {
        expected_generation: before.archive_generation,
        archived_at: TimestampMillis::new(20),
    })?;
    assert_eq!(outcome.archived, 1);
    assert_eq!(outcome.status.hot_count, 1);
    assert_eq!(outcome.status.cold_count, 1);
    assert_eq!(
        outcome.status.last_archived_at,
        Some(TimestampMillis::new(20))
    );
    assert!(matches!(
        store.archive_application_command_receipts(ApplicationReceiptArchiveRequest {
            expected_generation: before.archive_generation,
            archived_at: TimestampMillis::new(21),
        }),
        Err(
            PersistenceError::ApplicationReceiptArchiveGenerationConflict {
                expected: 0,
                actual: 1
            }
        )
    ));
    assert_eq!(
        store
            .application_command_receipt(proposal.actor(), proposal.command())?
            .ok_or("cold proposal receipt disappeared")?,
        proposal
    );
    assert_eq!(store.proposal_index(&run, &page(10)?)?.items.len(), 1);
    assert_eq!(store.rebuild_proposal_index()?, 1);

    let next = store.archive_application_command_receipts(ApplicationReceiptArchiveRequest {
        expected_generation: outcome.status.archive_generation,
        archived_at: TimestampMillis::new(22),
    })?;
    assert_eq!(next.archived, 1);
    assert_eq!(
        store
            .application_command_receipt(rejected.actor(), rejected.command())?
            .ok_or("cold rejected receipt disappeared")?,
        rejected
    );
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: rejected,
            effect: ApplicationCommandEffect::None,
        })?,
        ApplicationCommandCommitOutcome::Replayed(_)
    ));
    Ok(())
}

#[test]
fn archival_fault_boundaries_are_atomic_restart_safe_and_idempotent() -> TestResult {
    for (point, committed) in [
        (FaultPoint::BeforeApplicationReceiptColdInsert, false),
        (FaultPoint::AfterApplicationReceiptColdInsert, false),
        (FaultPoint::AfterApplicationReceiptHotRemove, false),
        (FaultPoint::BeforeApplicationReceiptArchiveCommit, false),
        (FaultPoint::AfterApplicationReceiptArchiveCommit, true),
    ] {
        let directory = tempfile::tempdir()?;
        let stored = receipt(
            "actor:archive-fault",
            "command-archive-fault",
            b"archive-fault-command",
            None,
        )?;
        {
            let store = RedbStore::open_with_config(
                RedbStoreConfig::new(directory.path())
                    .with_application_receipt_lifecycle(2, 1)
                    .with_security_audit_limit(2)
                    .with_fault_injector(Arc::new(FailOnce::new(point))),
            )?;
            store.commit_application_command(&ApplicationCommandCommit {
                receipt: stored.clone(),
                effect: ApplicationCommandEffect::None,
            })?;
            assert!(
                store
                    .archive_application_command_receipts(ApplicationReceiptArchiveRequest {
                        expected_generation: 0,
                        archived_at: TimestampMillis::new(30),
                    })
                    .is_err()
            );
        }
        let reopened = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_application_receipt_lifecycle(2, 1)
                .with_security_audit_limit(2),
        )?;
        let status = reopened.application_receipt_status()?;
        assert_eq!(status.cold_count, u64::from(committed));
        assert_eq!(status.hot_count, u64::from(!committed));
        assert_eq!(
            reopened
                .application_command_receipt(stored.actor(), stored.command())?
                .ok_or("receipt was lost at archival boundary")?,
            stored
        );
        if !committed {
            assert_eq!(
                reopened
                    .archive_application_command_receipts(ApplicationReceiptArchiveRequest {
                        expected_generation: status.archive_generation,
                        archived_at: TimestampMillis::new(31),
                    })?
                    .archived,
                1
            );
        }
    }
    Ok(())
}

#[test]
fn automatic_archival_failure_aborts_the_new_receipt_and_same_store_effect() -> TestResult {
    let directory = tempfile::tempdir()?;
    let original = receipt(
        "actor:automatic-archive",
        "command-original",
        b"original-command",
        None,
    )?;
    let workflow = WorkflowId::new("workflow-automatic-archive")?;
    let revision = revision_id('6')?;
    let document = b"must-not-survive-aborted-archive".to_vec();
    let digest = IntegrityDigest::hash(&document);
    let next = receipt(
        "actor:automatic-archive",
        "command-next",
        b"next-command",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 1,
            digest: digest.clone(),
        }),
    )?;
    {
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_application_receipt_lifecycle(1, 1)
                .with_security_audit_limit(1)
                .with_fault_injector(Arc::new(FailOnce::new(
                    FaultPoint::AfterApplicationReceiptHotRemove,
                ))),
        )?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: original.clone(),
            effect: ApplicationCommandEffect::None,
        })?;
        assert!(
            store
                .commit_application_command(&ApplicationCommandCommit {
                    receipt: next.clone(),
                    effect: ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
                        layout_schema_version: 1,
                        workflow: workflow.clone(),
                        revision: revision.clone(),
                        generation: 1,
                        digest,
                        author: next.actor().clone(),
                        updated_at: TimestampMillis::new(10),
                        document,
                    }),
                })
                .is_err()
        );
    }
    let reopened = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(1, 1)
            .with_security_audit_limit(1),
    )?;
    assert_eq!(reopened.application_receipt_status()?.hot_count, 1);
    assert_eq!(reopened.application_receipt_status()?.cold_count, 0);
    assert_eq!(
        reopened
            .application_command_receipt(original.actor(), original.command())?
            .ok_or("original receipt disappeared after aborted automatic archival")?,
        original
    );
    assert!(
        reopened
            .application_command_receipt(next.actor(), next.command())?
            .is_none()
    );
    assert!(reopened.application_layout(&workflow, &revision)?.is_none());
    Ok(())
}

#[test]
fn merged_receipt_pagination_is_stable_while_rows_move_between_tiers() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(8, 2)
            .with_security_audit_limit(8),
    )?;
    for ordinal in 0..5 {
        let stored = receipt(
            "actor:page",
            &format!("command-page-{ordinal}"),
            format!("page-command-{ordinal}").as_bytes(),
            None,
        )?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: stored,
            effect: ApplicationCommandEffect::None,
        })?;
    }
    let first = store.application_command_receipts(&page(2)?)?;
    let cursor = first
        .next
        .clone()
        .ok_or("first receipt page omitted cursor")?;
    let status = store.application_receipt_status()?;
    store.archive_application_command_receipts(ApplicationReceiptArchiveRequest {
        expected_generation: status.archive_generation,
        archived_at: TimestampMillis::new(40),
    })?;
    let second = store.application_command_receipts(&ApplicationPageQuery {
        after: Some(cursor),
        limit: PageSize::new(10)?,
    })?;
    let commands = first
        .items
        .iter()
        .chain(&second.items)
        .map(|receipt| receipt.command().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            "command-page-0",
            "command-page-1",
            "command-page-2",
            "command-page-3",
            "command-page-4",
        ]
    );
    Ok(())
}

#[test]
fn cold_layout_replay_does_not_repeat_effect_and_audit_retention_is_independent() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(1, 1)
            .with_security_audit_limit(2),
    )?;
    let workflow = WorkflowId::new("workflow-cold-layout")?;
    let revision = revision_id('5')?;
    let document = b"cold-layout-document".to_vec();
    let layout_digest = IntegrityDigest::hash(&document);
    let layout_receipt = receipt(
        "actor:layout-replay",
        "command-layout-cold",
        b"put-cold-layout",
        Some(ApplicationEffectReference::Layout {
            workflow: workflow.clone(),
            revision: revision.clone(),
            generation: 1,
            digest: layout_digest.clone(),
        }),
    )?;
    let layout_effect = ApplicationCommandEffect::PutLayout(ApplicationLayoutUpdate {
        layout_schema_version: 1,
        workflow: workflow.clone(),
        revision: revision.clone(),
        generation: 1,
        digest: layout_digest,
        author: layout_receipt.actor().clone(),
        updated_at: TimestampMillis::new(10),
        document: document.clone(),
    });
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: layout_receipt.clone(),
        effect: layout_effect.clone(),
    })?;
    let second = receipt(
        "actor:layout-replay",
        "command-layout-turnover",
        b"turn-over-layout-receipt",
        None,
    )?;
    store.commit_application_command(&ApplicationCommandCommit {
        receipt: second,
        effect: ApplicationCommandEffect::None,
    })?;
    assert_eq!(store.application_receipt_status()?.cold_count, 1);
    assert!(matches!(
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: layout_receipt,
            effect: layout_effect,
        })?,
        ApplicationCommandCommitOutcome::Replayed(_)
    ));
    let layout = store
        .application_layout(&workflow, &revision)?
        .ok_or("layout disappeared after cold replay")?;
    assert_eq!(layout.generation(), 1);
    assert_eq!(layout.document(), document);

    for ordinal in 0..3 {
        store.append_security_audit(&SecurityAuditEntry {
            evaluated_at: TimestampMillis::new(50 + ordinal),
            actor: ActorRef::new("actor:layout-replay")?,
            grant: GrantId::new("grant-application")?,
            grant_revision: 1,
            grant_digest: digest('a')?,
            operation: "inspect_layout".to_owned(),
            resource_digest: IntegrityDigest::hash(format!("audit-resource-{ordinal}").as_bytes()),
            decision_digest: IntegrityDigest::hash(format!("audit-decision-{ordinal}").as_bytes())
                .to_string(),
            outcome: "allowed".to_owned(),
            reason_codes: vec!["allowed".to_owned()],
        })?;
    }
    assert_eq!(store.security_audit(&page(10)?)?.items.len(), 2);
    assert_eq!(store.application_receipt_status()?.cold_count, 1);
    assert_eq!(
        store.application_command_receipts(&page(10)?)?.items.len(),
        2
    );
    Ok(())
}

#[test]
#[ignore = "manual release-mode longevity proof across many hot receipt turnovers"]
fn release_receipt_longevity_crosses_many_hot_bounds_and_replays_after_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let first = receipt(
        "actor:longevity",
        "command-longevity-first",
        b"longevity-first",
        None,
    )?;
    {
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_application_receipt_lifecycle(17, 7)
                .with_security_audit_limit(17),
        )?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: first.clone(),
            effect: ApplicationCommandEffect::None,
        })?;
        for ordinal in 0..10_000 {
            let stored = receipt(
                "actor:longevity",
                &format!("command-longevity-{ordinal:05}"),
                format!("longevity-command-{ordinal}").as_bytes(),
                None,
            )?;
            store.commit_application_command(&ApplicationCommandCommit {
                receipt: stored,
                effect: ApplicationCommandEffect::None,
            })?;
        }
        let status = store.application_receipt_status()?;
        assert!(status.hot_count <= 17);
        assert!(status.cold_count > 9_900);
    }
    let reopened = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_application_receipt_lifecycle(17, 7)
            .with_security_audit_limit(17),
    )?;
    assert!(matches!(
        reopened.commit_application_command(&ApplicationCommandCommit {
            receipt: first,
            effect: ApplicationCommandEffect::None,
        })?,
        ApplicationCommandCommitOutcome::Replayed(_)
    ));
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

#[test]
fn startup_detects_receipt_counter_corruption_and_dual_tier_ownership() -> TestResult {
    const HOT: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v2.application.command_receipts.hot");
    const COLD: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v2.application.command_receipts.cold");
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");

    let counter_directory = tempfile::tempdir()?;
    let stored = receipt(
        "actor:counter-corruption",
        "command-counter-corruption",
        b"counter-corruption",
        None,
    )?;
    {
        let store = RedbStore::open(counter_directory.path())?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: stored,
            effect: ApplicationCommandEffect::None,
        })?;
    }
    let database = Database::open(counter_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write
        .open_table(METADATA)?
        .insert("application_hot_receipt_count", 0)?;
    write.commit()?;
    drop(database);
    assert!(matches!(
        RedbStore::open(counter_directory.path()),
        Err(PersistenceError::Corruption(_))
            | Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            })
    ));

    let dual_directory = tempfile::tempdir()?;
    let stored = receipt(
        "actor:dual-corruption",
        "command-dual-corruption",
        b"dual-corruption",
        None,
    )?;
    {
        let store = RedbStore::open(dual_directory.path())?;
        store.commit_application_command(&ApplicationCommandCommit {
            receipt: stored,
            effect: ApplicationCommandEffect::None,
        })?;
    }
    let database = Database::open(dual_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    let (key, bytes) = {
        let hot = write.open_table(HOT)?;
        let (key, value) = hot
            .iter()?
            .next()
            .transpose()?
            .ok_or("hot receipt absent")?;
        (key.value().to_vec(), value.value().to_vec())
    };
    write
        .open_table(COLD)?
        .insert(key.as_slice(), bytes.as_slice())?;
    write
        .open_table(METADATA)?
        .insert("application_cold_receipt_count", 1)?;
    write.commit()?;
    drop(database);
    assert!(matches!(
        RedbStore::open(dual_directory.path()),
        Err(PersistenceError::Corruption(_))
            | Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            })
    ));
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

fn rejected_receipt(
    actor: &str,
    command: &str,
    canonical_command: &[u8],
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
        ApplicationCommandResult::Rejected {
            document: br#"{"accepted":false}"#.to_vec(),
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
