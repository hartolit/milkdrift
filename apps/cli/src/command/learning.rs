//! Transport named evaluation requests; eligibility and provenance remain server-owned.
use crate::{
    error::CliError,
    learning_args::{LearningArgs, LearningCommand},
    session::CliSession,
};
use milkdrift_control_protocol::{Command, MAX_DOCUMENT_BYTES};
use serde_json::{Value, json};
use std::path::Path;

fn receipt(parts: &[String]) -> Result<Value, CliError> {
    match parts {
        [] => Ok(Value::Null),
        [actor, command] => Ok(json!({"actor":actor,"command":command})),
        _ => Err(CliError::Invalid(
            "receipt requires exactly ACTOR COMMAND".into(),
        )),
    }
}

async fn read(session: &CliSession, path: &Path) -> Result<Value, CliError> {
    session
        .read_json(path, MAX_DOCUMENT_BYTES, "evaluation document")
        .await
}

pub(super) async fn execute(session: &CliSession, args: &LearningArgs) -> Result<(), CliError> {
    let document = match (&args.file, &args.command) {
        (Some(file), None) => read(session, file).await?,
        (None, Some(command)) => match command {
            LearningCommand::Select {
                method,
                workspace,
                guidance,
                artifact,
                page,
                supersedes,
                approval,
            } => {
                let mut pages = Vec::new();
                for parts in page.chunks(3) {
                    let [run, first, count] = parts else {
                        return Err(CliError::Invalid("page requires RUN FIRST COUNT".into()));
                    };
                    let first: u64 = first.parse().map_err(|_| {
                        CliError::Invalid("page FIRST must be an unsigned integer".into())
                    })?;
                    let count: u32 = count.parse().map_err(|_| {
                        CliError::Invalid("page COUNT must be an unsigned integer".into())
                    })?;
                    pages.push(json!({"run":run,"first":first,"count":count}));
                }
                json!({"type":"select_sources","selection":{"method":method,"workspace":workspace,"guidance":guidance,"artifacts":artifact,"pages":pages,"supersedes":receipt(supersedes)?,"approval":receipt(approval)?}})
            }
            LearningCommand::Declare { file } => {
                json!({"type":"declare","declaration":read(session, file).await?})
            }
            LearningCommand::Candidate {
                proposal,
                declaration,
                expected_benefit,
                applicability,
                counterevidence,
            } => {
                json!({"type":"candidate","declaration":receipt(declaration)?,"proposal":read(session, proposal).await?,"expected_benefit":expected_benefit,"applicability":applicability,"counterevidence":counterevidence})
            }
            LearningCommand::Compare {
                declaration,
                candidate,
            } => {
                json!({"type":"compare","declaration":receipt(declaration)?,"candidate":receipt(candidate)?})
            }
            LearningCommand::Inspect { actor, command } => {
                json!({"type":"inspect","receipt":{"actor":actor,"command":command}})
            }
            LearningCommand::Promote {
                method,
                comparison,
                expected_previous_version,
            } => {
                json!({"type":"promote","comparison":receipt(comparison)?,"method":read(session, method).await?,"expected_previous_version":expected_previous_version})
            }
        },
        _ => {
            return Err(CliError::Invalid(
                "choose an evaluation command or one advanced request file".into(),
            ));
        }
    };
    let inspected = (document["type"] == "inspect").then(|| document["receipt"].clone());
    let request = session.command_request(Command::Learning { document })?;
    let actor = session.client().authority().await?.actor;
    let accepted = session.client().submit(&request).await?;
    let kind = accepted.result_type.clone();
    let mut value =
        serde_json::to_value(accepted).map_err(|error| CliError::Internal(error.to_string()))?;
    value["receipt"] =
        inspected.unwrap_or_else(|| json!({"actor":actor,"command":request.command_id}));
    session.output(&kind, &value)
}
