//! Thin operator CLI over `milkdrift-control-client`.

use std::{
    env, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Args, Parser, Subcommand};
use futures_util::StreamExt as _;
use milkdrift_control_client::{
    BearerCredential, ClientConfig, ClientError, ControlClient, status_class,
};
use milkdrift_control_protocol::{
    Command, CommandRequest, Cursor, EvidenceRef, LayoutDocument, PageRequest, ProposalDecision,
    ProtocolVersion,
};
use serde::Serialize;
use serde_json::{Value, json};
use url::Url;

const JSON_OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Parser)]
#[command(
    name = "milkdrift",
    version,
    about = "Milkdrift local daemon operator client"
)]
struct Cli {
    /// Daemon base URL.
    #[arg(
        long,
        env = "MILKDRIFT_ENDPOINT",
        default_value = "http://127.0.0.1:9734/"
    )]
    endpoint: Url,
    /// Read the bearer credential from this restricted file.
    #[arg(long, env = "MILKDRIFT_TOKEN_FILE", conflicts_with = "token_env")]
    token_file: Option<PathBuf>,
    /// Read the bearer credential from this exact environment variable name.
    #[arg(long, env = "MILKDRIFT_TOKEN_ENV", default_value = "MILKDRIFT_TOKEN")]
    token_env: String,
    /// Emit stable compact JSON without colors or control sequences.
    #[arg(long, global = true)]
    json: bool,
    /// Confirm high-risk operations and permit noninteractive execution.
    #[arg(long, global = true)]
    yes: bool,
    /// Stable command idempotency identity; generated when omitted.
    #[arg(long, global = true)]
    command_id: Option<String>,
    /// Bounded durable command reason.
    #[arg(long, global = true, default_value = "operator CLI command")]
    reason: String,
    /// Optional optimistic run-sequence guard.
    #[arg(long, global = true)]
    expected_sequence: Option<u64>,
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand)]
enum TopCommand {
    /// Daemon lifecycle observations.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Immutable blueprint and revision operations.
    Blueprint {
        #[command(subcommand)]
        command: BlueprintCommand,
    },
    /// Durable run operations.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    /// Inspect one node execution.
    Node(NodeInspect),
    /// Inspect one attempt.
    Attempt(AttemptInspect),
    /// Workflow proposal operations.
    Proposal {
        #[command(subcommand)]
        command: ProposalCommand,
    },
    /// Live capability generations.
    Capability {
        #[command(subcommand)]
        command: CapabilityCommand,
    },
    /// Authenticated remote peer lifecycle and catalog status.
    Peer {
        #[command(subcommand)]
        command: PeerCommand,
    },
    /// Artifact metadata and bounded downloads.
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
    },
    /// Presentation-only workflow layout.
    Layout {
        #[command(subcommand)]
        command: LayoutCommand,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Read liveness state.
    Health,
    /// Require readiness after recovery/adapters.
    Readiness,
}

#[derive(Subcommand)]
enum BlueprintCommand {
    /// Import one exact versioned blueprint JSON document.
    Import { file: PathBuf },
    /// Inspect one immutable revision.
    Show { revision: String },
    /// List one bounded stable revision page.
    List(PageArgs),
    /// Compare two semantic revisions.
    Diff { from: String, to: String },
}

#[derive(Args)]
struct PageArgs {
    /// Maximum returned items; no implicit auto-pagination.
    #[arg(long, default_value_t = 100)]
    limit: u32,
    /// Opaque continuation from an earlier response.
    #[arg(long)]
    cursor: Option<String>,
    /// Optional workflow filter.
    #[arg(long)]
    workflow: Option<String>,
    /// Optional run state filter.
    #[arg(long)]
    state: Option<String>,
}

#[derive(Subcommand)]
enum RunCommand {
    /// Create and start from one exact immutable revision.
    Start {
        run: String,
        workflow: String,
        revision: String,
    },
    /// List one bounded stable run page.
    List(PageArgs),
    /// Inspect compact current state.
    Show { run: String },
    /// Pause new work.
    Pause { run: String },
    /// Resume paused work.
    Resume { run: String },
    /// Request durable cancellation.
    Cancel { run: String },
    /// Deliver a typed signal.
    Signal {
        run: String,
        #[arg(long)]
        signal_id: String,
        #[arg(long)]
        signal_type: String,
        #[arg(long)]
        correlation: Option<String>,
        #[arg(long)]
        broadcast: bool,
        /// JSON signal payload.
        #[arg(long, default_value = "null")]
        payload: String,
    },
    /// Read or follow the projected timeline.
    Timeline {
        run: String,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Args)]
