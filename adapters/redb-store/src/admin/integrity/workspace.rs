use super::super::*;
use super::{ScanContext, phase};

pub(super) fn scan_core(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let scopes = read.open_table(SCOPES).map_err(error::redb)?;
    let root_scopes = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
    let values = read.open_table(VALUES).map_err(error::redb)?;
    let usage = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;

    context.binary_bytes(phase::SCOPES, &scopes, "workspace_indexes", |key, bytes| {
        let scope: WorkspaceScope = json::decode(bytes, "workspace scope")?;
        let expected = codec::pair(
            scope.reference().run().as_str(),
            scope.reference().scope().as_str(),
        )?;
        if key != expected.as_slice() {
            return Err(error::corruption(
                "workspace-scope key does not match its checked document",
            ));
        }
        match (scope.kind(), scope.parent()) {
            (&ScopeKind::RunRoot, None) => match root_scopes
                .get(scope.reference().run().as_str())
                .map_err(error::redb)?
            {
                Some(root) if root.value() == scope.reference().scope().as_str() => {}
                _ => {
                    return Err(error::corruption(
                        "run-root scope is missing or mismatched in its root index",
                    ));
                }
            },
            (&ScopeKind::RunRoot, Some(_)) => {
                return Err(error::corruption("run-root scope has a parent"));
            }
            (_, Some(parent)) => {
                crate::journal::validate_owning_workspace_scope(&scopes, &root_scopes, parent)?;
            }
            (_, None) => {
                return Err(error::corruption("non-root workspace scope has no parent"));
            }
        }
        Ok(())
    })?;
    context.binary_bytes(phase::VALUES, &values, "workspace_indexes", |key, bytes| {
        let entry: WorkspaceValueEntry = json::decode(bytes, "workspace value")?;
        let expected = crate::journal::workspace_value_key(entry.reference())?;
        if key != expected.as_slice() {
            return Err(error::corruption(
                "workspace-value key does not match its checked document",
            ));
        }
        crate::journal::validate_workspace_value_provenance(
            &values,
            &scopes,
            &root_scopes,
            &entry,
            false,
        )
    })?;
    context.string_bytes(phase::USAGE, &usage, "workspace_indexes", |key, bytes| {
        let usage: WorkspaceUsage = json::decode(bytes, "workspace usage")?;
        let run = RunId::new(key).map_err(|cause| {
            error::corruption(format!("invalid workspace-usage run identity: {cause}"))
        })?;
        if budgets.get(key).map_err(error::redb)?.is_none() {
            return Err(error::corruption(
                "workspace usage is missing its immutable budget",
            ));
        }
        let stored_usage = crate::journal::validated_workspace_domain(read, &run)?
            .ok_or_else(|| error::corruption("workspace usage has no budget pair"))?;
        if stored_usage != usage {
            return Err(error::corruption(
                "workspace usage disagrees with its durable domain",
            ));
        }
        Ok(())
    })?;
    context.string_bytes(
        phase::BUDGET,
        &budgets,
        "workspace_indexes",
        |key, bytes| {
            let _budget: WorkspaceBudget = json::decode(bytes, "workspace budget")?;
            let run = RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid workspace-budget run identity: {cause}"))
            })?;
            if usage.get(key).map_err(error::redb)?.is_none() {
                return Err(error::corruption(
                    "workspace budget is missing its usage accounting",
                ));
            }
            if crate::journal::validated_workspace_domain(read, &run)?.is_none() {
                return Err(error::corruption(
                    "workspace budget has no durable usage pair",
                ));
            }
            Ok(())
        },
    )?;
    context.string_string(
        phase::ROOT_SCOPES,
        &root_scopes,
        "workspace_indexes",
        |run, scope| {
            let run = RunId::new(run).map_err(|cause| {
                error::corruption(format!("invalid root-scope run identity: {cause}"))
            })?;
            let scope_id = ScopeId::new(scope).map_err(|cause| {
                error::corruption(format!("invalid root-scope identity: {cause}"))
            })?;
            let key = codec::pair(run.as_str(), scope_id.as_str())?;
            let bytes = scopes
                .get(key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("root-scope index pointer is dangling"))?;
            let document: WorkspaceScope = json::decode(bytes.value(), "workspace scope")?;
            if document.reference().run() != &run
                || document.reference().scope() != &scope_id
                || document.kind() != &ScopeKind::RunRoot
                || document.parent().is_some()
            {
                return Err(error::corruption(
                    "root-scope index does not match its checked root document",
                ));
            }
            if crate::journal::validated_workspace_domain(read, &run)?.is_none() {
                return Err(error::corruption(
                    "root workspace scope has no durable workspace domain",
                ));
            }
            Ok(())
        },
    )
}

pub(super) fn scan_value_heads(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let value_heads = read
        .open_table(WORKSPACE_VALUE_HEADS)
        .map_err(error::redb)?;
    let values = read.open_table(VALUES).map_err(error::redb)?;
    context.binary_bytes(
        phase::WORKSPACE_VALUE_HEADS,
        &value_heads,
        "workspace_indexes",
        |key, bytes| {
            let reference: WorkspaceValueReference = json::decode(bytes, "workspace value head")?;
            let expected = codec::value_prefix(
                reference.scope().run().as_str(),
                reference.scope().scope().as_str(),
                reference.key().as_str(),
            )?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "workspace value-head key does not match its document",
                ));
            }
            let value_key = crate::journal::workspace_value_key(&reference)?;
            let value = values
                .get(value_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("workspace value head names a missing value"))?;
            let value: WorkspaceValueEntry = json::decode(value.value(), "workspace value")?;
            if value.reference() != &reference {
                return Err(error::corruption(
                    "workspace value head disagrees with its authoritative value",
                ));
            }
            Ok(())
        },
    )
}
