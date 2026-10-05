use super::*;
use milkdrift_authority::WorkflowSet;
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Mutation, MutationBatch, Node, NodeKind, TerminalOutcome,
};
use milkdrift_persistence::{RevisionFilter, RevisionPageQuery};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_id");
const BY_WORKFLOW: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_workflow_and_id");

fn definition(name: &str) -> TestResult<BlueprintRevision> {
    Ok(BlueprintRevision::genesis(
        WorkflowId::new(name)?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:fixture")?,
        "index fixture",
    )?)
}
fn filter(names: &[&str]) -> TestResult<RevisionFilter> {
    Ok(RevisionFilter {
        workflows: Some(WorkflowSet::new(
            names
                .iter()
                .map(|name| WorkflowId::new(*name))
                .collect::<Result<Vec<_>, _>>()?,
        )?),
    })
}
fn key(revision: &BlueprintRevision) -> TestResult<Vec<u8>> {
    let mut result = Vec::new();
    for part in [
        revision.semantic().workflow().as_str(),
        revision.id().as_str(),
    ] {
        result.extend_from_slice(&u32::try_from(part.len())?.to_be_bytes());
        result.extend_from_slice(part.as_bytes());
    }
    Ok(result)
}

#[test]
fn scoped_revision_pages_do_not_decode_hidden_documents_and_keep_global_order() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let mut allowed = Vec::new();
    let mut hidden = Vec::new();
    for name in ["allowed-a", "allowed-b", "hidden"] {
        let mut base = definition(name)?;
        for _ in 0..4 {
            store.put_revision(&base)?;
            if name == "hidden" {
                hidden.push(base.id().to_string());
            } else {
                allowed.push(base.id().clone());
            }
            base = base.revise(
                base.id(),
                MutationBatch::new(vec![Mutation::SetMetadata {
                    metadata: base.semantic().metadata().clone(),
                }])?,
                AuthorRef::new("human:fixture")?,
                "different parent",
            )?;
        }
    }
    drop(store);
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut table = write.open_table(REVISIONS)?;
        for id in hidden {
            let _old = table.insert(id.as_str(), b"corrupt hidden document".as_slice())?;
        }
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(directory.path())?;
    let mut query = RevisionPageQuery {
        filter: filter(&["allowed-a", "allowed-b"])?,
        cursor: None,
        limit: PageSize::new(1)?,
    };
    let mut found = Vec::new();
    let mut first = None;
    for _ in 0..9 {
        let page = store.revisions(&query)?;
        assert!(page.revisions.len() <= 1);
        found.extend(page.revisions.into_iter().map(|revision| revision.revision));
        if first.is_none() {
            first = page.next.clone();
        }
        if page.next.is_none() {
            break;
        }
        assert_ne!(page.next, query.cursor);
        query.cursor = page.next;
    }
    allowed.sort();
    assert_eq!(found, allowed);
    query.filter = filter(&["allowed-a"])?;
    query.cursor = first;
    assert!(matches!(
        store.revisions(&query),
        Err(PersistenceError::InvalidCursor(_))
    ));
    query.filter = filter(&["absent"])?;
    query.cursor = None;
    let empty = store.revisions(&query)?;
    assert!(empty.revisions.is_empty());
    assert!(empty.next.is_none());
    Ok(())
}

#[test]
fn workflow_index_corruption_is_refused_by_reads_and_paged_integrity_scan() -> TestResult {
    for damage in ["missing", "wrong-summary", "dangling"] {
        let directory = TempDir::new()?;
        let revision = definition("allowed")?;
        let store = RedbStore::open(directory.path())?;
        store.put_revision(&revision)?;
        drop(store);
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut table = write.open_table(BY_WORKFLOW)?;
            match damage {
                "missing" => {
                    let _removed = table.remove(key(&revision)?.as_slice())?;
                }
                "wrong-summary" => {
                    let _old = table.insert(key(&revision)?.as_slice(), b"invalid".as_slice())?;
                }
                "dangling" => {
                    let mut revisions = write.open_table(REVISIONS)?;
                    let _removed = revisions.remove(revision.id().as_str())?;
                }
                _ => return Err("unexpected damage".into()),
            }
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open(directory.path())?;
        if damage == "missing" {
            assert!(store.revision(revision.id()).is_err());
        } else {
            assert!(
                store
                    .revisions(&RevisionPageQuery {
                        filter: filter(&["allowed"])?,
                        cursor: None,
                        limit: PageSize::new(1)?
                    })
                    .is_err()
            );
        }
        let mut cursor = None;
        let mut detected = false;
        for _ in 0..100 {
            match store.scan_integrity(IntegrityScanRequest {
                limit: PageSize::new(1)?,
                verify_artifact_content: false,
                cursor,
            }) {
                Err(_) => {
                    detected = true;
                    break;
                }
                Ok(page) => {
                    if !page.failures.is_empty() {
                        detected = true;
                        break;
                    }
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
            }
        }
        assert!(detected, "{damage}");
    }
    Ok(())
}
