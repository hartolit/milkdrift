use super::*;
use super::{
    append::{
        predecessor_path, run_membership_path, run_membership_payload,
        validate_nonterminal_membership_in_transaction, workspace_domain_path,
        workspace_domain_payload,
    },
    queries::validated_run_head,
};
pub(crate) fn apply_workspace(
    write: &redb::WriteTransaction,
    request: &AtomicRunCommitRequest,
) -> Result<(), PersistenceError> {
    crate::trie::validate_roots_in_transaction(write)?;
    let mut scopes = write.open_table(SCOPES).map_err(error::redb)?;
    let mut roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let mut values = write.open_table(VALUES).map_err(error::redb)?;

    for mutation in request.workspace() {
        match mutation {
            WorkspaceMutation::CreateScope { scope } => {
                put_scope(write, &mut scopes, &mut roots, scope)?;
            }
            WorkspaceMutation::PutValue { entry } => {
                put_value(write, &scopes, &roots, &mut values, entry)?;
            }
        }
    }
    let Some(accounting) = request.workspace_accounting() else {
        return Ok(());
    };
    let value_delta = accounting
        .resulting_usage
        .value_versions()
        .checked_sub(accounting.expected_usage.value_versions())
        .ok_or_else(|| error::corruption("workspace value accounting moved backwards"))?;
    let mutation_count = u64::try_from(
        request
            .workspace()
            .iter()
            .filter(|mutation| matches!(mutation, WorkspaceMutation::PutValue { .. }))
            .count(),
    )
    .map_err(|_| error::corruption("workspace value mutation count exceeds u64"))?;
    if value_delta != mutation_count {
        return Err(error::corruption(
            "workspace usage delta does not match inserted value versions",
        ));
    }
    drop(values);
    drop(roots);
    drop(scopes);
    Ok(())
}

pub(crate) fn workspace_scope_run_group(run: &RunId) -> [u8; 16] {
    let family = crate::trie::CatalogFamily::WorkspaceScope;
    let hash = crate::trie::hashed_path(family, run.as_str().as_bytes());
    let mut group = [0_u8; 16];
    group.copy_from_slice(&hash[..16]);
    group
}

pub(crate) fn workspace_scope_catalog_path(
    reference: &ScopeReference,
    logical_key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::WorkspaceScope,
        &workspace_scope_run_group(reference.run()),
        logical_key,
    )
}

pub(crate) fn ensure_workspace_run_has_no_scopes(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let mut first = [0_u8; 32];
    first[..16].copy_from_slice(&workspace_scope_run_group(run));
    let page = crate::trie::page_in_transaction(
        write,
        crate::trie::CatalogFamily::WorkspaceScope,
        None,
        predecessor_path(first),
        1,
    )?;
    if page
        .leaves
        .first()
        .is_some_and(|leaf| leaf.path[..16] == first[..16])
    {
        return Err(error::corruption(
            "run has an authenticated workspace scope but no root-scope index",
        ));
    }
    Ok(())
}

pub(crate) fn validate_scope_catalog_lineage_in_transaction(
    write: &redb::WriteTransaction,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            workspace_scope_catalog_path(&current, &key)?,
            &key,
        )?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            return if witness.is_some() || depth > 0 {
                Err(error::corruption("workspace scope lineage is incomplete"))
            } else {
                Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                })
            };
        };
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace scope disagrees with its authenticated catalog",
            ));
        }
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}

pub(crate) fn validate_scope_catalog_lineage(
    read: &redb::ReadTransaction,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    require_run_history_membership(read, reference.run())?;
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member(
            read,
            family,
            workspace_scope_catalog_path(&current, &key)?,
            &key,
        )?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            return if witness.is_some() || depth > 0 {
                Err(error::corruption("workspace scope lineage is incomplete"))
            } else {
                Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                })
            };
        };
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace scope disagrees with its authenticated catalog",
            ));
        }
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}

