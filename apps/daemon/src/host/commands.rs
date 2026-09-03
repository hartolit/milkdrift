//! Authorized external command adaptation into focused domain command owners.

use super::{
    ActorSession, Command, CommandAccepted, CommandRequest, Owner, PublicFailure, layouts,
};

mod control;
mod controllers;
mod definitions;
mod proposals;
mod runs;

impl Owner {
    #[expect(
        clippy::too_many_lines,
        reason = "this exhaustive match is the single routing map from the public command protocol to focused owners"
    )]
    pub(super) fn execute_new_command(
        &mut self,
        session: &ActorSession,
        request: &CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        match &request.command {
            Command::ImportBlueprint { document } => {
                definitions::blueprint(self, session, request, document, true)
            }
            Command::ValidateBlueprint { document } => {
                definitions::blueprint(self, session, request, document, false)
            }
            Command::ImportPromptSequence { document } => {
                definitions::prompt_sequence(self, session, request, document, true)
            }
            Command::ValidatePromptSequence { document } => {
                definitions::prompt_sequence(self, session, request, document, false)
            }
            Command::StartRun {
                run_id,
                workflow_id,
                revision_id,
            } => runs::start(self, session, request, run_id, workflow_id, revision_id),
            Command::PauseRun { run_id } => runs::pause(self, session, request, run_id),
            Command::ResumeRun { run_id } => runs::resume(self, session, request, run_id),
            Command::CancelRun { run_id } => runs::cancel(self, session, request, run_id),
            Command::SignalRun {
                run_id,
                signal_id,
                signal_type,
                correlation,
                broadcast,
                payload,
            } => runs::signal(
                self,
                session,
                request,
                runs::SignalArguments {
                    run_id,
                    signal_id,
                    signal_type,
                    correlation: correlation.as_deref(),
                    broadcast: *broadcast,
                    payload,
                },
            ),
            Command::ResolveWork {
                run_id,
                attempt_id,
                decision_id,
                action,
                remediation_node,
            } => runs::resolve(
                self,
                session,
                request,
                runs::ResolveArguments {
                    run_id,
                    attempt_id,
                    decision_id,
                    action: *action,
                    remediation_node: remediation_node.as_deref(),
                },
            ),
            Command::InspectController {
                run_id,
                controller_execution,
            } => controllers::inspect(self, session, request, run_id, controller_execution),
            Command::ContinueController {
                run_id,
                controller_execution,
                decision_id,
            } => controllers::continue_checkpoint(
                self,
                session,
                request,
                run_id,
                controller_execution,
                decision_id,
            ),
            Command::SubmitProposal { document } => {
                proposals::submit(self, session, request, document)
            }
            Command::DecideProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
                decision_id,
                decision,
            } => proposals::decide(
                self,
                session,
                request,
                proposals::DecisionArguments {
                    run_id,
                    proposal_id,
                    proposal_digest,
                    proposed_revision,
                    decision_id,
                    decision: *decision,
                },
            ),
            Command::ApplyProposal {
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
            } => proposals::apply(
                self,
                session,
                request,
                run_id,
                proposal_id,
                proposal_digest,
                proposed_revision,
            ),
            Command::PutLayout { layout } => layouts::execute(self, session, request, layout),
        }
    }
}
