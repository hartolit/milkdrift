//! Immutable artifact-owner budgets and exact cumulative logical usage.
use crate::{
    error, json,
    schema::{WORKSPACE_BUDGETS, WORKSPACE_USAGE},
};
use milkdrift_persistence::PersistenceError;
use milkdrift_workspace::WorkspaceUsage;
use redb::ReadableTable;
pub(crate) fn workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
) -> Result<Option<(milkdrift_workspace::WorkspaceBudget, WorkspaceUsage)>, PersistenceError> {
    let owner = owner.into();
    let key = crate::artifact::owner::domain_key(&owner)?;
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let table = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        table
            .get(key.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        table
            .get(key.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    match (budget, usage) {
        (None, None) => Ok(None),
        (Some(budget), Some(usage)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            Ok(Some((budget, usage)))
        }
        _ => Err(error::corruption(
            "workspace budget and usage records are incomplete",
        )),
    }
}

pub(crate) fn persist_workspace_value_usage_accounting_in_transaction(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    let owner = owner.into();
    let Some((_budget, stored_usage)) = workspace_domain_in_transaction(write, &owner)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if stored_usage != usage {
        return Err(error::corruption(
            "workspace usage argument disagrees with its durable document",
        ));
    }
    Ok(())
}

pub(crate) fn validate_workspace_domain_in_transaction(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    let owner = owner.into();
    let Some((budget, usage)) = workspace_domain_in_transaction(write, &owner)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if &budget != supplied {
        return Err(PersistenceError::ImmutableConflict {
            entity: "workspace_budget",
            identity: crate::artifact::owner::domain_key(&owner)?,
        });
    }
    budget.validate_usage(&usage).map_err(|cause| {
        error::corruption(format!(
            "workspace usage exceeds its durable budget: {cause}"
        ))
    })?;
    Ok(usage)
}

pub(crate) fn validate_or_initialize_workspace_domain(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    supplied: &milkdrift_workspace::WorkspaceBudget,
) -> Result<WorkspaceUsage, PersistenceError> {
    let owner = owner.into();
    let key = crate::artifact::owner::domain_key(&owner)?;
    match workspace_domain_in_transaction(write, &owner)? {
        None => {
            supplied
                .validate_usage(&WorkspaceUsage::EMPTY)
                .map_err(|cause| {
                    PersistenceError::InvalidDocument(format!(
                        "empty workspace usage violates the supplied budget: {cause}"
                    ))
                })?;
            let budget_bytes = json::encode(supplied, "workspace budget")?;
            let usage_bytes = json::encode(&WorkspaceUsage::EMPTY, "workspace usage")?;
            write
                .open_table(WORKSPACE_BUDGETS)
                .map_err(error::redb)?
                .insert(key.as_str(), budget_bytes.as_slice())
                .map_err(error::redb)?;
            write
                .open_table(WORKSPACE_USAGE)
                .map_err(error::redb)?
                .insert(key.as_str(), usage_bytes.as_slice())
                .map_err(error::redb)?;
            Ok(WorkspaceUsage::EMPTY)
        }
        Some((_budget, _usage)) => {
            validate_workspace_domain_in_transaction(write, &owner, supplied)
        }
    }
}

pub(crate) fn advance_workspace_global_usage_in_transaction(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    expected: WorkspaceUsage,
    resulting: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    let owner = owner.into();
    let Some((budget, actual)) = workspace_domain_in_transaction(write, &owner)? else {
        return Err(error::corruption("workspace accounting domain is absent"));
    };
    if actual != expected {
        return Err(error::corruption(
            "workspace usage changed before artifact accounting advance",
        ));
    }
    budget.validate_usage(&resulting).map_err(|cause| {
        PersistenceError::InvalidDocument(format!("workspace usage exceeds budget: {cause}"))
    })?;
    Ok(())
}
pub(crate) fn validated_workspace_domain(
    read: &redb::ReadTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
) -> Result<Option<WorkspaceUsage>, PersistenceError> {
    let owner = owner.into();
    let key = crate::artifact::owner::domain_key(&owner)?;
    let budget: Option<milkdrift_workspace::WorkspaceBudget> = {
        let budgets = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
        budgets
            .get(key.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace budget"))
            .transpose()?
    };
    let usage = {
        let usages = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        usages
            .get(key.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode(bytes.value(), "workspace usage"))
            .transpose()?
    };
    match (budget, usage) {
        (None, None) => Ok(None),
        (Some(budget), Some(usage)) => {
            budget.validate_usage(&usage).map_err(|cause| {
                error::corruption(format!("workspace usage exceeds its budget: {cause}"))
            })?;
            Ok(Some(usage))
        }
        _ => Err(error::corruption(
            "workspace budget and usage records are incomplete",
        )),
    }
}