pub(crate) fn require_run_history_membership(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
    let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    let head = validated_run_head(&heads, &events, run)?;
    if validate_run_history_membership(read, run, head)?.is_none() {
        return Err(error::corruption(
            "durable workspace fact has no authenticated owning run",
        ));
    }
    Ok(())
}

pub(crate) fn require_prior_run_history_membership_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
) -> Result<(), PersistenceError> {
    let head = {
        let heads = write.open_table(RUN_HEADS).map_err(error::redb)?;
        heads
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|head| RunSequence::new(head.value()))
            .ok_or_else(|| {
                error::corruption("cross-run workspace provenance has no durable run head")
            })?
    };
    let summary_bytes = {
        let summaries = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
        summaries
            .get(run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("cross-run workspace provenance has no run summary"))?
            .value()
            .to_vec()
    };
    let summary: RunSummaryIndex = json::decode(&summary_bytes, "run summary")?;
    if summary.run != *run || summary.through_sequence != head {
        return Err(error::corruption(
            "cross-run workspace provenance summary disagrees with its head",
        ));
    }
    let family = crate::trie::CatalogFamily::RunMembership;
    let witness = crate::trie::verify_member_in_transaction(
        write,
        family,
        run_membership_path(run),
        run.as_str().as_bytes(),
    )?;
    let payload = run_membership_payload(run, head, &summary_bytes);
    if witness != Some(payload) {
        return Err(error::corruption(
            "cross-run workspace provenance has no authenticated run membership",
        ));
    }
    validate_nonterminal_membership_in_transaction(write, &summary, payload)
}

pub(crate) fn validate_workspace_value_catalog_provenance_in_transaction(
    write: &redb::WriteTransaction,
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
    proposed: bool,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    for depth in 0..MAX_VALUE_PROVENANCE_DEPTH {
        if current.reference().scope().run() != selected.reference().scope().run() {
            require_prior_run_history_membership_in_transaction(
                write,
                current.reference().scope().run(),
            )?;
        }
        validate_scope_catalog_lineage_in_transaction(
            write,
            scopes,
            roots,
            current.reference().scope(),
        )?;
        if !(proposed && depth == 0) {
            let key = workspace_value_key(current.reference())?;
            let bytes = values
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace provenance value is missing"))?;
            let family = crate::trie::CatalogFamily::WorkspaceValue;
            let witness = crate::trie::verify_member_in_transaction(
                write,
                family,
                crate::trie::hashed_path(family, &key),
                &key,
            )?;
            if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
                return Err(error::corruption(
                    "workspace value disagrees with its authenticated catalog",
                ));
            }
        }
        let (source, missing_entity) = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => (previous, "previous_workspace_value"),
            ValueOrigin::Inherited { source } => (source, "inherited_workspace_value"),
            ValueOrigin::Imported { source } => (source, "imported_workspace_value"),
        };
        let key = workspace_value_key(source)?;
        let stored = values
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec());
        let family = crate::trie::CatalogFamily::WorkspaceValue;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            crate::trie::hashed_path(family, &key),
            &key,
        )?;
        current = match (stored.as_deref(), witness) {
            (Some(bytes), Some(witness))
                if witness == crate::trie::digest_payload(family, bytes) =>
            {
                let source_entry: WorkspaceValueEntry = json::decode(bytes, "workspace value")?;
                if source_entry.reference() != source {
                    return Err(error::corruption(
                        "workspace provenance key disagrees with its document",
                    ));
                }
                source_entry
            }
            (None, None) if proposed && depth == 0 => {
                return Err(PersistenceError::NotFound {
                    entity: missing_entity,
                    identity: format!(
                        "{}/{}/{}/{}",
                        source.scope().run(),
                        source.scope().scope(),
                        source.key(),
                        source.version()
                    ),
                });
            }
            _ => {
                return Err(error::corruption(
                    "workspace provenance source and authenticated catalog disagree",
                ));
            }
        };
    }
    Err(error::corruption(format!(
        "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
    )))
}

