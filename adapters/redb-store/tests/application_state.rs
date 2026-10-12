//! Security audit retention is independent of task command recovery.
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_persistence::{
    ApplicationPageQuery, IntegrityDigest, PageSize, SecurityAuditEntry, SecurityAuditStore,
    TimestampMillis,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn audit(index: u64) -> Result<SecurityAuditEntry, Box<dyn std::error::Error>> {
    Ok(SecurityAuditEntry {
        evaluated_at: TimestampMillis::new(index),
        actor: ActorRef::new("actor:audit")?,
        grant: GrantId::new("grant:audit")?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "a".repeat(64)))?,
        operation: "read_artifact_content".into(),
        resource_digest: IntegrityDigest::hash(format!("resource-{index}").as_bytes()),
        decision_digest: IntegrityDigest::hash(format!("decision-{index}").as_bytes()).to_string(),
        outcome: "allowed".into(),
        reason_codes: vec!["allowed".into()],
    })
}
#[test]
fn security_audit_prefix_is_bounded_paged_and_restart_durable() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(root.path()).with_security_audit_limit(3),
    )?;
    for index in 1..=5 {
        store.append_security_audit(&audit(index)?)?;
    }
    let first = store.security_audit(&ApplicationPageQuery {
        after: None,
        limit: PageSize::new(1)?,
    })?;
    assert_eq!(first.items[0].sequence, 3);
    let next = store.security_audit(&ApplicationPageQuery {
        after: first.next,
        limit: PageSize::new(10)?,
    })?;
    assert_eq!(
        next.items
            .iter()
            .map(|row| row.sequence)
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert!(next.next.is_none());
    drop(store);
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(root.path()).with_security_audit_limit(1),
    )?;
    let retained = store.security_audit(&ApplicationPageQuery {
        after: None,
        limit: PageSize::new(10)?,
    })?;
    assert_eq!(
        retained
            .items
            .iter()
            .map(|row| row.sequence)
            .collect::<Vec<_>>(),
        vec![5]
    );
    assert_eq!(store.append_security_audit(&audit(6)?)?.sequence, 6);
    Ok(())
}
