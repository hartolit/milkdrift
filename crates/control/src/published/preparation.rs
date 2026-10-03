//! Resolve immutable identities without registering a callable implementation.
use super::{PublishedWorkflowService, publication_error, rejected};
use milkdrift_authority::GrantId;
use milkdrift_blueprint::RevisionId;
use milkdrift_capability::CapabilityDescriptor;
use milkdrift_persistence::{
    ControllerResourceBudget,
    published::{
        PUBLISHED_METHOD_SCHEMA_VERSION, PublishedInput, PublishedMethod, PublishedOutput,
    },
};
use milkdrift_workspace::WorkspaceBudget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Reviewed publication choices. The owner supplies the revision's agreement and the configured
/// service's immutable grant facts; every contract and resource ceiling remains explicit.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationDraft {
    /// Public capability, generation, operation contract and admission limits.
    pub descriptor: CapabilityDescriptor,
    /// Purpose, input meaning and result limitations.
    pub documentation: String,
    /// Exact saved governed method.
    pub revision: RevisionId,
    /// Explicit service grant; must match the operator-configured capability relationship.
    pub service_grant: GrantId,
    /// Reviewed finite input rules.
    pub inputs: BTreeMap<String, PublishedInput>,
    /// Declared accepted fields permitted to leave the private run.
    pub outputs: BTreeMap<String, PublishedOutput>,
    /// Internal writable-state limits.
    pub workspace_budget: WorkspaceBudget,
    /// Cumulative internal execution allowance.
    pub allowance: ControllerResourceBudget,
    /// Outstanding calls permitted for this generation.
    pub maximum_outstanding: u32,
    /// Inclusive publication ancestry ceiling.
    pub maximum_depth: u16,
    /// Elapsed call limit; expiry requests cancellation.
    pub maximum_duration_ms: u64,
}

impl PublishedWorkflowService {
    /// Prepare the ordinary publication document without publishing or granting authority.
    /// The application must authorize access to the revision and method administration first.
    pub fn prepare_publication(
        &self,
        draft: PublicationDraft,
    ) -> Result<PublishedMethod, crate::ControlError> {
        let service = self
            .services
            .get(draft.descriptor.identity())
            .filter(|service| service.grant == draft.service_grant)
            .ok_or_else(|| {
                publication_error(rejected(
                    "publication requires the exact configured service grant",
                ))
            })?;
        let revision = self
            .store
            .revision(&draft.revision)?
            .ok_or_else(|| publication_error(rejected("starting method is absent")))?;
        let agreement = revision.semantic().agreement().ok_or_else(|| {
            publication_error(rejected("published method requires a governing agreement"))
        })?;
        let method = PublishedMethod {
            schema_version: PUBLISHED_METHOD_SCHEMA_VERSION,
            descriptor: draft.descriptor,
            documentation: draft.documentation,
            revision: draft.revision,
            agreement: agreement.digest().to_owned(),
            service: service.clone(),
            inputs: draft.inputs,
            outputs: draft.outputs,
            workspace_budget: draft.workspace_budget,
            allowance: draft.allowance,
            maximum_outstanding: draft.maximum_outstanding,
            maximum_depth: draft.maximum_depth,
            maximum_duration_ms: draft.maximum_duration_ms,
        };
        self.validate_method(&method, &self.host)
            .map_err(publication_error)?;
        Ok(method)
    }
}