pub(crate) fn validate_workspace_value_catalog_provenance(
    read: &redb::ReadTransaction,
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    for _ in 0..MAX_VALUE_PROVENANCE_DEPTH {
        validate_scope_catalog_lineage(read, scopes, roots, current.reference().scope())?;
        let key = workspace_value_key(current.reference())?;
        let bytes = values
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace provenance value is missing"))?;
        let family = crate::trie::CatalogFamily::WorkspaceValue;
        let witness =
            crate::trie::verify_member(read, family, crate::trie::hashed_path(family, &key), &key)?;
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "workspace value disagrees with its authenticated catalog",
            ));
        }
        let source = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => previous,
            ValueOrigin::Inherited { source } | ValueOrigin::Imported { source } => source,
        };
        current = load_provenance_value(values, source, true, "workspace_value")?;
    }
    Err(error::corruption(format!(
        "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
    )))
}

pub(crate) fn update_workspace_value_head(
    write: &redb::WriteTransaction,
    reference: &WorkspaceValueReference,
    value_key: &[u8],
    value_bytes: &[u8],
) -> Result<(), PersistenceError> {
    let head_key = codec::value_prefix(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
    )?;
    let previous_bytes = {
        let heads = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        heads
            .get(head_key.as_slice())
            .map_err(error::redb)?
            .map(|bytes| bytes.value().to_vec())
    };
    let family = crate::trie::CatalogFamily::WorkspaceValueHead;
    let path = crate::trie::hashed_path(family, &head_key);
    let previous_witness =
        crate::trie::verify_member_in_transaction(write, family, path, &head_key)?;
    match previous_bytes.as_deref() {
        None if previous_witness.is_none() && reference.version().get() == 1 => {}
        Some(bytes) if previous_witness == Some(crate::trie::digest_payload(family, bytes)) => {
            let previous: WorkspaceValueReference = json::decode(bytes, "workspace value head")?;
            let expected_version = previous
                .version()
                .get()
                .checked_add(1)
                .ok_or_else(|| error::corruption("workspace value version overflowed"))?;
            if previous.scope() != reference.scope()
                || previous.key() != reference.key()
                || expected_version != reference.version().get()
            {
                return Err(error::corruption(
                    "workspace value head is not the immediate predecessor",
                ));
            }
        }
        None if previous_witness.is_none() => {
            return Err(error::corruption(
                "workspace value sequence begins after version one",
            ));
        }
        _ => {
            return Err(error::corruption(
                "workspace value head disagrees with its authenticated catalog",
            ));
        }
    }
    let head_bytes = json::encode(reference, "workspace value head")?;
    {
        let mut heads = write
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        let replaced = heads
            .insert(head_key.as_slice(), head_bytes.as_slice())
            .map_err(error::redb)?;
        if replaced.as_ref().map(|bytes| bytes.value()) != previous_bytes.as_deref() {
            return Err(error::corruption(
                "workspace value head changed outside its authoritative transaction",
            ));
        }
    }
    let replaced_witness = crate::trie::put(
        write,
        family,
        path,
        &head_key,
        crate::trie::digest_payload(family, &head_bytes),
    )?;
    if replaced_witness != previous_witness {
        return Err(error::corruption(
            "workspace value head witness changed outside its transaction",
        ));
    }
    let value_family = crate::trie::CatalogFamily::WorkspaceValue;
    let value_witness = crate::trie::verify_member_in_transaction(
        write,
        value_family,
        crate::trie::hashed_path(value_family, value_key),
        value_key,
    )?;
    if value_witness != Some(crate::trie::digest_payload(value_family, value_bytes)) {
        return Err(error::corruption(
            "workspace value head does not name an authenticated value",
        ));
    }
    Ok(())
}

