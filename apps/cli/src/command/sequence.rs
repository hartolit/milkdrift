//! Compile sequence inputs through their owner and submit ordinary daemon commands.
//!
//! Stage inspection uses the saved revision's mapping. Remediation reads the current run and
//! actor before building a proposal, so rerunning the same CLI arguments after a lost reply can
//! produce a different guarded document; inspect durable proposal state before proceeding.

use milkdrift_control_protocol::Command;
use milkdrift_prompt_sequence::{
    PromptSource, RemediationProposalSpec, build_remediation_proposal,
};
use serde_json::json;

use crate::{SequenceCommand, error::CliError, session::CliSession};

pub(super) async fn execute(
    session: &CliSession,
    command: &SequenceCommand,
) -> Result<(), CliError> {
    match command {
        SequenceCommand::Compile { .. } => Err(CliError::Internal(
            "local compilation must precede connection".to_owned(),
        )),
        SequenceCommand::Validate { file } => {
            let document = session.read_prompt_sequence(file).await?;
            let request = session.command_request(Command::ValidatePromptSequence { document })?;
            session.output(
                "sequence.validate",
                &session.client().submit(&request).await?,
            )
        }
        SequenceCommand::Import { file } => {
            let document = session.read_prompt_sequence(file).await?;
            let request = session.command_request(Command::ImportPromptSequence { document })?;
            session.output("sequence.import", &session.client().submit(&request).await?)
        }
        SequenceCommand::Show { revision } => {
            let mut read = session.client().revision(revision).await?;
            read.document = None;
            session.output("sequence.show", &read)
        }
        SequenceCommand::Status { run, revision } => {
            let mut revision = session.client().revision(revision).await?;
            revision.document = None;
            let run = session.client().run(run).await?;
            session.output(
                "sequence.status",
                &json!({"schema_version": 1, "revision": revision, "run": run}),
            )
        }
        SequenceCommand::Stage { run, stage } => show_stage(session, run, stage).await,
        SequenceCommand::Remediate {
            sequence_file,
            run,
            revision,
            stage,
            generation,
            proposal,
            prompt,
        } => {
            if sequence_file == std::path::Path::new("-") && prompt == std::path::Path::new("-") {
                return Err(CliError::Invalid(
                    "sequence and remediation prompt cannot both consume stdin".to_owned(),
                ));
            }
            remediate(
                session,
                RemediationArguments {
                    sequence_file,
                    run,
                    revision,
                    stage,
                    generation: *generation,
                    proposal,
                    prompt,
                },
            )
            .await
        }
    }
}

async fn show_stage(session: &CliSession, run: &str, stage: &str) -> Result<(), CliError> {
    let state = session.client().run(run).await?;
    let revision_id = state
        .revision_id
        .as_deref()
        .ok_or_else(|| CliError::Internal("run has no current revision".to_owned()))?;
    let revision_read = session.client().revision(revision_id).await?;
    let document = revision_read
        .document
        .ok_or_else(|| CliError::Internal("revision document is unavailable".to_owned()))?;
    let bytes =
        serde_json::to_vec(&document).map_err(|error| CliError::Internal(error.to_string()))?;
    let node_ids = milkdrift_prompt_sequence::stage_node_ids(&bytes, stage)
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    let nodes = state
        .nodes
        .iter()
        .filter(|node| node_ids.iter().any(|identity| identity == &node.node_id))
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return Err(CliError::NotFound(
            "stage has no current node occurrences".to_owned(),
        ));
    }
    session.output(
        "sequence.stage",
        &json!({"schema_version": 1, "run_id": run, "stage_id": stage, "nodes": nodes}),
    )
}

struct RemediationArguments<'a> {
    sequence_file: &'a std::path::Path,
    run: &'a str,
    revision: &'a str,
    stage: &'a str,
    generation: u16,
    proposal: &'a str,
    prompt: &'a std::path::Path,
}

async fn remediate(
    session: &CliSession,
    arguments: RemediationArguments<'_>,
) -> Result<(), CliError> {
    let sequence = session
        .read_prompt_sequence_document(arguments.sequence_file)
        .await?;
    let state = session.client().run(arguments.run).await?;
    let revision_read = session.client().revision(arguments.revision).await?;
    let revision_value = revision_read
        .document
        .ok_or_else(|| CliError::Internal("revision document is unavailable".to_owned()))?;
    let revision_bytes = serde_json::to_vec(&revision_value)
        .map_err(|error| CliError::Internal(error.to_string()))?;
    let prompt = session.read_remediation_prompt(arguments.prompt).await?;
    let authority = session.client().authority().await?;
    let proposal_document = build_remediation_proposal(
        &sequence,
        &revision_bytes,
        RemediationProposalSpec {
            run: arguments.run.to_owned(),
            observed_sequence: state.sequence,
            proposal: arguments.proposal.to_owned(),
            proposer: authority.actor,
            stage_id: arguments.stage.to_owned(),
            generation: arguments.generation,
            prompt: PromptSource::InlineMarkdown { content: prompt },
            verification_override: None,
        },
    )
    .map_err(|error| CliError::Invalid(error.to_string()))?;
    let proposal_value = serde_json::from_slice(
        &proposal_document
            .to_canonical_json()
            .map_err(|error| CliError::Invalid(error.to_string()))?,
    )
    .map_err(|error| CliError::Internal(error.to_string()))?;
    let request = session.command_request_with_revision(
        Command::SubmitProposal {
            document: proposal_value,
        },
        arguments.revision,
    )?;
    session.output(
        "sequence.remediate",
        &session.client().submit(&request).await?,
    )
}
