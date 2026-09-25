//! Prepare and qualify the maintained Slotbook method through actual product binaries.
macro_rules! args { ($($value:expr),* $(,)?) => { vec![$(format!("{}", $value)),*] }; }
mod client;
mod learning;
mod model_fixture;
mod prepare;
mod publication;
mod qualification;

use clap::{Parser, Subcommand, ValueEnum};
use milkdrift_evidence::EvidenceResult;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Author and qualify the finite Rust Slotbook example")]
struct Arguments {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Write a fresh private example using the product's authoring commands.
    Prepare(Prepare),
    /// Execute and inspect the configured method; retain resources on failure for recovery.
    Qualify(Qualify),
    /// Continue an accepted source qualification with explicit held-out learning and variants.
    Learn(learning::Arguments),
    /// Serve a bounded, explicitly deterministic proposal endpoint for validation scenarios.
    ModelFixture(model_fixture::Arguments),
}
#[derive(clap::Args)]
struct Prepare {
    #[arg(long)]
    root: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift")]
    cli: PathBuf,
    /// Approved exact image containing /fixtures/slotbook and /fixtures/slotbook-seeded.
    #[arg(long)]
    image: String,
    #[arg(long, default_value = "target/debug/slotbook-verifier")]
    verifier: PathBuf,
    #[arg(long, default_value_t = 6)]
    target_version: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum InvocationMode {
    Direct,
    Workflow,
    Peer,
}
impl InvocationMode {
    fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Workflow => "workflow",
            Self::Peer => "peer",
        }
    }
}
#[derive(clap::Args)]
struct Qualify {
    #[arg(long)]
    root: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift")]
    cli: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift-daemon")]
    daemon: PathBuf,
    #[arg(long, default_value_t = 19748)]
    port: u16,
    #[arg(long)]
    published: bool,
    #[arg(long,value_enum,default_value_t=InvocationMode::Direct)]
    invocation_mode: InvocationMode,
    /// Exact corrected binary used to build the image, for independent deployed-byte comparison.
    #[arg(long)]
    candidate: PathBuf,
}
fn main() -> EvidenceResult {
    match Arguments::parse().command {
        Action::Prepare(args) => prepare::run(args),
        Action::Qualify(args) => qualification::run(args),
        Action::Learn(args) => learning::run(args),
        Action::ModelFixture(args) => model_fixture::run(args),
    }
}
