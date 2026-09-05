//! Application receipt, layout, proposal-index, audit, restart, and fault contracts.

use std::sync::Arc;

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
use milkdrift_redb_store::{FaultPoint, RedbStore, RedbStoreConfig};
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

#[path = "support/fault.rs"]
mod fault;
use fault::FailOnce;

#[path = "application_state/retention.rs"]
mod retention;
