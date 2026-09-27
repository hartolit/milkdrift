//! Prepare and qualify the maintained Slotbook method through actual product binaries.
macro_rules! args { ($($value:expr),* $(,)?) => { vec![$(format!("{}", $value)),*] }; }
mod client;
mod develop;
mod learning;
mod model_fixture;
mod prepare;
mod preservation;
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
    /// Develop source in a managed worker using a real model or explicitly assisted input.
    Develop(develop::Arguments),
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
    /// Stop this scenario's owned service before renewal when only two private mappings fit.
    #[arg(long)]
    drain_before_renewal: bool,
}
fn main() -> EvidenceResult {
    match Arguments::parse().command {
        Action::Prepare(args) => prepare::run(args),
        Action::Qualify(args) => qualification::run(args),
        Action::Develop(args) => develop::run(args),
        Action::Learn(args) => learning::run(args),
        Action::ModelFixture(args) => model_fixture::run(args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assisted_source_is_an_explicit_exclusive_single_submission() -> EvidenceResult {
        let base = [
            "slotbook-evidence",
            "develop",
            "--root",
            "unused",
            "--image",
            "unused",
            "--assisted-source",
            "source.rs",
        ];
        for extra in [
            vec!["--model-profile", "model.json"],
            vec!["--repair-model-profile", "repair.json"],
            vec!["--seeded-initial"],
        ] {
            assert!(Arguments::try_parse_from(base.iter().copied().chain(extra)).is_err());
        }
        let parsed = Arguments::try_parse_from(base)?;
        let Action::Develop(args) = parsed.command else {
            return Err("wrong action".into());
        };
        let error = develop::run(args)
            .err()
            .ok_or("multiple assisted submissions accepted")?;
        assert!(error.to_string().contains("--maximum-attempts 1"));
        assert!(
            Arguments::try_parse_from(base.into_iter().chain(["--maximum-attempts", "1"])).is_ok()
        );
        Ok(())
    }
}
