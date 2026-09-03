//! Deterministic context ranking, bounded selection, and final manifest construction.

use milkdrift_blueprint::{ContextTruncation, NodeId};
use milkdrift_model::{
    ContextInclusionReason, ContextManifest, ContextManifestEntry, ContextOmission,
    ContextOmissionReason, ContextSemanticKind, ContextTotals, ModelContractError,
};

use super::{
    ContextBuildError, ContextBuildRequest, ContextCandidate, ContextCandidateAvailability,
    ancestor_depths, budget_overflow, candidate_is_artifact, eligibility, exact_source_requested,
    omission,
};

type RankedCandidate = (
    u16,
    ContextSemanticKind,
    Option<NodeId>,
    Vec<u8>,
    bool,
    Option<ContextInclusionReason>,
    Option<ContextOmissionReason>,
    ContextCandidate,
);

pub(super) fn build(
    request: ContextBuildRequest<'_>,
) -> Result<ContextManifest, ContextBuildError> {
    SelectionState::new(request)?.select()?.finish()
}

struct SelectionState<'a> {
    request: ContextBuildRequest<'a>,
    ranked: Vec<RankedCandidate>,
    entries: Vec<ContextManifestEntry>,
    omissions: Vec<ContextOmission>,
    totals: ContextTotals,
    stopped: bool,
}

