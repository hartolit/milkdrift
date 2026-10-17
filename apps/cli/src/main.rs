//! Submit operator intent and present daemon evidence through `milkdrift-control-client`.
//!
//! Arguments and the overall deadline live here; session, command families, and output owners
//! implement the call. Expiring a CLI request or pressing Ctrl-C stops local waiting without
//! deciding whether a daemon command was accepted or cancelling the workflow.

use std::{env, io::IsTerminal as _, path::PathBuf, process::ExitCode, time::Duration};

use clap::{Args, Parser, Subcommand, error::ErrorKind};
use url::Url;

mod command;
mod error;
mod input;
mod output;
mod session;
mod workflow_args;

#[cfg(test)]
#[path = "tests/documentation.rs"]
mod documentation;

use error::{CliError, report};

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
    /// Hard wall-clock bound including connection, input, retries and observations (default 60).
    /// Required explicitly for invocation wait and noninteractive follow; maximum 86400 seconds.
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..=86_400))]
    timeout_secs: Option<u64>,
    /// Maximum reconnects across an observation command.
    #[arg(long, global = true, default_value_t = 5, value_parser = clap::value_parser!(u16).range(0..=100))]
    max_reconnects: u16,
    /// Confirm high-risk operations and permit noninteractive execution.
    #[arg(long, global = true)]
    yes: bool,
    /// Stable command idempotency identity; generated when omitted.
    #[arg(long, global = true)]
    command_id: Option<String>,
    /// Bounded durable command reason.
    #[arg(long, global = true, default_value = "operator CLI command")]
    reason: String,
    #[command(subcommand)]
    command: TopCommand,
}

#[derive(Subcommand)]
enum TopCommand {
    /// Create, edit and inspect daemon-owned task workflows through the daemon.
    Workflow(workflow_args::WorkflowArgs),
    /// Approved persistent installation lifecycle, shared with workflow capabilities.
    Resource(ResourceArgs),
    /// Independent execution on the explicitly selected daemon endpoint.
    Invocation {
        #[command(subcommand)]
        command: InvocationCommand,
    },
    /// Daemon lifecycle observations.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Live capability generations.
    Capability {
        #[command(subcommand)]
        command: CapabilityCommand,
    },
    /// Provider profiles projected from authorized capability generations.
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
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
}

#[derive(clap::Args)]
struct ResourceArgs {
    #[arg(long)]
    installation: String,
    #[arg(long, default_value_t = 0)]
    expected_version: u64,
    #[command(subcommand)]
    command: ResourceCommand,
}

#[derive(Subcommand)]
enum ResourceCommand {
    /// Run the target's fixed trusted verifier against an immutable candidate artifact.
    Evaluate {
        #[arg(long)]
        artifact: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        media_type: String,
        #[arg(long)]
        size_bytes: u64,
    },
    /// Inspect retained trusted verification observations, including failed/unknown checks.
    Evidence {
        #[arg(long)]
        evaluation: String,
    },
    /// Publish the exact verified candidate under the current protected target policy.
    Publish {
        #[arg(long)]
        evaluation: String,
    },
    Handoff(EditingArgs),
    Return {
        #[command(flatten)]
        transfer: EditingArgs,
        #[arg(long)]
        resume_parent: bool,
    },
    Prepare {
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        digest: String,
    },
    Apply {
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        digest: String,
    },
    Inspect,
    Start,
    Stop,
    Update {
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        allow_interruption: bool,
    },
    Preserve {
        /// Permit deletion of owned data during removal; default retains data.
        #[arg(long)]
        delete_on_removal: bool,
    },
    Remove,
    Recover,
    Resolve {
        #[arg(long)]
        use_id: String,
        #[arg(long)]
        expected_claim: u64,
    },
}

#[derive(clap::Args)]
struct EditingArgs {
    #[arg(long)]
    generation: u64,
    #[arg(long)]
    parent: String,
    #[arg(long)]
    child: String,
    #[arg(long)]
    parent_claim: u64,
    #[arg(long)]
    child_claim: u64,
    #[arg(long)]
    association: String,
}