pub(crate) fn put_scope(
    write: &redb::WriteTransaction,
    scopes: &mut Table<'_, &[u8], &[u8]>,
    roots: &mut Table<'_, &str, &str>,
    scope: &WorkspaceScope,
) -> Result<(), PersistenceError> {
    let reference = scope.reference();
    let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
    let family = crate::trie::CatalogFamily::WorkspaceScope;
    if crate::trie::verify_member_in_transaction(
        write,
        family,
        workspace_scope_catalog_path(reference, &key)?,
        &key,
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace scope catalog names an existing immutable scope",
        ));
    }
    if scopes.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_scope",
            identity: format!("{}/{}", reference.run(), reference.scope()),
        });
    }
    match (scope.kind(), scope.parent()) {
        (ScopeKind::RunRoot, None) => {
            ensure_workspace_run_has_no_scopes(write, reference.run())?;
            if roots
                .get(reference.run().as_str())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "workspace_root_scope",
                    identity: reference.run().to_string(),
                });
            }
            roots
                .insert(reference.run().as_str(), reference.scope().as_str())
                .map_err(error::redb)?;
        }
        (_, Some(parent)) => {
            validate_new_scope_depth(write, scopes, roots, parent)?;
        }
        _ => {
            return Err(PersistenceError::InvalidDocument(
                "workspace scope kind/parent invariant failed".to_owned(),
            ));
        }
    }
    let bytes = json::encode(scope, "workspace scope")?;
    if scopes
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "workspace scope insert replaced an existing document",
        ));
    }
    if crate::trie::put(
        write,
        family,
        workspace_scope_catalog_path(reference, &key)?,
        &key,
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace scope insert replaced an authenticated leaf",
        ));
    }
    Ok(())
}

pub(crate) fn put_value(
    write: &redb::WriteTransaction,
    scopes: &Table<'_, &[u8], &[u8]>,
    roots: &Table<'_, &str, &str>,
    values: &mut Table<'_, &[u8], &[u8]>,
    entry: &WorkspaceValueEntry,
) -> Result<(), PersistenceError> {
    let reference = entry.reference();
    let scope = reference.scope();
    validate_scope_catalog_lineage_in_transaction(write, scopes, roots, scope)?;
    let key = workspace_value_key(reference)?;
    let family = crate::trie::CatalogFamily::WorkspaceValue;
    if crate::trie::verify_member_in_transaction(
        write,
        family,
        crate::trie::hashed_path(family, &key),
        &key,
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace value catalog names an existing immutable value",
        ));
    }
    if values.get(key.as_slice()).map_err(error::redb)?.is_some() {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_value",
            identity: format!(
                "{}/{}/{}/{}",
                scope.run(),
                scope.scope(),
                reference.key(),
                reference.version()
            ),
        });
    }
    validate_workspace_value_catalog_provenance_in_transaction(
        write, values, scopes, roots, entry, true,
    )?;
    validate_workspace_value_provenance(values, scopes, roots, entry, true)?;
    let bytes = json::encode(entry, "workspace value")?;
    if values
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "workspace value insert replaced an existing document",
        ));
    }
    if crate::trie::put(
        write,
        family,
        crate::trie::hashed_path(family, &key),
        &key,
        crate::trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "workspace value insert replaced an authenticated leaf",
        ));
    }
    update_workspace_value_head(write, reference, &key, &bytes)?;
    Ok(())
}

pub(crate) fn load_provenance_value(
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    reference: &WorkspaceValueReference,
    missing_is_corruption: bool,
    missing_entity: &'static str,
) -> Result<WorkspaceValueEntry, PersistenceError> {
    let key = workspace_value_key(reference)?;
    let bytes = values.get(key.as_slice()).map_err(error::redb)?;
    let bytes = match bytes {
        Some(bytes) => bytes,
        None if missing_is_corruption => {
            return Err(error::corruption(format!(
                "workspace provenance source {}/{}/{}/{} is missing",
                reference.scope().run(),
                reference.scope().scope(),
                reference.key(),
                reference.version()
            )));
        }
        None => {
            return Err(PersistenceError::NotFound {
                entity: missing_entity,
                identity: format!(
                    "{}/{}/{}/{}",
                    reference.scope().run(),
                    reference.scope().scope(),
                    reference.key(),
                    reference.version()
                ),
            });
        }
    };
    let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
    if stored.reference() != reference {
        return Err(error::corruption(
            "workspace-value key does not match its document",
        ));
    }
    Ok(stored)
}

