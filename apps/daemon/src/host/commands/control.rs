//! One control-document envelope and result path shared by command families.

use super::super::{
    ActorSession, ControlCommand, ControlCommandDocument, ControlResult, OptimisticGuard, Owner,
    ProposalDigest, PublicFailure, Reason, RunQueryStore, RunSequence, TimestampMillis, evidence,
    internal, internal_control_id, parse_revision_id, public_control, public_persistence,
};

impl Owner {
    pub(in crate::host) fn execute_control(
        &self,
        session: &ActorSession,
        request: &milkdrift_control_protocol::CommandRequest,
        expected_sequence: Option<u64>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            None,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            _ => Err(internal()),
        }
    }

    pub(in crate::host) fn execute_control_guarded(
        &self,
        session: &ActorSession,
        request: &milkdrift_control_protocol::CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<u64, PublicFailure> {
        let run = match &command {
            ControlCommand::ApproveProposal { run, .. }
            | ControlCommand::RejectProposal { run, .. }
            | ControlCommand::ApplyProposal { run, .. } => run.clone(),
            _ => return Err(internal()),
        };
        let result = self.execute_control_result(
            session,
            request,
            expected_sequence,
            proposal_digest,
            command,
            suffix,
        )?;
        match result {
            ControlResult::RuntimeCommand { resulting_sequence } => Ok(resulting_sequence.get()),
            ControlResult::ProposalStatus { .. } => self
                .store
                .run_summary(&run)
                .map_err(public_persistence)?
                .map(|summary| summary.through_sequence.get())
                .ok_or_else(super::super::not_found),
            _ => Err(internal()),
        }
    }

    pub(in crate::host) fn execute_control_result(
        &self,
        session: &ActorSession,
        request: &milkdrift_control_protocol::CommandRequest,
        expected_sequence: Option<u64>,
        proposal_digest: Option<ProposalDigest>,
        command: ControlCommand,
        suffix: &str,
    ) -> Result<ControlResult, PublicFailure> {
        let document = ControlCommandDocument::new(
            internal_control_id(session, request, suffix)?,
            session.context.clone(),
            TimestampMillis::new(self.now()?),
            OptimisticGuard {
                expected_run_sequence: expected_sequence.map(RunSequence::new),
                expected_revision: request
                    .expected_revision
                    .as_deref()
                    .map(parse_revision_id)
                    .transpose()?,
                expected_proposal_digest: proposal_digest,
            },
            Reason::new(request.reason.clone()).map_err(public_persistence)?,
            evidence(request)?,
            command,
        )
        .map_err(public_control)?;
        self.control.execute(&document).map_err(public_control)
    }
}