#[derive(Subcommand)]
enum InvocationCommand {
    /// Discover the host identity, limits and exact callable generations.
    Catalog,
    /// Save an exact request from current discovery without admitting execution.
    Prepare {
        capability: String,
        operation: String,
        #[arg(long)]
        host: String,
        #[arg(long)]
        request_id: String,
        /// JSON array of named inline or artifact input references.
        #[arg(long)]
        inputs: Option<PathBuf>,
        /// Upload a UTF-8 file as a named restricted text input; may be repeated.
        #[arg(long, value_name = "NAME=FILE")]
        input: Vec<String>,
        /// New file to retain for submission and exact replay.
        #[arg(long)]
        output: PathBuf,
    },
    /// Submit an immutable direct request document once; save it for exact replay.
    Submit { file: PathBuf },
    /// Recover durable acceptance after a lost reply.
    Lookup { request: String },
    /// Inspect one accepted invocation in the authenticated caller's namespace.
    Show { execution: String },
    /// Read one bounded observation page.
    Observations {
        execution: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u32).range(1..=1024))]
        limit: u32,
    },
    /// Follow bounded pages until terminal evidence or retained uncertainty; requires a timeout.
    Wait { execution: String },
    /// Download one retained terminal output of this caller's accepted invocation.
    Output {
        execution: String,
        artifact: String,
        destination: PathBuf,
    },
    /// Request cancellation; acknowledgement is separate from execution outcome.
    Cancel {
        execution: String,
        #[arg(long)]
        request_id: String,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
        sequence: u64,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Read this daemon's shared name, generation and rename permission.
    Name,
    /// Rename this daemon for every client using an observed generation and exact request identity.
    Rename {
        /// Proposed shared name. The daemon validates and trims it.
        name: String,
        /// Previously observed name generation from daemon name.
        #[arg(long)]
        expected_generation: u64,
        /// Preserve this identity and every argument if the response is lost.
        #[arg(long)]
        request_id: String,
    },
    /// Recover this actor's accepted rename after a lost reply.
    RenameReceipt { request_id: String },
    /// Read liveness state.
    Health(StreamArgs),
    /// Require readiness after recovery/adapters.
    Readiness,
    /// Inspect the server-owned actor and exact immutable grant revision.
    Authority,
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
enum CapabilityCommand {
    /// List visible generation health.
    List(StreamArgs),
    /// Show all generations for one capability identity.
    Show { capability: String },
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// List authorized profile identities and their exact registered generations.
    List,
    /// Show generations bound to one exact provider profile.
    Show { profile: String },
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
    /// Hash a bounded local candidate or verifier file without uploading or executing it.
    Digest { file: PathBuf },
    /// Publish one complete input file (at most 512 KiB) with authenticated provenance.
    Upload {
        file: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        upload_id: String,
        #[arg(long, default_value = "application/octet-stream")]
        media_type: String,
        #[arg(long, default_value = "restricted", value_parser = ["restricted", "internal", "public"])]
        sensitivity: String,
    },
    /// Read safe immutable metadata.
    Metadata { artifact: String },
    /// Download verified bounded ranges into one new explicit destination.
    Get {
        artifact: String,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return ExitCode::from(9),
    };
    let exit = runtime.block_on(run());
    // A timed-out stdin read can still be blocked in the OS. It owns no product state;
    // bounded runtime shutdown lets the CLI process release it on exit.
    runtime.shutdown_timeout(Duration::from_millis(50));
    exit
}

async fn run() -> ExitCode {
    let json_requested = env::args_os().any(|argument| argument.to_str() == Some("--json"));
    let mut cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            if json_requested {
                let kind = if error.kind() == ErrorKind::DisplayVersion {
                    "version"
                } else {
                    "help"
                };
                match output::encode(
                    kind,
                    None,
                    "success",
                    serde_json::json!({"text": error.to_string()}),
                    serde_json::Value::Null,
                    true,
                )
                .and_then(|encoded| output::line(format_args!("{encoded}")))
                {
                    Ok(()) => {}
                    Err(failure) => return report(true, kind, None, &failure),
                }
            } else {
                if error.print().is_err() {
                    return ExitCode::from(9);
                }
            }
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            if json_requested {
                let failure = CliError::Invalid(
                    "command-line arguments are invalid; use --help for the accepted syntax"
                        .to_owned(),
                );
                return report(true, "arguments", None, &failure);
            }
            let code = error.exit_code();
            if error.print().is_err() {
                return ExitCode::from(9);
            }
            return ExitCode::from(u8::try_from(code).unwrap_or(2));
        }
    };
    let json = cli.json;
    let operation = cli.operation();
    cli.command_id
        .get_or_insert_with(session::generated_command_id);
    let command_id = cli.command_id.clone();
    if (cli.is_wait() || (cli.is_follow() && !std::io::stdout().is_terminal()))
        && cli.timeout_secs.is_none()
    {
        let error = CliError::Invalid(
            "invocation wait and noninteractive follow require explicit --timeout-secs".to_owned(),
        );
        return report(json, operation, command_id.as_deref(), &error);
    }
    let deadline = cli
        .timeout_secs
        .or_else(|| (!cli.is_follow()).then_some(60));
    let work = command::execute(cli);
    tokio::pin!(work);
    let result = tokio::select! {
        result = &mut work => result,
        () = async {
            if let Some(seconds) = deadline { tokio::time::sleep(Duration::from_secs(seconds)).await; }
            else { std::future::pending::<()>().await; }
        } => Err(CliError::Deadline),
        signal = tokio::signal::ctrl_c() => match signal {
            Ok(()) => Err(CliError::Cancelled),
            Err(error) => Err(CliError::Internal(error.to_string())),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(json, operation, command_id.as_deref(), &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_commands_use_the_shared_identity_and_exact_version()
    -> Result<(), Box<dyn std::error::Error>> {
        for verb in ["inspect", "start", "stop", "remove", "recover", "preserve"] {
            let cli = Cli::try_parse_from([
                "milkdrift",
                "--command-id",
                "resource-one",
                "resource",
                "--installation",
                "slotbook",
                "--expected-version",
                "7",
                verb,
            ])?;
            assert_eq!(cli.command_id.as_deref(), Some("resource-one"));
            let TopCommand::Resource(args) = cli.command else {
                return Err("wrong command".into());
            };
            assert_eq!(args.expected_version, 7);
        }
        let cli = Cli::try_parse_from([
            "milkdrift",
            "resource",
            "--installation",
            "slotbook",
            "update",
            "--recipe",
            "slotbook",
            "--digest",
            "digest",
            "--allow-interruption",
        ])?;
        assert!(matches!(
            cli.command,
            TopCommand::Resource(ResourceArgs {
                command: ResourceCommand::Update {
                    allow_interruption: true,
                    ..
                },
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn retired_workflow_commands_are_not_accepted() {
        for command in [
            "run",
            "blueprint",
            "sequence",
            "controller",
            "proposal",
            "method",
            "learning",
            "layout",
            "attempt",
            "node",
        ] {
            assert!(
                Cli::try_parse_from(["milkdrift", command]).is_err(),
                "{command}"
            );
        }
    }
}
