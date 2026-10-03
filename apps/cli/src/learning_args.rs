//! Named evaluation actions keep receipt selection explicit and leave criteria with the daemon.
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true, arg_required_else_help = true)]
pub(crate) struct LearningArgs {
    /// Advanced strict LearningRequest document (also supports preauthorization).
    pub(crate) file: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Option<LearningCommand>,
}

#[derive(Subcommand)]
pub(crate) enum LearningCommand {
    /// Freeze authorized source artifacts and exact history pages; no scores are accepted.
    Select {
        method: String,
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        guidance: String,
        #[arg(long)]
        artifact: Vec<String>,
        /// Repeat for each explicit page. FIRST is inclusive; COUNT must be 1..=64.
        #[arg(long, num_args = 3, required = true, value_names = ["RUN", "FIRST", "COUNT"])]
        page: Vec<String>,
        #[arg(long, num_args = 2, value_names = ["ACTOR", "COMMAND"])]
        supersedes: Vec<String>,
        #[arg(long, num_args = 2, value_names = ["ACTOR", "COMMAND"])]
        approval: Vec<String>,
    },
    /// Fix an explicit evaluator-owned declaration before the proposal run exists.
    Declare { file: PathBuf },
    /// Submit the actual model-produced proposal; the daemon checks retained provenance.
    Candidate {
        proposal: PathBuf,
        #[arg(long, num_args = 2, required = true, value_names = ["ACTOR", "COMMAND"])]
        declaration: Vec<String>,
        #[arg(long)]
        expected_benefit: String,
        #[arg(long)]
        applicability: String,
        #[arg(long)]
        counterevidence: String,
    },
    /// Derive a comparison from recorded work under its fixed declaration.
    Compare {
        #[arg(long, num_args = 2, required = true, value_names = ["ACTOR", "COMMAND"])]
        declaration: Vec<String>,
        #[arg(long, num_args = 2, required = true, value_names = ["ACTOR", "COMMAND"])]
        candidate: Vec<String>,
    },
    /// Read an exact selection, declaration, candidate, comparison or promotion receipt.
    Inspect { actor: String, command: String },
    /// Publish only a candidate matching an eligible comparison and current publish authority.
    Promote {
        method: PathBuf,
        #[arg(long, num_args = 2, required = true, value_names = ["ACTOR", "COMMAND"])]
        comparison: Vec<String>,
        #[arg(long)]
        expected_previous_version: u64,
    },
}
