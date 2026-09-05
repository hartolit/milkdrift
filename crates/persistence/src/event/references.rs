mod continuation;
mod execution;
mod lifecycle;
mod reconciliation;
mod structured;

use milkdrift_workspace::{
    ArtifactId, ArtifactReference, CausalReference, ContentDigest as ArtifactContentDigest,
    MediaType, RunId, WorkspaceScope, WorkspaceValueReference,
};

use crate::{EvidenceReference, PersistenceError, bounded::MAX_EVIDENCE_REFERENCES};

use super::{MAX_REFERENCES_PER_EVENT, kind::RunEventKind};

impl RunEventKind {
    /// Derives every complete content-addressed artifact reference retained by this fact.
    ///
    /// The atomic journal uses this as the sole event-side ownership source. Executor
    /// requests may carry provider-neutral artifact references, but durable history
    /// requires their media type and exact size so verification and workspace accounting
    /// cannot be bypassed by a direct blueprint artifact binding.
    pub fn required_artifacts(&self) -> Result<Vec<ArtifactReference>, PersistenceError> {
        match self {
            Self::RunTerminal { artifacts, .. } => Ok(artifacts.clone()),
            Self::NodeScheduled { request, .. } => {
                let mut references = request
                    .inputs()
                    .iter()
                    .filter_map(|input| input.value().artifact())
                    .map(workspace_artifact_reference)
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(manifest) = request.context_manifest() {
                    references.push(workspace_artifact_reference(manifest)?);
                }
                Ok(references)
            }
            Self::NodeOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::DeterministicOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::ArtifactPublished { metadata } => {
                let mut references = vec![metadata.reference().clone()];
                for causal in std::iter::once(metadata.provenance().producer())
                    .chain(metadata.provenance().causes())
                {
                    if let CausalReference::Artifact { reference } = causal {
                        references.push(reference.clone());
                    }
                }
                Ok(references)
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) fn validate_for_run(&self, run: &RunId) -> Result<(), PersistenceError> {
        if matches!(
            self,
            Self::NodeProgressRecorded {
                report_sequence: 0,
                ..
            } | Self::NodeOutputPublished {
                report_sequence: 0,
                ..
            } | Self::NodeTerminal {
                report_sequence: 0,
                ..
            } | Self::ExternalOutcomeUncertain {
                report_sequence: 0,
                ..
            } | Self::LateTerminalEvidenceRecorded {
                report_sequence: 0,
                ..
            }
        ) {
            return Err(PersistenceError::InvalidDocument(
                "executor report sequences are one-based".to_owned(),
            ));
        }
        let context = ReferenceContext { run };
        lifecycle::validate(self, &context)?;
        continuation::validate(self, &context)?;
        execution::validate(self, &context)?;
        structured::validate(self, &context)?;
        reconciliation::validate(self, &context)?;
        // Validate provider-neutral artifact references even before a commit request
        // derives its exact ownership/accounting set.
        let _ = self.required_artifacts()?;
        context.validate_workspace_run(self)?;
        Ok(())
    }
}

fn workspace_artifact_reference(
    reference: &milkdrift_capability::ArtifactReference,
) -> Result<ArtifactReference, PersistenceError> {
    let media_type = reference.media_type().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact media type".to_owned(),
        )
    })?;
    let size_bytes = reference.size_bytes().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact byte size".to_owned(),
        )
    })?;
    let artifact = ArtifactId::new(reference.identity()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid identity: {error}"
        ))
    })?;
    let digest = ArtifactContentDigest::from_hex(reference.digest()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid digest: {error}"
        ))
    })?;
    let media_type = MediaType::new(media_type).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid media type: {error}"
        ))
    })?;
    Ok(ArtifactReference::new(
        artifact, digest, media_type, size_bytes,
    ))
}

struct ReferenceContext<'run> {
    run: &'run RunId,
}

