//! Thin operator CLI over `milkdrift-control-client`.

use std::{env, path::PathBuf, process::ExitCode};

use clap::{Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
use milkdrift_control_protocol::EvidenceRef;
use url::Url;

mod command;
mod error;
mod session;

use error::{CliError, emit_error, exit_code};

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
    /// Optional exact semantic revision guard.
    #[arg(long, global = true)]
    expected_revision: Option<String>,
    /// Durable external evidence reference in KIND=ID form; repeat at most 32 times.
    #[arg(
        long,
        global = true,
        value_name = "KIND=ID",
        value_parser = parse_evidence_reference
    )]
    evidence: Vec<EvidenceRef>,
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
    /// Ordered implementation prompt-sequence operations.
    Sequence {
        #[command(subcommand)]
        command: SequenceCommand,
    },
    /// Durable run operations.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    /// Durable bounded controller lifecycle operations.
    Controller {
        #[command(subcommand)]
        command: ControllerCommand,
    },
    /// Inspect one node execution.
    Node(NodeInspect),
    /// Inspect or resolve one exact attempt.
    Attempt {
        #[command(subcommand)]
        command: AttemptCommand,
    },
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
    Health(StreamArgs),
    /// Require readiness after recovery/adapters.
    Readiness,
    /// Inspect the server-owned actor and exact immutable grant revision.
    Authority,
}

#[derive(Subcommand)]
enum SequenceCommand {
    /// Parse and compile a JSON or Markdown sequence without storing it.
    Validate { file: PathBuf },
    /// Parse, compile, and store an ordinary immutable blueprint revision.
    Import { file: PathBuf },
    /// Inspect the exact generated ordinary blueprint revision.
    Show { revision: String },
    /// Show the generated revision and current run stage frontier together.
    Status { run: String, revision: String },
    /// Inspect every current node occurrence belonging to one imported stage.
    Stage { run: String, stage: String },
    /// Submit a bounded prospective remediation/re-verification/re-review revision.
    Remediate {
        /// Original sequence document used for the exact stage contract.
        sequence_file: PathBuf,
        /// Paused live run.
        run: String,
        /// Exact current base revision.
        revision: String,
        /// Failed imported stage identity.
        stage: String,
        /// Nonzero bounded remediation generation.
        #[arg(long)]
        generation: u16,
        /// Stable proposal identity.
        #[arg(long)]
        proposal: String,
        /// Fresh remediation prompt Markdown/text file.
        #[arg(long)]
        prompt: PathBuf,
    },
}

#[derive(Subcommand)]
enum BlueprintCommand {
    /// Validate one exact versioned blueprint JSON document without storing it.
    Validate { file: PathBuf },
    /// Import one exact versioned blueprint JSON document.
    Import { file: PathBuf },
    /// Inspect one immutable revision.
    Show {
        revision: String,
        /// Emit the exact canonical stored document to stdout without a presentation wrapper.
        #[arg(long, conflicts_with = "output")]
        document: bool,
        /// Write the exact canonical stored document to a new file.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
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

#[derive(Args)]
struct StreamArgs {
    /// Opaque stream continuation from an earlier observation.
    #[arg(long)]
    cursor: Option<String>,
    /// Follow resumable observations after the initial read.
    #[arg(long)]
    follow: bool,
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

#[derive(Subcommand)]
enum ControllerCommand {
    /// Inspect exact progress, limits, checkpoint, and reached-bound provenance.
    Status {
        run: String,
        controller_execution: String,
    },
    /// Continue one exact human checkpoint under the current actor grant.
    Continue {
        run: String,
        controller_execution: String,
        decision_id: String,
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
enum AttemptCommand {
    /// Inspect one exact current or historical attempt.
    Inspect(AttemptInspect),
    /// Ask the daemon to resolve retained or uncertain external work.
    Resolve(AttemptResolve),
}

#[derive(Args)]
struct AttemptResolve {
    /// Run aggregate.
    run: String,
    /// Immutable retained or uncertain attempt identity.
    attempt: String,
    /// Exact reconciliation decision identity.
    decision: String,
    /// Explicit daemon-evaluated resolution action.
    #[arg(long, value_enum)]
    action: ResolveChoice,
    /// Exact remediation node; required only for compensation.
    #[arg(long)]
    remediation_node: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ResolveChoice {
    /// Query external truth without claiming a terminal outcome.
    Query,
    /// Request retry under the runtime's durable idempotency policy.
    Retry,
    /// Create explicit compensation at the selected remediation node.
    Compensate,
    /// Keep the uncertain obligation visible.
    Retain,
    /// Resolve succeeded only from supplied durable evidence.
    ResolveSucceeded,
    /// Resolve failed only from supplied durable evidence.
    ResolveFailed,
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
    List(StreamArgs),
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
    let json_requested = env::args_os().any(|argument| argument.to_str() == Some("--json"));
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(u8::try_from(code).unwrap_or(0));
        }
        Err(error) => {
            if json_requested {
                let failure = CliError::Invalid(
                    "command-line arguments are invalid; use --help for the accepted syntax"
                        .to_owned(),
                );
                emit_error(true, &failure);
                return ExitCode::from(exit_code(&failure));
            }
            let code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(u8::try_from(code).unwrap_or(2));
        }
    };
    let json = cli.json;
    match command::execute(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            emit_error(json, &error);
            ExitCode::from(exit_code(&error))
        }
    }
}

fn parse_evidence_reference(value: &str) -> Result<EvidenceRef, String> {
    let (kind, id) = value
        .split_once('=')
        .ok_or_else(|| "evidence must use KIND=ID syntax".to_owned())?;
    if kind.is_empty() || id.is_empty() || id.contains('=') {
        return Err("evidence must contain one nonempty KIND and ID".to_owned());
    }
    Ok(EvidenceRef {
        id: id.to_owned(),
        kind: kind.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_families_retain_their_clap_shapes() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from([
            "milkdrift",
            "--json",
            "--command-id",
            "command-fixed",
            "run",
            "start",
            "run-one",
            "workflow-one",
            "revision-one",
        ])?;
        assert!(cli.json);
        assert_eq!(cli.command_id.as_deref(), Some("command-fixed"));
        assert!(matches!(
            cli.command,
            TopCommand::Run {
                command: RunCommand::Start { run, workflow, revision }
            } if run == "run-one" && workflow == "workflow-one" && revision == "revision-one"
        ));
        Ok(())
    }

    #[test]
    fn attempt_resolution_and_evidence_have_unambiguous_shapes()
    -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli::try_parse_from([
            "milkdrift",
            "--expected-revision",
            "revision-one",
            "--evidence",
            "artifact=artifact-one",
            "attempt",
            "resolve",
            "run-one",
            "attempt-one",
            "decision-one",
            "--action",
            "resolve-succeeded",
        ])?;
        assert_eq!(cli.expected_revision.as_deref(), Some("revision-one"));
        assert_eq!(cli.evidence.len(), 1);
        assert!(matches!(
            cli.command,
            TopCommand::Attempt {
                command: AttemptCommand::Resolve(AttemptResolve {
                    action: ResolveChoice::ResolveSucceeded,
                    ..
                })
            }
        ));
        Ok(())
    }
}
