//! Workspace read port verifies exact history, provenance and lineage in one read transaction.
use super::{
    super::BTreeSet, super::MAX_SCOPE_DEPTH, super::PersistenceError, super::ROOT_SCOPES,
    super::RUN_EVENTS, super::RUN_HEADS, super::RedbStore, super::RunId, super::RunSequence,
    super::SCOPES, super::ScopeId, super::ScopeReference, super::VALUES, super::ValueKey,
    super::WORKSPACE_VALUE_HEADS, super::WorkspaceScope, super::WorkspaceStore,
    super::WorkspaceUsage, super::WorkspaceValueEntry, super::WorkspaceValueReference,
    super::codec, super::error, super::json, super::queries::validated_run_head,
    super::validate_run_history_membership, require_run_history_membership, validate_scope_lineage,
    validate_workspace_value_provenance, validate_workspace_value_storage_provenance,
    validated_workspace_domain, workspace_value_key,
};

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
                "workspace usage belongs to an absent run aggregate",
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
        let read = self.database().begin_read().map_err(error::redb)?;
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, run)?;
        let _membership = validate_run_history_membership(&read, run, head)?;
        let table = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
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
            validate_scope_lineage(&read, &table, &roots, stored.reference())?;
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
        let heads = read.open_table(RUN_HEADS).map_err(error::redb)?;
        let events = read.open_table(RUN_EVENTS).map_err(error::redb)?;
        let head = validated_run_head(&heads, &events, reference.scope().run())?;
        let _membership = validate_run_history_membership(&read, reference.scope().run(), head)?;
        let table = read.open_table(VALUES).map_err(error::redb)?;
        let scopes = read.open_table(SCOPES).map_err(error::redb)?;
        let roots = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
        validate_scope_lineage(&read, &scopes, &roots, reference.scope())?;
        let stored = table.get(key.as_slice()).map_err(error::redb)?;
        let Some(bytes) = stored else {
            return Ok(None);
        };
        let value_head_key = codec::value_prefix(
            reference.scope().run().as_str(),
            reference.scope().scope().as_str(),
            reference.key().as_str(),
        )?;
        let value_heads = read
            .open_table(WORKSPACE_VALUE_HEADS)
            .map_err(error::redb)?;
        let head_bytes = value_heads
            .get(value_head_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption("workspace value exists without a latest-value head")
            })?;
        let latest: WorkspaceValueReference =
            json::decode(head_bytes.value(), "workspace value head")?;
        if latest.scope() != reference.scope()
            || latest.key() != reference.key()
            || latest.version() < reference.version()
        {
            return Err(error::corruption(
                "workspace value lies beyond or disagrees with its latest-value head",
            ));
        }
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
            validate_scope_lineage(&read, &scopes, &roots, stored.reference().scope())?;
            validate_workspace_value_storage_provenance(&read, &table, &scopes, &roots, &stored)?;
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
        validate_scope_lineage(&read, &scopes, &roots, scope)?;
        let head = {
            let heads = read
                .open_table(WORKSPACE_VALUE_HEADS)
                .map_err(error::redb)?;
            heads
                .get(head_key.as_slice())
                .map_err(error::redb)?
                .map(|bytes| bytes.value().to_vec())
        };
        let Some(head) = head else {
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
                    "workspace values exist without a latest-value head",
                ));
            }
            return Ok(None);
        };
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
        validate_workspace_value_storage_provenance(&read, &table, &scopes, &roots, &entry)?;
        Ok(Some(entry))
    }

    fn scope_lineage(
        &self,
        leaf: &ScopeReference,
    ) -> Result<Vec<WorkspaceScope>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
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