struct NodeInspect {
    /// Run aggregate.
    run: String,
    /// Logical node-execution identity.
    execution: String,
}

#[derive(Args)]
struct AttemptInspect {
    /// Run aggregate.
    run: String,
    /// Immutable attempt identity.
    attempt: String,
}

#[derive(Subcommand)]
enum ProposalCommand {
    /// Submit one exact versioned proposal JSON document.
    Submit { file: PathBuf },
    /// List proposal statuses known for one run.
    List {
        run: String,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Inspect one proposal/reconciliation status.
    Show {
        run: String,
        proposal: String,
        revision: String,
    },
    /// Approve an exact proposal.
    Approve(ProposalDecisionArgs),
    /// Reject an exact proposal.
    Reject(ProposalDecisionArgs),
    /// Apply an exact approved proposal.
    Apply(ProposalApplyArgs),
}

#[derive(Args)]
struct ProposalDecisionArgs {
    run: String,
    proposal: String,
    proposal_digest: String,
    proposed_revision: String,
    decision_id: String,
}

#[derive(Args)]
struct ProposalApplyArgs {
    run: String,
    proposal: String,
    proposal_digest: String,
    proposed_revision: String,
}

#[derive(Subcommand)]
enum CapabilityCommand {
    /// List visible generation health.
    List,
    /// Show all generations for one capability identity.
    Show { capability: String },
}

#[derive(Subcommand)]
enum PeerCommand {
    /// List configured peer health and catalog expiry.
    List,
    /// Show one authenticated peer relationship.
    Show { peer: String },
    /// Authenticate and refresh one remote catalog.
    Connect { peer: String },
    /// Re-authenticate and replace registrations from the current remote catalog.
    Reload { peer: String },
    /// Drain and remove one peer's local remote registrations.
    Disconnect { peer: String },
    /// Gracefully drain one peer's local remote registrations.
    Drain { peer: String },
    /// Revoke the live relationship until configuration is reloaded/restarted.
    Revoke { peer: String },
}

#[derive(Subcommand)]
enum ArtifactCommand {
    /// Read safe immutable metadata.
    Metadata { artifact: String },
    /// Download verified bounded ranges into one new explicit destination.
    Get {
        artifact: String,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum LayoutCommand {
    /// Read layout for one exact workflow/revision association.
    Get { workflow: String, revision: String },
    /// Optimistically update one versioned layout JSON document.
    Put { file: PathBuf },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("milkdrift: {error}");
            ExitCode::from(exit_code(&error))
        }
    }
}

async fn execute(cli: Cli) -> Result<(), CliError> {
    let credential = load_credential(&cli)?;
    let client = ControlClient::new(ClientConfig::new(cli.endpoint.clone()), credential)?;
    let _ = client.negotiate().await?;
    match &cli.command {
        TopCommand::Daemon { command } => match command {
            DaemonCommand::Health => output(&cli, "daemon.health", &client.health().await?)?,
            DaemonCommand::Readiness => {
                output(&cli, "daemon.readiness", &client.readiness().await?)?
            }
        },
        TopCommand::Blueprint { command } => match command {
            BlueprintCommand::Import { file } => {
                let document = read_json(file)?;
                let request = command_request(&cli, Command::ImportBlueprint { document });
                output(&cli, "blueprint.import", &client.submit(&request).await?)?;
            }
            BlueprintCommand::Show { revision } => {
                output(&cli, "blueprint.show", &client.revision(revision).await?)?;
            }
            BlueprintCommand::List(page) => {
                let request = page_request(page.limit, page.cursor.as_deref())?;
                output(
                    &cli,
                    "blueprint.list",
                    &client.revisions(page.workflow.as_deref(), &request).await?,
                )?;
            }
            BlueprintCommand::Diff { from, to } => {
                output(
                    &cli,
                    "blueprint.diff",
                    &client.revision_diff(from, to).await?,
                )?;
            }
        },
        TopCommand::Run { command } => match command {
            RunCommand::Start {
                run,
                workflow,
                revision,
            } => {
                let request = command_request(
                    &cli,
                    Command::StartRun {
                        run_id: run.clone(),
                        workflow_id: workflow.clone(),
                        revision_id: revision.clone(),
                    },
                );
                output(&cli, "run.start", &client.submit(&request).await?)?;
            }
            RunCommand::List(page) => {
                let request = page_request(page.limit, page.cursor.as_deref())?;
                output(
                    &cli,
                    "run.list",
                    &client
                        .runs(page.state.as_deref(), page.workflow.as_deref(), &request)
                        .await?,
                )?;
            }
            RunCommand::Show { run } => {
                let run = client.run(run).await?;
                output(&cli, "run.show", &run)?;
                if run.terminal.as_deref() == Some("failed") {
                    return Err(CliError::FailedTask(
                        "run reached a failed terminal outcome".to_owned(),
                    ));
                }
            }
            RunCommand::Pause { run } => {
                let request = command_request(
                    &cli,
                    Command::PauseRun {
                        run_id: run.clone(),
                    },
                );
                output(&cli, "run.pause", &client.submit(&request).await?)?;
            }
            RunCommand::Resume { run } => {
                let request = command_request(
                    &cli,
                    Command::ResumeRun {
                        run_id: run.clone(),
                    },
                );
                output(&cli, "run.resume", &client.submit(&request).await?)?;
            }
            RunCommand::Cancel { run } => {
                confirm(&cli, "request durable run cancellation")?;
                let request = command_request(
                    &cli,
                    Command::CancelRun {
                        run_id: run.clone(),
                    },
                );
                output(&cli, "run.cancel", &client.submit(&request).await?)?;
            }
            RunCommand::Signal {
                run,
                signal_id,
                signal_type,
                correlation,
                broadcast,
                payload,
            } => {
                let payload: Value = serde_json::from_str(payload)
                    .map_err(|error| CliError::Invalid(error.to_string()))?;
                let request = command_request(
                    &cli,
                    Command::SignalRun {
                        run_id: run.clone(),
                        signal_id: signal_id.clone(),
                        signal_type: signal_type.clone(),
                        correlation: correlation.clone(),
                        broadcast: *broadcast,
                        payload,
                    },
                );
                output(&cli, "run.signal", &client.submit(&request).await?)?;
            }
            RunCommand::Timeline {
                run,
                limit,
                cursor,
                follow: should_follow,
            } => {
                let request = page_request(*limit, cursor.as_deref())?;
                let page = client.timeline(run, &request).await?;
                output(&cli, "run.timeline", &page)?;
                if *should_follow {
                    follow(&cli, &client, run, page.observed_cursor.or(request.cursor)).await?;
                }
            }
        },
        TopCommand::Node(arguments) => output(
            &cli,
            "node.inspect",
            &client.node(&arguments.run, &arguments.execution).await?,
        )?,
        TopCommand::Attempt(arguments) => output(
            &cli,
            "attempt.inspect",
            &client.attempt(&arguments.run, &arguments.attempt).await?,
        )?,
        TopCommand::Proposal { command } => match command {
            ProposalCommand::Submit { file } => {
                let request = command_request(
                    &cli,
                    Command::SubmitProposal {
                        document: read_json(file)?,
                    },
                );
                output(&cli, "proposal.submit", &client.submit(&request).await?)?;
            }
            ProposalCommand::List { run, limit, cursor } => {
                let page = page_request(*limit, cursor.as_deref())?;
                output(&cli, "proposal.list", &client.proposals(run, &page).await?)?;
            }
            ProposalCommand::Show {
                run,
                proposal,
                revision,
            } => output(
                &cli,
                "proposal.show",
                &client.proposal(run, proposal, revision).await?,
            )?,
            ProposalCommand::Approve(arguments) => {
                confirm(&cli, "approve this exact workflow proposal")?;
                proposal_decision(&cli, &client, arguments, ProposalDecision::Approve).await?;
            }
            ProposalCommand::Reject(arguments) => {
                confirm(&cli, "reject this exact workflow proposal")?;
                proposal_decision(&cli, &client, arguments, ProposalDecision::Reject).await?;
            }
            ProposalCommand::Apply(arguments) => {
                confirm(&cli, "apply this exact workflow proposal")?;
                let request = command_request(
                    &cli,
                    Command::ApplyProposal {
                        run_id: arguments.run.clone(),
                        proposal_id: arguments.proposal.clone(),
                        proposal_digest: arguments.proposal_digest.clone(),
                        proposed_revision: arguments.proposed_revision.clone(),
                    },
                );
                output(&cli, "proposal.apply", &client.submit(&request).await?)?;
            }
        },
        TopCommand::Capability { command } => {
            let capabilities = client.capabilities().await?;
            match command {
                CapabilityCommand::List => output(&cli, "capability.list", &capabilities)?,
                CapabilityCommand::Show { capability } => {
                    let matches = capabilities
                        .into_iter()
                        .filter(|item| &item.capability_id == capability)
                        .collect::<Vec<_>>();
                    if matches.is_empty() {
                        return Err(CliError::NotFound("capability was not found".to_owned()));
                    }
                    output(&cli, "capability.show", &matches)?;
                }
            }
        }
        TopCommand::Peer { command } => match command {
            PeerCommand::List => output(&cli, "peer.list", &client.peers().await?)?,
            PeerCommand::Show { peer } => {
                output(&cli, "peer.show", &client.peer(peer).await?)?;
            }
            PeerCommand::Connect { peer } => {
                output(
                    &cli,
                    "peer.connect",
                    &client.peer_action(peer, "connect").await?,
                )?;
            }
            PeerCommand::Reload { peer } => {
                output(
                    &cli,
                    "peer.reload",
                    &client.peer_action(peer, "reload").await?,
                )?;
            }
            PeerCommand::Disconnect { peer } => {
                confirm(&cli, "disconnect and drain this peer")?;
                output(
                    &cli,
                    "peer.disconnect",
                    &client.peer_action(peer, "disconnect").await?,
                )?;
            }
            PeerCommand::Drain { peer } => {
                confirm(&cli, "drain this peer")?;
                output(
                    &cli,
                    "peer.drain",
                    &client.peer_action(peer, "drain").await?,
                )?;
            }
            PeerCommand::Revoke { peer } => {
                confirm(&cli, "revoke this live peer relationship")?;
                output(
                    &cli,
                    "peer.revoke",
                    &client.peer_action(peer, "revoke").await?,
                )?;
            }
        },
        TopCommand::Artifact { command } => match command {
            ArtifactCommand::Metadata { artifact } => output(
                &cli,
                "artifact.metadata",
                &client.artifact_metadata(artifact).await?,
            )?,
            ArtifactCommand::Get {
                artifact,
                output: destination,
            } => {
                download(&cli, &client, artifact, destination).await?;
            }
        },
        TopCommand::Layout { command } => match command {
            LayoutCommand::Get { workflow, revision } => {
                output(
                    &cli,
                    "layout.get",
                    &client.layout(workflow, revision).await?,
                )?;
            }
            LayoutCommand::Put { file } => {
                let bytes = fs::read(file).map_err(|error| {
                    CliError::Invalid(format!("layout read failed: {:?}", error.kind()))
                })?;
                let layout: LayoutDocument = milkdrift_control_protocol::decode_json(&bytes)
                    .map_err(|error| CliError::Invalid(error.to_string()))?;
                let request = command_request(&cli, Command::PutLayout { layout });
                output(&cli, "layout.put", &client.submit(&request).await?)?;
            }
        },
    }
    Ok(())
}

async fn proposal_decision(
    cli: &Cli,
    client: &ControlClient,
    arguments: &ProposalDecisionArgs,
    decision: ProposalDecision,
) -> Result<(), CliError> {
    let request = command_request(
        cli,
        Command::DecideProposal {
            run_id: arguments.run.clone(),
            proposal_id: arguments.proposal.clone(),
            proposal_digest: arguments.proposal_digest.clone(),
            proposed_revision: arguments.proposed_revision.clone(),
            decision_id: arguments.decision_id.clone(),
            decision,
        },
    );
    output(cli, "proposal.decide", &client.submit(&request).await?)
}

async fn follow(
    cli: &Cli,
    client: &ControlClient,
    run: &str,
    cursor: Option<Cursor>,
) -> Result<(), CliError> {
    safe_identity(run)?;
    let mut observations = client.subscribe(format!("v1/runs/{run}/stream"), cursor);
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| CliError::Internal(error.to_string()))?;
                return Ok(());
            }
            item = observations.next() => match item {
                Some(Ok(observation)) => output(cli, "run.observation", &observation)?,
                Some(Err(error)) => {
                    if cli.json {
                        println!("{}", serde_json::to_string(&json!({
                            "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
                            "type": "stream_status",
                            "status": "reconnecting",
                            "retryable": error.retryable()
                        })).map_err(|encode| CliError::Internal(encode.to_string()))?);
                    } else {
                        eprintln!("timeline stream: {error}; reconnecting when permitted");
                    }
                    if !error.retryable() {
                        return Err(error.into());
                    }
                }
                None => return Ok(()),
            }
        }
    }
}

