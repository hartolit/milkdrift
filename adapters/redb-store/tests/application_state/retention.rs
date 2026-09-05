use super::*;

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