impl ReferenceContext<'_> {
    fn check_references(
        &self,
        location: &'static str,
        count: usize,
    ) -> Result<(), PersistenceError> {
        if count > MAX_REFERENCES_PER_EVENT {
            Err(PersistenceError::Bounds {
                location,
                reason: format!("at most {MAX_REFERENCES_PER_EVENT} references are allowed"),
            })
        } else {
            Ok(())
        }
    }
    fn check_evidence(&self, evidence: &[EvidenceReference]) -> Result<(), PersistenceError> {
        if evidence.len() > MAX_EVIDENCE_REFERENCES {
            Err(PersistenceError::Bounds {
                location: "event.evidence",
                reason: format!("at most {MAX_EVIDENCE_REFERENCES} references are allowed"),
            })
        } else if evidence
            .iter()
            .map(|item| &item.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != evidence.len()
        {
            Err(PersistenceError::InvalidDocument(
                "event evidence identities must be distinct".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    fn validate_workspace_run(&self, event: &RunEventKind) -> Result<(), PersistenceError> {
        let run = self.run;
        let value_in_run = |value: &WorkspaceValueReference| value.scope().run() == run;
        let scope_in_run = |scope: &WorkspaceScope| scope.reference().run() == run;
        let valid = match event {
            RunEventKind::RunCreated {
                root_scope, inputs, ..
            } => {
                scope_in_run(root_scope)
                    && root_scope.kind().is_run_root()
                    && inputs.iter().all(value_in_run)
            }
            RunEventKind::RunTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            RunEventKind::NodeBecameEligible { scope, .. } => scope.run() == run,
            RunEventKind::NodeOutputPublished { value, .. } => value_in_run(value),
            RunEventKind::DeterministicOutputPublished { value, .. } => value_in_run(value),
            RunEventKind::BranchScopeCreated { scope, .. }
            | RunEventKind::RepeatIterationCreated { scope, .. } => scope_in_run(scope),
            RunEventKind::BranchTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            RunEventKind::JoinSatisfied { branches, .. } => branches
                .iter()
                .all(|branch| branch.scope.run() == run && branch.outputs.iter().all(value_in_run)),
            RunEventKind::SubworkflowCreated {
                child_run,
                scope,
                inputs,
                ..
            } => child_run != run && scope_in_run(scope) && inputs.iter().all(value_in_run),
            RunEventKind::SubworkflowTerminal {
                child_run, outputs, ..
            } => child_run != run && outputs.iter().all(|value| value.scope().run() == child_run),
            RunEventKind::SubworkflowOutputImported {
                child_value,
                parent_value,
                ..
            } => {
                child_value.scope().run() != run
                    && parent_value.scope().run() == run
                    && child_value.scope().run() != parent_value.scope().run()
            }
            RunEventKind::SubworkflowCancellationRequested { child_run, .. } => child_run != run,
            RunEventKind::ReconciliationRemediationCreated { scope, .. }
            | RunEventKind::RemediationWorkCreated { scope, .. } => scope.run() == run,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(PersistenceError::InvalidDocument(
                "workspace scopes/value references in an event must belong to its run aggregate"
                    .to_owned(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{AttemptId, NodeExecutionId};
    use milkdrift_blueprint::NodeId;
    use milkdrift_capability::{
        ArtifactReference as CapabilityArtifactReference, CapabilityId, InputReference,
        InvocationId, InvocationRequest, InvocationValueReference, OperationId,
    };

    use super::*;

    fn scheduled_with_artifact(
        media_type: Option<String>,
        size_bytes: Option<u64>,
    ) -> Result<RunEventKind, Box<dyn std::error::Error>> {
        let invocation = InvocationId::new("invocation-artifact")?;
        let input = InputReference::new(
            "source",
            InvocationValueReference::Artifact {
                reference: CapabilityArtifactReference::new(
                    "artifact-source",
                    "a".repeat(64),
                    media_type,
                    size_bytes,
                )?,
            },
        )?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("artifact-consumer")?,
            OperationId::new("artifact.consume")?,
            None,
            None,
            vec![input],
            BTreeMap::new(),
        )?;
        Ok(RunEventKind::NodeScheduled {
            node: NodeId::new("consume")?,
            execution: NodeExecutionId::new("execution-consume")?,
            attempt: AttemptId::new("attempt-consume")?,
            invocation,
            idempotency_key: None,
            request,
        })
    }

    #[test]
    fn scheduled_artifact_inputs_are_exact_atomic_ownership_requirements()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts =
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), Some(7))?
                .required_artifacts()?;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact().as_str(), "artifact-source");
        assert_eq!(artifacts[0].digest().to_hex(), "a".repeat(64));
        assert_eq!(
            artifacts[0].media_type().as_str(),
            "application/octet-stream"
        );
        assert_eq!(artifacts[0].size_bytes(), 7);

        assert!(
            scheduled_with_artifact(None, Some(7))?
                .required_artifacts()
                .is_err()
        );
        assert!(
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), None)?
                .required_artifacts()
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn scheduled_context_manifest_is_an_exact_atomic_ownership_requirement()
    -> Result<(), Box<dyn std::error::Error>> {
        let invocation = InvocationId::new("invocation-context")?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("model-provider")?,
            OperationId::new("model.generate")?,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?
        .with_context_manifest(CapabilityArtifactReference::new(
            "artifact-context",
            "b".repeat(64),
            Some("application/vnd.milkdrift.context-manifest.v2+json".to_owned()),
            Some(42),
        )?)?;
        let event = RunEventKind::NodeScheduled {
            node: NodeId::new("generate")?,
            execution: NodeExecutionId::new("execution-generate")?,
            attempt: AttemptId::new("attempt-generate")?,
            invocation,
            idempotency_key: None,
            request,
        };

        let artifacts = event.required_artifacts()?;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact().as_str(), "artifact-context");
        assert_eq!(artifacts[0].digest().to_hex(), "b".repeat(64));
        assert_eq!(
            artifacts[0].media_type().as_str(),
            "application/vnd.milkdrift.context-manifest.v2+json"
        );
        assert_eq!(artifacts[0].size_bytes(), 42);
        Ok(())
    }
}
