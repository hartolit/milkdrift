//! One accounting key space for local workspaces and independently served invocations.
use crate::error;
use milkdrift_persistence::PersistenceError;
use milkdrift_workspace::{ArtifactOwner, RunId};

pub(crate) fn domain_key(owner: &ArtifactOwner) -> Result<String, PersistenceError> {
    match owner {
        ArtifactOwner::Workflow { run } => Ok(run.to_string()),
        ArtifactOwner::HostInvocation { .. }
        | ArtifactOwner::Transfer { .. }
        | ArtifactOwner::PeerInput { .. }
        | milkdrift_workspace::ArtifactOwner::ClientInput { .. } => serde_json::to_string(owner)
            .map_err(|cause| error::corruption(format!("artifact owner encoding failed: {cause}"))),
    }
}

pub(crate) fn domain_owner(key: &str) -> Result<ArtifactOwner, PersistenceError> {
    let owner = if key.starts_with('{') {
        serde_json::from_str(key)
            .map_err(|cause| error::corruption(format!("invalid artifact owner: {cause}")))?
    } else {
        ArtifactOwner::Workflow {
            run: RunId::new(key).map_err(|cause| {
                error::corruption(format!("invalid artifact workflow owner: {cause}"))
            })?,
        }
    };
    if domain_key(&owner)? != key {
        return Err(error::corruption("noncanonical artifact owner key"));
    }
    Ok(owner)
}
