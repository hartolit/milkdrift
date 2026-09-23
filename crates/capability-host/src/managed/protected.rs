use super::{ManagedError, ManagedResources, rejected};
use milkdrift_authority::{AuthorityOperation, AuthorityRequest, BoundaryTimeMillis};
use milkdrift_capability::{
    BoundedJson,
    managed::{ManagedAction, ManagedRequest, ManagedResponse},
};
use milkdrift_persistence::{
    ArtifactReadAuthority, ArtifactReadRequest, EvidenceId,
    managed::{ApprovedSetup, InstallationRecord},
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, CandidateCheck, CandidateEvaluation, CandidateSubject,
};

impl ManagedResources {
    pub(super) fn read_candidate(
        &self,
        caller: &AuthorityRequest,
        reference: &milkdrift_capability::ArtifactReference,
        maximum: u64,
    ) -> Result<(ArtifactReference, Vec<u8>), ManagedError> {
        let store = self
            .artifacts
            .as_ref()
            .ok_or_else(|| rejected("candidate artifact reader unavailable"))?;
        let id = ArtifactId::new(reference.identity()).map_err(rejected)?;
        let metadata = store
            .metadata(&id)?
            .ok_or_else(|| rejected("candidate artifact absent"))?;
        let durable = metadata.reference();
        if reference.digest() != durable.digest().to_string()
            || reference.size_bytes() != Some(durable.size_bytes())
            || reference.media_type() != Some(durable.media_type().as_str())
            || durable.size_bytes() > maximum
        {
            return Err(rejected(
                "candidate reference differs from immutable metadata or exceeds policy",
            ));
        }
        let mut authority = caller.clone();
        authority.operation = AuthorityOperation::ReadArtifactContent;
        authority.resources.artifact = Some(id);
        authority.resources.artifact_sensitivity = Some(metadata.sensitivity());
        authority.evaluated_at = BoundaryTimeMillis::new(self.clock.now().map_err(rejected)?.get());
        if !self
            .authority
            .evaluate(&authority)
            .map_err(rejected)?
            .is_allowed()
        {
            return Err(ManagedError::Unauthorized);
        }
        let mut bytes = Vec::new();
        while bytes.len() as u64 != durable.size_bytes() {
            let maximum = (durable.size_bytes() - bytes.len() as u64).min(262_144) as u32;
            let chunk = store.read_chunk(&ArtifactReadRequest::new(
                durable.clone(),
                bytes.len() as u64,
                maximum,
                ArtifactReadAuthority::Authorized {
                    actor: caller.actor.clone(),
                    evidence: EvidenceId::new(caller.decision.as_str()).map_err(rejected)?,
                },
            )?)?;
            if chunk.offset != bytes.len() as u64
                || chunk.bytes.is_empty()
                || chunk.bytes.len() > maximum as usize
            {
                return Err(rejected(
                    "candidate artifact reader made no bounded progress",
                ));
            }
            bytes.extend_from_slice(&chunk.bytes);
        }
        if !durable.verifies(&bytes) {
            return Err(rejected("candidate bytes differ from immutable reference"));
        }
        Ok((durable.clone(), bytes))
    }

