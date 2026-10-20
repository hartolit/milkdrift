//! Project paths identify folders on the selected daemon, never on the CLI machine.
use clap::{Args, Subcommand, ValueEnum};

#[derive(Args)]
pub(crate) struct ProjectArgs {
    #[command(subcommand)]
    pub(crate) command: ProjectCommand,
}

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// Discover authorized projects without guessing their identities.
    List {
        #[arg(long, default_value_t = 30)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read the folder, availability and current prospective settings.
    Show { project: String },
    /// Browse one bounded page of permitted folders on the daemon.
    Directories {
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Inspect configured compatible host and Podman environments.
    Environments,
    /// Register an existing folder with initial read-only host file access.
    Attach { name: String, folder: String },
    /// Exclusively create one new child beneath an existing permitted parent.
    Create {
        name: String,
        parent: String,
        folder_name: String,
    },
    /// Rename only the registration, preserving the folder.
    Rename {
        project: String,
        name: String,
        #[arg(long)]
        expected_generation: u64,
    },
    /// Change defaults for new tasks; accepted selections stay unchanged.
    Settings {
        project: String,
        #[arg(long)]
        expected_generation: u64,
        #[command(flatten)]
        settings: EnvironmentArgs,
    },
    /// Remove an unused registration; never delete its folder.
    Remove {
        project: String,
        #[arg(long)]
        expected_generation: u64,
    },
    /// Associate earlier work for later submissions without rewriting history.
    Associate {
        project: String,
        workflow: String,
        #[arg(long)]
        expected_generation: u64,
    },
    /// Recover the original exact command result.
    Receipt { request: String },
    /// Inspect unresolved folder creations owned by this actor.
    Operations,
    /// Attach the inspected intended folder after an unresolved creation.
    Reconcile { operation: String },
    /// Abandon an unresolved registration attempt, preserving any directory.
    Abandon { operation: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum EnvironmentKind {
    Host,
    Process,
    Podman,
}

#[derive(Args)]
pub(crate) struct EnvironmentArgs {
    #[arg(long, value_enum)]
    pub(crate) environment: EnvironmentKind,
    /// Exact configured capability returned by project environments.
    #[arg(long)]
    pub(crate) capability: Option<String>,
    #[arg(long)]
    pub(crate) operation: Option<String>,
    /// Exact configured environment generation reviewed by the caller.
    #[arg(long)]
    pub(crate) environment_generation: Option<u64>,
    /// Deliberately enable project writes for new tasks.
    #[arg(long)]
    pub(crate) allow_changes: bool,
}