pub(crate) fn validate_workspace_value_provenance(
    values: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    selected: &WorkspaceValueEntry,
    proposed: bool,
) -> Result<(), PersistenceError> {
    let mut current = selected.clone();
    let mut seen = BTreeSet::new();
    for depth in 0..MAX_VALUE_PROVENANCE_DEPTH {
        if !seen.insert(current.reference().clone()) {
            return Err(error::corruption(
                "workspace value provenance contains a cycle",
            ));
        }
        validate_owning_workspace_scope(scopes, roots, current.reference().scope())?;
        let (source, missing_entity) = match current.origin() {
            ValueOrigin::Initial => return Ok(()),
            ValueOrigin::Successor { previous } => (previous, "previous_workspace_value"),
            ValueOrigin::Inherited { source } => {
                require_ancestor(scopes, source.scope(), current.reference().scope()).map_err(
                    |cause| {
                        if proposed && depth == 0 {
                            cause
                        } else {
                            error::corruption(format!(
                                "stored inherited workspace ancestry is invalid: {cause}"
                            ))
                        }
                    },
                )?;
                (source, "inherited_workspace_value")
            }
            ValueOrigin::Imported { source } => (source, "imported_workspace_value"),
        };
        let source_entry =
            load_provenance_value(values, source, !proposed || depth > 0, missing_entity)?;
        match current.origin() {
            ValueOrigin::Inherited { .. } if source_entry.value() != current.value() => {
                let message =
                    "an inherited workspace value must preserve its exact ancestor content";
                return if proposed && depth == 0 {
                    Err(PersistenceError::InvalidDocument(message.to_owned()))
                } else {
                    Err(error::corruption(message))
                };
            }
            ValueOrigin::Imported { .. } if source_entry.value() != current.value() => {
                let message = "an imported workspace value must preserve its exact source content";
                return if proposed && depth == 0 {
                    Err(PersistenceError::InvalidDocument(message.to_owned()))
                } else {
                    Err(error::corruption(message))
                };
            }
            _ => {}
        }
        current = source_entry;
    }
    if proposed {
        Err(PersistenceError::Bounds {
            location: "workspace_value.provenance",
            reason: format!("provenance may contain at most {MAX_VALUE_PROVENANCE_DEPTH} entries"),
        })
    } else {
        Err(error::corruption(format!(
            "workspace value provenance exceeds {MAX_VALUE_PROVENANCE_DEPTH} entries"
        )))
    }
}