    pub(super) fn evaluate_candidate(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
        record: &InstallationRecord,
        setup: &ApprovedSetup,
    ) -> Result<ManagedResponse, ManagedError> {
        let ManagedAction::Evaluate { candidate } = &request.action else {
            return Err(rejected("evaluation action required"));
        };
        let protected = setup
            .protection
            .as_ref()
            .ok_or_else(|| rejected("target is not protected"))?;
        self.platform.check_protected_policy(setup)?;
        let (artifact, bytes) =
            self.read_candidate(caller, candidate, protected.policy.maximum_candidate_bytes)?;
        let authorization = self.authorize(caller, request, setup)?;
        let now = self.clock.now().map_err(rejected)?.get();
        let mut evaluation = CandidateEvaluation {
            schema_version: 1,
            identity: super::digest(
                &serde_json::to_string(&(caller.actor.clone(), request)).map_err(rejected)?,
            ),
            subject: CandidateSubject {
                artifact,
                target: record.name.clone(),
                generation: record.generation,
                agreement: protected.agreement.clone(),
                configuration: setup.recipe.digest.clone(),
                policy: protected.policy.digest().map_err(rejected)?,
                verifier: protected.policy.verifier.clone(),
                producer: protected.policy.producer.clone(),
            },
            started_at: now,
            expires_at: now
                .checked_add(protected.policy.validity_ms)
                .ok_or_else(|| rejected("evaluation time overflow"))?,
            complete: false,
            checks: protected
                .policy
                .required_checks
                .iter()
                .map(|name| CandidateCheck {
                    name: name.clone(),
                    passed: None,
                    diagnostic: "evaluation has not completed".to_owned(),
                })
                .collect(),
        };
        let receipt = self
            .store
            .begin_managed_evaluation(request, &authorization, &evaluation)?;
        // No transport can supply this report. Only the configured platform verifier is called,
        // after its exact candidate and incomplete outcome are durably bound.
        match self.platform.evaluate_candidate(setup, &evaluation, &bytes) {
            Ok(checks)
                if checks
                    .iter()
                    .map(|c| &c.name)
                    .eq(protected.policy.required_checks.iter()) =>
            {
                evaluation.checks = checks
            }
            Ok(_) => {
                for check in &mut evaluation.checks {
                    check.diagnostic = "trusted verifier returned a different check set".to_owned();
                }
            }
            Err(_) => {
                for check in &mut evaluation.checks {
                    check.diagnostic =
                        "trusted verifier did not complete; inspect the retained target".to_owned();
                }
            }
        }
        evaluation.complete = true;
        self.store.finish_managed_evaluation(&evaluation)?;
        Ok(receipt.response)
    }

    pub(super) fn evidence_view(
        &self,
        request: &ManagedRequest,
        record: &InstallationRecord,
        identity: &str,
    ) -> Result<ManagedResponse, ManagedError> {
        let evidence = self
            .store
            .managed_evaluation(identity)?
            .ok_or_else(|| rejected("trusted evaluation absent"))?;
        if evidence.subject.target != request.installation {
            return Err(rejected("evaluation names another target"));
        }
        let mut response = record.view();
        response.state = if evidence.complete {
            "evaluated"
        } else {
            "evaluating"
        }
        .to_owned();
        response.evaluation = Some(
            BoundedJson::new(serde_json::to_value(evidence).map_err(rejected)?)
                .map_err(rejected)?,
        );
        Ok(response)
    }

    pub(super) fn publication_candidate(
        &self,
        caller: &AuthorityRequest,
        request: &ManagedRequest,
        record: &InstallationRecord,
        setup: &ApprovedSetup,
        identity: &str,
    ) -> Result<ApprovedSetup, ManagedError> {
        let protected = setup
            .protection
            .as_ref()
            .ok_or_else(|| rejected("target is not protected"))?;
        self.platform.check_protected_policy(setup)?;
        let evidence = self
            .store
            .managed_evaluation(identity)?
            .ok_or_else(|| rejected("trusted evaluation absent"))?;
        protected
            .policy
            .require_pass(&evidence, self.clock.now().map_err(rejected)?.get())
            .map_err(rejected)?;
        if evidence.subject.target != request.installation
            || evidence.subject.generation != record.generation
            || evidence.subject.agreement != protected.agreement
            || evidence.subject.configuration != setup.recipe.digest
        {
            return Err(rejected(
                "evaluation candidate, agreement, target or configuration differs",
            ));
        }
        let artifact = &evidence.subject.artifact;
        let reference = milkdrift_capability::ArtifactReference::new(
            artifact.artifact().as_str(),
            artifact.digest().to_string(),
            Some(artifact.media_type().to_string()),
            Some(artifact.size_bytes()),
        )
        .map_err(rejected)?;
        let (_, bytes) =
            self.read_candidate(caller, &reference, protected.policy.maximum_candidate_bytes)?;
        self.platform.prepare_publication(
            setup,
            &evidence,
            &bytes,
            record
                .generation
                .checked_add(1)
                .ok_or_else(|| rejected("target generation exhausted"))?,
        )
    }
}
