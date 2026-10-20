use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct WorkflowArgs {
    #[command(subcommand)]
    pub(crate) command: WorkflowCommand,
}

#[derive(Subcommand)]
pub(crate) enum WorkflowCommand {
    /// Discover recent workflows across every permitted scope.
    List {
        #[arg(long, default_value = "")]
        search: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
        /// Restrict discovery to one daemon-owned project.
        #[arg(long, conflicts_with = "earlier")]
        project: Option<String>,
        /// Discover earlier workflows that have not been associated with a project.
        #[arg(long)]
        earlier: bool,
    },
    /// Read one page of a workflow's task summaries and actual times.
    Show {
        workflow: String,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Submit a typed JSON task request; the daemon allocates all identities.
    Create { file: PathBuf },
    /// Apply a complete typed, generation-guarded future-work batch.
    Edit {
        workflow: String,
        file: PathBuf,
        /// Read current guards and display the exact request without submitting it.
        #[arg(long)]
        preview: bool,
    },
    /// Find exact durable acceptance after a lost reply.
    Receipt { request: String },
    /// Read one task and its frozen execution facts.
    Task { workflow: String, task: String },
    /// Read an immutable task result.
    Result { workflow: String, task: String },
    /// Request cancellation; a request alone does not prove stopped.
    Stop { workflow: String, task: String },
    /// Upload and freeze UTF-8 file bytes for task inputs.
    Upload {
        file: PathBuf,
        #[arg(long)]
        request: String,
    },
    /// Inspect this daemon identity, actor and configured task choices.
    Capabilities,
}