async fn download(
    cli: &Cli,
    client: &ControlClient,
    artifact: &str,
    destination: &Path,
) -> Result<(), CliError> {
    let metadata = client.artifact_metadata(artifact).await?;
    let mut file = create_download_destination(destination)?;
    let result = async {
        let mut offset = 0_u64;
        while offset < metadata.size {
            let end = offset
                .saturating_add(1_048_576 - 1)
                .min(metadata.size.saturating_sub(1));
            let range = client.artifact_range(artifact, offset, end).await?;
            if range.bytes.is_empty() || range.start != offset {
                return Err(CliError::Internal(
                    "artifact range did not advance".to_owned(),
                ));
            }
            file.write_all(&range.bytes).map_err(|error| {
                CliError::Internal(format!("artifact write failed: {:?}", error.kind()))
            })?;
            offset = offset.saturating_add(u64::try_from(range.bytes.len()).unwrap_or(0));
        }
        file.sync_all().map_err(|error| {
            CliError::Internal(format!("artifact flush failed: {:?}", error.kind()))
        })?;
        Ok::<(), CliError>(())
    }
    .await;
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(destination);
    }
    result?;
    output(
        cli,
        "artifact.get",
        &json!({"artifact_id": metadata.artifact_id, "size": metadata.size, "destination": destination}),
    )
}