pub(crate) fn validate_new_scope_depth(
    write: &redb::WriteTransaction,
    scopes: &Table<'_, &[u8], &[u8]>,
    roots: &Table<'_, &str, &str>,
    parent: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = Some(parent.clone());
    let mut total_depth = 1_usize; // Include the new child being validated.
    let mut seen = BTreeSet::new();
    while let Some(reference) = current {
        total_depth = total_depth.saturating_add(1);
        if total_depth > MAX_SCOPE_DEPTH {
            return Err(PersistenceError::InvalidDocument(format!(
                "workspace scope lineage may contain at most {MAX_SCOPE_DEPTH} entries"
            )));
        }
        if !seen.insert(reference.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(reference.run().as_str(), reference.scope().as_str())?;
        let bytes = scopes.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = bytes else {
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            if crate::trie::verify_member_in_transaction(
                write,
                family,
                workspace_scope_catalog_path(&reference, &key)?,
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "parent workspace scope catalog names a missing document",
                ));
            }
            return Err(PersistenceError::NotFound {
                entity: "parent_workspace_scope",
                identity: format!("{}/{}", reference.run(), reference.scope()),
            });
        };
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &reference {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        let family = crate::trie::CatalogFamily::WorkspaceScope;
        let witness = crate::trie::verify_member_in_transaction(
            write,
            family,
            workspace_scope_catalog_path(&reference, &key)?,
            &key,
        )?;
        if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
            return Err(error::corruption(
                "parent workspace scope disagrees with its authenticated catalog",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let indexed = roots
                    .get(reference.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("parent lineage root is missing from its root index")
                    })?;
                if indexed.value() != reference.scope().as_str() {
                    return Err(error::corruption(
                        "parent lineage root disagrees with its root index",
                    ));
                }
                current = None;
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("parent lineage run root has a parent"));
            }
            (_, Some(next)) => current = Some(next.clone()),
            (_, None) => {
                return Err(error::corruption(
                    "parent lineage non-root scope has no parent",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn require_ancestor(
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    candidate: &ScopeReference,
    leaf: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = leaf.clone();
    for _ in 0..MAX_SCOPE_DEPTH {
        if &current == candidate {
            return Ok(());
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace scope lineage is incomplete"))?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        let Some(parent) = scope.parent() else {
            break;
        };
        current = parent.clone();
    }
    Err(PersistenceError::InvalidDocument(format!(
        "scope {candidate:?} is not an ancestor of {leaf:?}"
    )))
}

pub(crate) fn workspace_value_key(
    reference: &WorkspaceValueReference,
) -> Result<Vec<u8>, PersistenceError> {
    codec::value(
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
        reference.version().get(),
    )
}

pub(crate) fn validated_workspace_domain(
    read: &redb::ReadTransaction,
    run: &RunId,
) -> Result<Option<WorkspaceUsage>, PersistenceError> {
    crate::trie::validate_roots(read)?;
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        budgets
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let usages = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        usages
            .get(run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    let family = crate::trie::CatalogFamily::WorkspaceDomain;
    let witness = crate::trie::verify_member(
        read,
        family,
        workspace_domain_path(run),
        run.as_str().as_bytes(),
    )?;
    match (budget, usage, witness) {
        (None, None, None) => Ok(None),
        (Some(budget), Some(usage), Some(witness)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            if witness != workspace_domain_payload(&budget, usage)? {
                return Err(error::corruption(
                    "workspace domain disagrees with its authenticated catalog",
                ));
            }
            Ok(Some(usage))
        }
        _ => Err(error::corruption(
            "workspace budget, usage, and authenticated domain are incomplete",
        )),
    }
}

impl WorkspaceStore for RedbStore {
    fn workspace_usage(&self, run: &RunId) -> Result<WorkspaceUsage, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let usage = validated_workspace_domain(&read, run)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let membership = validate_run_history_membership(&read, run, head)?;
        match usage {
            Some(usage) if head == RunSequence::ZERO && membership.is_none() => Ok(usage),
            Some(usage) if membership.is_some() => Ok(usage),
            Some(_) => Err(error::corruption(
                "workspace usage belongs to an unauthenticated run aggregate",
            )),
            None if head == RunSequence::ZERO && membership.is_none() => Ok(WorkspaceUsage::EMPTY),
            None => Err(error::corruption(
                "an existing run is missing its durable workspace usage",
            )),
        }
    }

    fn scope(
        &self,
        run: &RunId,
        scope: &ScopeId,
    ) -> Result<Option<WorkspaceScope>, PersistenceError> {
        let key = codec::pair(run.as_str(), scope.as_str())?;
        let reference = ScopeReference::new(run.clone(), scope.clone());
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let _membership = validate_run_history_membership(&read, run, head)?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            if crate::trie::verify_member(
                &read,
                family,
                workspace_scope_catalog_path(&reference, &key)?,
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "workspace scope catalog names a missing document",
                ));
            }
            if roots
                .get(run.as_str())
                .map_err(error::redb)?
                .is_some_and(|root| root.value() == scope.as_str())
            {
                return Err(error::corruption(
                    "root-scope index points to a missing workspace scope",
                ));
            }
            return Ok(None);
        };
        if validated_workspace_domain(&read, run)?.is_none() {
            return Err(error::corruption(
                "stored workspace scope has no accounting domain",
            ));
        }
        let stored = {
            let stored: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if stored.reference().run() != run || stored.reference().scope() != scope {
                return Err(error::corruption(
                    "workspace-scope key does not match its document",
                ));
            }
            validate_scope_catalog_lineage(&read, &table, &roots, stored.reference())?;
            stored
        };
        Ok(Some(stored))
    }

    fn value(
        &self,
        reference: &WorkspaceValueReference,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let key = workspace_value_key(reference)?;
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, reference.scope().run())?;
        let _membership = validate_run_history_membership(&read, reference.scope().run(), head)?;
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let scopes = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        validate_scope_catalog_lineage(&read, &scopes, &roots, reference.scope())?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
            let family = crate::trie::CatalogFamily::WorkspaceValue;
            if crate::trie::verify_member(
                &read,
                family,
                crate::trie::hashed_path(family, &key),
                &key,
            )?
            .is_some()
            {
                return Err(error::corruption(
                    "workspace value catalog names a missing document",
                ));
            }
            return Ok(None);
        };
        if validated_workspace_domain(&read, reference.scope().run())?.is_none() {
            return Err(error::corruption(
                "stored workspace value has no accounting domain",
            ));
        }
        let stored = {
            let stored: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
            if stored.reference() != reference {
                return Err(error::corruption(
                    "workspace-value key does not match its document",
                ));
            }
            validate_workspace_value_provenance(&table, &scopes, &roots, &stored, false)?;
            validate_scope_catalog_lineage(&read, &scopes, &roots, stored.reference().scope())?;
            validate_workspace_value_catalog_provenance(&read, &table, &scopes, &roots, &stored)?;
            stored
        };
        Ok(Some(stored))
    }

    fn latest_value(
        &self,
        scope: &ScopeReference,
        key: &ValueKey,
    ) -> Result<Option<WorkspaceValueEntry>, PersistenceError> {
        let head_key =
            codec::value_prefix(scope.run().as_str(), scope.scope().as_str(), key.as_str())?;
        let read = self.database().begin_read().map_err(error::redb)?;
        if validated_workspace_domain(&read, scope.run())?.is_none() {
            return Err(error::corruption(
                "workspace value lookup has no accounting domain",
            ));
        }
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let scopes = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        validate_scope_catalog_lineage(&read, &scopes, &roots, scope)?;
        let head = {
            let heads = read
                .open_table(WORKSPACE_VALUE_HEADS)
                .map_err(error::redb)?;
            heads
                .get(head_key.as_slice())
                .map_err(error::redb)?
                .map(|bytes| bytes.value().to_vec())
        };
        let head_family = crate::trie::CatalogFamily::WorkspaceValueHead;
        let witness = crate::trie::verify_member(
            &read,
            head_family,
            crate::trie::hashed_path(head_family, &head_key),
            &head_key,
        )?;
        let Some(head) = head else {
            if witness.is_some() {
                return Err(error::corruption(
                    "workspace value-head catalog names a missing document",
                ));
            }
            let end = codec::prefix_end(head_key.clone()).ok_or_else(|| {
                error::corruption("workspace value prefix has no exclusive range end")
            })?;
            if table
                .range(head_key.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "workspace values exist without an authenticated latest-value head",
                ));
            }
            return Ok(None);
        };
        if witness != Some(crate::trie::digest_payload(head_family, &head)) {
            return Err(error::corruption(
                "workspace value head disagrees with its authenticated catalog",
            ));
        }
        let reference: WorkspaceValueReference = json::decode(&head, "workspace value head")?;
        if reference.scope() != scope || reference.key() != key {
            return Err(error::corruption(
                "workspace value-head key disagrees with its document",
            ));
        }
        let stored_key = workspace_value_key(&reference)?;
        let bytes = table
            .get(stored_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace value head names a missing value"))?;
        let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
        if entry.reference().scope() != scope || entry.reference().key() != key {
            return Err(error::corruption(
                "workspace latest-value range contains a mismatched document",
            ));
        }
        if stored_key != workspace_value_key(entry.reference())? {
            return Err(error::corruption(
                "workspace-value key does not match its document",
            ));
        }
        validate_workspace_value_provenance(&table, &scopes, &roots, &entry, false)?;
        validate_workspace_value_catalog_provenance(&read, &table, &scopes, &roots, &entry)?;
        Ok(Some(entry))
    }

    fn scope_lineage(
        &self,
        leaf: &ScopeReference,
    ) -> Result<Vec<WorkspaceScope>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        require_run_history_membership(&read, leaf.run())?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        let mut current = leaf.clone();
        let mut reversed = Vec::new();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_SCOPE_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(error::corruption(
                    "workspace scope lineage contains a cycle",
                ));
            }
            let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
            let bytes = table.get(key.as_slice()).map_err(error::redb)?;
            let Some(bytes) = bytes else {
                let family = crate::trie::CatalogFamily::WorkspaceScope;
                if crate::trie::verify_member(
                    &read,
                    family,
                    workspace_scope_catalog_path(&current, &key)?,
                    &key,
                )?
                .is_some()
                {
                    return Err(error::corruption(
                        "workspace scope lineage catalog names a missing document",
                    ));
                }
                return Err(PersistenceError::NotFound {
                    entity: "workspace_scope",
                    identity: format!("{}/{}", current.run(), current.scope()),
                });
            };
            let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if reversed.is_empty() && validated_workspace_domain(&read, leaf.run())?.is_none() {
                return Err(error::corruption(
                    "workspace scope lineage has no accounting domain",
                ));
            }
            if scope.reference() != &current {
                return Err(error::corruption(
                    "workspace-scope key does not match its document",
                ));
            }
            let family = crate::trie::CatalogFamily::WorkspaceScope;
            let witness = crate::trie::verify_member(
                &read,
                family,
                workspace_scope_catalog_path(&current, &key)?,
                &key,
            )?;
            if witness != Some(crate::trie::digest_payload(family, bytes.value())) {
                return Err(error::corruption(
                    "workspace scope disagrees with its authenticated catalog",
                ));
            }
            let parent = scope.parent().cloned();
            reversed.push(scope);
            match parent {
                Some(parent) => current = parent,
                None => {
                    let root = reversed.last().ok_or_else(|| {
                        error::corruption("workspace scope lineage unexpectedly became empty")
                    })?;
                    let indexed_root = roots
                        .get(root.reference().run().as_str())
                        .map_err(error::redb)?
                        .ok_or_else(|| {
                            error::corruption("run-root scope is missing from its root index")
                        })?;
                    if indexed_root.value() != root.reference().scope().as_str() {
                        return Err(error::corruption(
                            "run-root scope disagrees with its root index",
                        ));
                    }
                    reversed.reverse();
                    milkdrift_workspace::ScopeLineage::new(reversed.clone()).map_err(|cause| {
                        error::corruption(format!(
                            "stored workspace scope lineage failed validation: {cause}"
                        ))
                    })?;
                    return Ok(reversed);
                }
            }
        }
        Err(error::corruption(format!(
            "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
        )))
    }
}

pub(crate) fn validate_owning_workspace_scope(
    scopes: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    roots: &impl redb::ReadableTable<&'static str, &'static str>,
    reference: &ScopeReference,
) -> Result<(), PersistenceError> {
    let mut current = reference.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_SCOPE_DEPTH {
        if !seen.insert(current.clone()) {
            return Err(error::corruption(
                "workspace scope lineage contains a cycle",
            ));
        }
        let key = codec::pair(current.run().as_str(), current.scope().as_str())?;
        let bytes = scopes
            .get(key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("workspace scope lineage is incomplete"))?;
        let scope: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
        if scope.reference() != &current {
            return Err(error::corruption(
                "workspace-scope key does not match its document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (ScopeKind::RunRoot, None) => {
                let root = roots
                    .get(current.run().as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run-root scope is missing from its root index")
                    })?;
                if root.value() != current.scope().as_str() {
                    return Err(error::corruption(
                        "run-root scope disagrees with its root index",
                    ));
                }
                return Ok(());
            }
            (ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => current = parent.clone(),
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
    }
    Err(error::corruption(format!(
        "workspace scope lineage exceeds {MAX_SCOPE_DEPTH} entries"
    )))
}