impl<'a> SelectionState<'a> {
    fn new(mut request: ContextBuildRequest<'a>) -> Result<Self, ContextBuildError> {
        validate_nodes(&request)?;
        let ancestors = ancestor_depths(
            request.semantic,
            &request.identity.node,
            request.policy.ancestor_depth(),
        );
        let mut ranked = Vec::with_capacity(request.candidates.len());
        for candidate in std::mem::take(&mut request.candidates) {
            let (eligible, reason, omitted) = eligibility(
                request.policy,
                &request.visible_scopes,
                &ancestors,
                &candidate,
            );
            let depth = candidate.causal_distance.unwrap_or_else(|| {
                candidate
                    .node
                    .as_ref()
                    .and_then(|node| ancestors.get(node).copied())
                    .unwrap_or(u16::MAX)
            });
            let source_key = candidate
                .source
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|error| ModelContractError::Invalid(error.to_string()))?
                .unwrap_or_default();
            ranked.push((
                depth,
                candidate.kind,
                candidate.node.clone(),
                source_key,
                eligible,
                reason,
                omitted,
                candidate,
            ));
        }
        ranked.sort_by(|left, right| {
            (&left.0, &left.1, &left.2, &left.3).cmp(&(&right.0, &right.1, &right.2, &right.3))
        });
        Ok(Self {
            request,
            ranked,
            entries: Vec::new(),
            omissions: Vec::new(),
            totals: ContextTotals::default(),
            stopped: false,
        })
    }

    fn select(mut self) -> Result<Self, ContextBuildError> {
        for (_, _, _, _, eligible, reason, omitted, candidate) in std::mem::take(&mut self.ranked) {
            self.consider(candidate, eligible, reason, omitted)?;
        }
        Ok(self)
    }

    fn consider(
        &mut self,
        candidate: ContextCandidate,
        eligible: bool,
        inclusion_reason: Option<ContextInclusionReason>,
        omission_reason: Option<ContextOmissionReason>,
    ) -> Result<(), ContextBuildError> {
        if !eligible || self.stopped {
            if !eligible
                && candidate.required
                && self.request.policy.fail_closed()
                && exact_source_requested(self.request.policy, &candidate)
            {
                return Err(ContextBuildError::RequiredUnavailable(
                    "exact context source is excluded or not visible",
                ));
            }
            let reason = if self.stopped {
                ContextOmissionReason::SelectionStopped
            } else {
                omission_reason.unwrap_or(ContextOmissionReason::NotSelected)
            };
            self.omissions.push(omission(&candidate, reason));
            return Ok(());
        }
        if candidate.availability != ContextCandidateAvailability::Available {
            return self.omit_unavailable(&candidate);
        }
        if candidate.authority.required && !candidate.authority.authorized {
            if candidate.required && self.request.policy.fail_closed() {
                return Err(ContextBuildError::AuthorityDenied);
            }
            self.omissions
                .push(omission(&candidate, ContextOmissionReason::AuthorityDenied));
            return Ok(());
        }
        if let Some((reason, budget_name)) =
            budget_overflow(self.request.policy.budget(), self.totals, &candidate)?
        {
            if candidate.required && self.request.policy.fail_closed() {
                return Err(ContextBuildError::RequiredBudget(budget_name));
            }
            self.omissions.push(omission(&candidate, reason));
            self.stopped =
                self.request.policy.truncation() == ContextTruncation::StopAtFirstOverflow;
            return Ok(());
        }
        self.include(candidate, inclusion_reason)
    }

    fn omit_unavailable(&mut self, candidate: &ContextCandidate) -> Result<(), ContextBuildError> {
        if candidate.required && self.request.policy.fail_closed() {
            return Err(ContextBuildError::RequiredUnavailable(
                "missing exact source",
            ));
        }
        let reason = match candidate.availability {
            ContextCandidateAvailability::Available => unreachable!(),
            ContextCandidateAvailability::MissingOrCorrupt => {
                ContextOmissionReason::MissingOrCorrupt
            }
            ContextCandidateAvailability::Unsupported => ContextOmissionReason::Unsupported,
            ContextCandidateAvailability::Superseded => ContextOmissionReason::Superseded,
        };
        self.omissions.push(omission(candidate, reason));
        Ok(())
    }

    fn include(
        &mut self,
        candidate: ContextCandidate,
        reason: Option<ContextInclusionReason>,
    ) -> Result<(), ContextBuildError> {
        self.totals.items = self
            .totals
            .items
            .checked_add(1)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        self.totals.bytes = self
            .totals
            .bytes
            .checked_add(candidate.selected_bytes)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        self.totals.artifact_bytes = self
            .totals
            .artifact_bytes
            .checked_add(candidate.selected_artifact_bytes)
            .ok_or(ContextBuildError::AccountingOverflow)?;
        let selected_artifact = candidate_is_artifact(&candidate);
        if selected_artifact {
            self.totals.artifacts = self
                .totals
                .artifacts
                .checked_add(1)
                .ok_or(ContextBuildError::AccountingOverflow)?;
        }
        if let Some(units) = candidate.estimated_model_input_units {
            self.totals.model_input_units = Some(
                self.totals
                    .model_input_units
                    .unwrap_or(0)
                    .checked_add(units)
                    .ok_or(ContextBuildError::AccountingOverflow)?,
            );
        }
        let missing_source = if candidate.required {
            "missing source reference"
        } else {
            "eligible source reference"
        };
        let source = candidate
            .source
            .ok_or(ContextBuildError::RequiredUnavailable(missing_source))?;
        self.entries.push(ContextManifestEntry::new(
            self.totals.items,
            candidate.kind,
            candidate.roles,
            source,
            candidate.content_digest,
            candidate.source_revision,
            candidate.execution,
            candidate.attempt,
            candidate.scope,
            candidate.causal_distance,
            candidate.source_sequence,
            candidate.occurred_at_ms,
            candidate.producer,
            candidate.causal_parents,
            selected_artifact,
            candidate.selected_bytes,
            candidate.selected_artifact_bytes,
            candidate.estimated_model_input_units,
            candidate.sensitivity,
            candidate.authority,
            reason.unwrap_or(ContextInclusionReason::IncludedCategory),
        )?);
        Ok(())
    }

    fn finish(self) -> Result<ContextManifest, ContextBuildError> {
        let budget = self.request.policy.budget();
        let digest = self
            .request
            .policy
            .digest()
            .map_err(|error| ContextBuildError::Policy(error.to_string()))?;
        let identity = self.request.identity;
        let manifest = ContextManifest::new(
            identity.run,
            identity.revision,
            identity.node,
            identity.execution,
            identity.attempt,
            1,
            digest,
            self.entries,
            self.omissions,
            self.totals,
            budget,
        )?;
        let encoded =
            milkdrift_model::ContextManifestDocument::new(manifest.clone()).to_canonical_json()?;
        if u64::try_from(encoded.len()).map_or(true, |size| size > budget.max_manifest_bytes) {
            return Err(ContextBuildError::RequiredBudget(
                "serialized manifest byte",
            ));
        }
        Ok(manifest)
    }
}

fn validate_nodes(request: &ContextBuildRequest<'_>) -> Result<(), ContextBuildError> {
    if !request
        .semantic
        .nodes()
        .contains_key(&request.identity.node)
    {
        return Err(ContextBuildError::MissingNode(
            request.identity.node.clone(),
        ));
    }
    for node in request.policy.selected_nodes() {
        if !request.semantic.nodes().contains_key(node) {
            return Err(ContextBuildError::MissingNode(node.clone()));
        }
    }
    Ok(())
}