fn create_download_destination(destination: &Path) -> Result<fs::File, CliError> {
    if destination.file_name().is_none() {
        return Err(CliError::Invalid(
            "artifact destination must name a file".to_owned(),
        ));
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            CliError::Invalid(format!(
                "artifact destination must not already exist and must be writable: {:?}",
                error.kind()
            ))
        })
}

fn command_request(cli: &Cli, command: Command) -> CommandRequest {
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: cli.command_id.clone().unwrap_or_else(generated_command_id),
        expected_sequence: cli.expected_sequence,
        expected_revision: None,
        reason: cli.reason.clone(),
        evidence: Vec::<EvidenceRef>::new(),
        command,
    }
}

fn generated_command_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    format!("cli-{millis}-{}", std::process::id())
}

fn page_request(limit: u32, cursor: Option<&str>) -> Result<PageRequest, CliError> {
    let cursor = cursor
        .map(|value| serde_json::from_value(Value::String(value.to_owned())))
        .transpose()
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    let page = PageRequest { cursor, limit };
    page.validate()
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    Ok(page)
}

fn read_json(path: &Path) -> Result<Value, CliError> {
    let bytes = fs::read(path)
        .map_err(|error| CliError::Invalid(format!("JSON file read failed: {:?}", error.kind())))?;
    milkdrift_control_protocol::decode_json(&bytes)
        .map_err(|error| CliError::Invalid(error.to_string()))
}

