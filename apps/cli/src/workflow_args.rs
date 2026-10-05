use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct WorkflowArgs {
    /// Refuse if the draft's edit token differs from the last inspected value.
    #[arg(long, global = true)]
    pub(crate) expected_edit: Option<String>,
    #[command(subcommand)]
    pub(crate) command: WorkflowCommand,
}

#[derive(Subcommand)]
pub(crate) enum WorkflowCommand {
    /// Find saved versions in one bounded page; choose an exact revision when running or editing.
    List {
        #[arg(long)]
        workflow: Option<String>,
        #[arg(long, default_value_t = 32)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read a saved version's name, inputs, outputs and provenance without raw graph JSON.
    Show { revision: String },
    /// Save an independent copy and open its new editable draft. Governing agreements refuse copy.
    Copy {
        revision: String,
        workflow: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Create a new local draft through the daemon's authoring operation.
    New {
        workflow: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Reopen one exact saved revision in a new local draft file.
    Open {
        revision: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// List the caller's permitted model capabilities and current availability.
    Models,
    /// Inspect prompts, steps, selected models and connections.
    Inspect { file: PathBuf },
    /// Add a model step; prompt is a UTF-8 file or - for stdin.
    Add {
        file: PathBuf,
        step: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        prompt: PathBuf,
        #[arg(long)]
        maximum_output_units: u64,
    },
    /// Replace a step's prompt from a UTF-8 file or stdin.
    Prompt {
        file: PathBuf,
        step: String,
        #[arg(long)]
        prompt: PathBuf,
    },
    /// Change an existing model step's per-invocation output allowance.
    OutputLimit {
        file: PathBuf,
        step: String,
        maximum_output_units: u64,
    },
    /// Explicitly change the selected model.
    Model {
        file: PathBuf,
        step: String,
        model: String,
    },
    /// Declare a required per-run input without supplying its value.
    Input { file: PathBuf, name: String },
    /// Remove a declared input after disconnecting every consumer.
    RemoveInput { file: PathBuf, name: String },
    /// Rename a declared input and all its bindings; step ports and prompts stay unchanged.
    RenameInput {
        file: PathBuf,
        name: String,
        new_name: String,
    },
    /// Bind a named step input to a run input or an earlier step's final text.
    Connect {
        file: PathBuf,
        step: String,
        input: String,
        #[arg(
            long,
            required_unless_present = "from_step",
            conflicts_with = "from_step"
        )]
        run_input: Option<String>,
        #[arg(long)]
        from_step: Option<String>,
    },
    /// Remove one step input connection.
    Disconnect {
        file: PathBuf,
        step: String,
        input: String,
    },
    /// Select the final text exposed after result acceptance.
    Output {
        file: PathBuf,
        step: String,
        #[arg(long, default_value = "result")]
        name: String,
    },
    /// Clear the final-output selection; the incomplete draft can be continued but not saved.
    ClearOutput { file: PathBuf },
    /// Remove an unused step. Connected or selected output steps must be disconnected first.
    Remove { file: PathBuf, step: String },
    /// Move a step before another, or to the end when --before is absent.
    Move {
        file: PathBuf,
        step: String,
        #[arg(long)]
        before: Option<String>,
    },
    /// Change the workflow's display name.
    Rename { file: PathBuf, name: String },
    /// Validate and save an immutable revision, then advance this file's exact base.
    Save { file: PathBuf },
}
