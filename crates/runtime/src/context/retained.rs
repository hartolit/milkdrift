//! Validate whether saved selection evidence can authorize another execution without
//! reconstructing candidates or changing the historical manifest.

use milkdrift_blueprint::TaskContextPolicy;
use milkdrift_model::{ContextManifest, ContextOmissionReason};

use super::{ContextBuildError, ContextBuildIdentity};

pub(crate) fn validate_retained_manifest(
    manifest: &ContextManifest,
    identity: &ContextBuildIdentity,
    policy: &TaskContextPolicy,
) -> Result<(), ContextBuildError> {
    if manifest.run() != &identity.run
        || manifest.revision() != &identity.revision
        || manifest.node() != &identity.node
        || manifest.execution() != &identity.execution
        || manifest.attempt() != &identity.attempt
    {
        return Err(ContextBuildError::RequiredUnavailable(
            "retained manifest contradicts its exact attempt",
        ));
    }
    if !matches!(
        manifest.policy_version(),
        1 | super::CONTEXT_SELECTION_POLICY_VERSION
    ) || manifest.policy_digest()
        != &policy
            .digest()
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?
        || manifest.budget() != policy.budget()
    {
        return Err(ContextBuildError::RequiredUnavailable(
            "retained manifest policy contradicts its governing task",
        ));
    }
    for omission in manifest.omissions() {
        if policy.fail_closed()
            && omission.required
            && omission.reason == ContextOmissionReason::SelectionStopped
        {
            return Err(ContextBuildError::RequiredUnavailable(
                "retained manifest stopped before required evidence",
            ));
        }
        // Old writers could mask isolation or denied authority with these reasons.
        // Omissions retain no independent scope/authority proof, so source-bearing
        // instances cannot safely be forwarded again. Policy version 2 is emitted only
        // by the corrected selector; version 1 remains readable as historical evidence.
        if manifest.policy_version() == 1
            && (omission.source.is_some()
                || omission.omitted_bytes != 0
                || omission.omitted_artifact_bytes != 0)
            && matches!(
                omission.reason,
                ContextOmissionReason::SelectionStopped
                    | ContextOmissionReason::ExcludedCategory
                    | ContextOmissionReason::NotSelected
                    | ContextOmissionReason::MissingOrCorrupt
                    | ContextOmissionReason::Unsupported
                    | ContextOmissionReason::Superseded
                    | ContextOmissionReason::AuthorityDenied
                    | ContextOmissionReason::BranchIsolated
            )
        {
            return Err(ContextBuildError::RequiredUnavailable(
                "retained omission lacks independent disclosure evidence",
            ));
        }
    }
    Ok(())
}