fn load_credential(cli: &Cli) -> Result<BearerCredential, CliError> {
    let mut value = if let Some(path) = &cli.token_file {
        let metadata = fs::metadata(path).map_err(|error| {
            CliError::Invalid(format!("credential file unavailable: {:?}", error.kind()))
        })?;
        if !metadata.is_file() || metadata.len() > 4_097 {
            return Err(CliError::Invalid(
                "credential file is not a bounded regular file".to_owned(),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(CliError::Invalid(
                    "credential file must not be accessible by group or other users".to_owned(),
                ));
            }
        }
        fs::read_to_string(path).map_err(|error| {
            CliError::Invalid(format!("credential file read failed: {:?}", error.kind()))
        })?
    } else {
        env::var(&cli.token_env).map_err(|_| {
            CliError::Invalid(
                "configured credential environment reference is unavailable".to_owned(),
            )
        })?
    };
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    BearerCredential::new(value).map_err(CliError::from)
}

fn confirm(cli: &Cli, operation: &str) -> Result<(), CliError> {
    if cli.yes {
        return Ok(());
    }
    if cli.json {
        return Err(CliError::Invalid(
            "high-risk JSON-mode commands require --yes".to_owned(),
        ));
    }
    eprint!("Confirm {operation}? Type 'yes': ");
    io::stderr()
        .flush()
        .map_err(|error| CliError::Internal(error.to_string()))?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| CliError::Internal(error.to_string()))?;
    if answer.trim() == "yes" {
        Ok(())
    } else {
        Err(CliError::Invalid("operation was not confirmed".to_owned()))
    }
}

fn output<T: Serialize>(cli: &Cli, kind: &str, value: &T) -> Result<(), CliError> {
    let document = json!({
        "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
        "type": kind,
        "value": value,
    });
    let encoded = if cli.json {
        serde_json::to_string(&document)
    } else {
        serde_json::to_string_pretty(&document)
    }
    .map_err(|error| CliError::Internal(error.to_string()))?;
    println!("{encoded}");
    Ok(())
}

fn safe_identity(value: &str) -> Result<(), CliError> {
    if value.is_empty()
        || value.len() > 256
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CliError::Invalid("resource identity is invalid".to_owned()));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("task failed: {0}")]
    FailedTask(String),
    #[error("internal error: {0}")]
    Internal(String),
}

fn exit_code(error: &CliError) -> u8 {
    match error {
        CliError::Invalid(_) => 2,
        CliError::NotFound(_) => 6,
        CliError::Internal(_) => 7,
        CliError::FailedTask(_) => 8,
        CliError::Client(client) => match status_class(client).map(|status| status.as_u16()) {
            Some(401 | 403) => 3,
            Some(409) => 4,
            Some(429 | 502 | 503 | 504) | None if client.retryable() => 5,
            Some(404) => 6,
            _ => 7,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_output_schema_is_stable_and_has_no_control_characters()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = serde_json::to_string(&json!({
            "schema_version": JSON_OUTPUT_SCHEMA_VERSION,
            "type": "fixture",
            "value": {"ok": true}
        }))?;
        assert_eq!(
            value,
            r#"{"schema_version":1,"type":"fixture","value":{"ok":true}}"#
        );
        assert!(!value.contains('\u{1b}'));
        Ok(())
    }

    #[test]
    fn high_risk_json_mode_requires_yes() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from(["milkdrift", "--json", "daemon", "health"])?;
        assert!(confirm(&cli, "test").is_err());
        Ok(())
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&CliError::Invalid("fixture".to_owned())), 2);
        assert_eq!(exit_code(&CliError::FailedTask("fixture".to_owned())), 8);
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(
                milkdrift_control_protocol::ErrorEnvelope::new(
                    milkdrift_control_protocol::ErrorCode::Unauthorized,
                    "fixture",
                    false,
                ),
            ))),
            3
        );
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(
                milkdrift_control_protocol::ErrorEnvelope::new(
                    milkdrift_control_protocol::ErrorCode::Conflict,
                    "fixture",
                    false,
                ),
            ))),
            4
        );
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(
                milkdrift_control_protocol::ErrorEnvelope::new(
                    milkdrift_control_protocol::ErrorCode::Overload,
                    "fixture",
                    true,
                ),
            ))),
            5
        );
    }

    #[test]
    fn artifact_destination_must_be_new() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("artifact.bin");
        drop(create_download_destination(&destination)?);
        assert!(create_download_destination(&destination).is_err());
        Ok(())
    }
}
