//! Continue retained source evidence through the supported learning and publication operations.
mod author;
use super::{
    client::{Session, load},
    prepare,
};
use milkdrift_evidence::EvidenceResult;
use std::path::PathBuf;

#[derive(clap::Args)]
pub(super) struct Arguments {
    /// Accepted 04 qualification directory. Its private host retains the source run and artifacts.
    #[arg(long)]
    pub root: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift")]
    pub cli: PathBuf,
    #[arg(long, default_value = "target/debug/milkdrift-daemon")]
    pub daemon: PathBuf,
    /// Exact native Slotbook image, containing the seeded and corrected fixture binaries.
    #[arg(long)]
    pub image: String,
    #[arg(long, default_value = "target/debug/slotbook-verifier")]
    pub verifier: PathBuf,
    /// Operator-reviewed endpoint profile, with structured JSON support and one generation at a time.
    #[arg(long)]
    pub model_profile: PathBuf,
    /// Native static workspace tool whose exact bytes are recorded before recipe promotion.
    #[arg(
        long,
        default_value = "target/slotbook-static/x86_64-unknown-linux-gnu/release/slotbook-workspace"
    )]
    pub workspace_tool: PathBuf,
    /// New prospective proposal after an unaccepted candidate; preserves prior runs and fixed cases.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=4))]
    pub proposal_attempt: u8,
    /// New command for revalidating the same retained response after a refused local submission.
    #[arg(long, default_value_t=1, value_parser=clap::value_parser!(u8).range(1..=4))]
    pub candidate_validation: u8,
    /// Retain an accepted candidate for later evaluation with --resume using the same attempt.
    #[arg(long)]
    pub proposal_only: bool,
    #[arg(long, default_value_t = 19758)]
    pub port: u16,
    /// First of eleven loopback service ports allocated when preparing a new study.
    #[arg(long, default_value_t = 19900, value_parser = clap::value_parser!(u16).range(1024..=65525))]
    pub service_port_base: u16,
    /// Resume the exact retained inputs after an interrupted run; conflicting files are refused.
    #[arg(long)]
    pub resume: bool,
}

pub(super) fn run(args: Arguments) -> EvidenceResult {
    if args.proposal_attempt > 1 && !args.resume {
        return Err("a new proposal attempt requires --resume of its frozen study".into());
    }
    let root = args.root.canonicalize()?;
    let directory = root.join("learning-05");
    if !args.resume {
        prepare::private_directory(&directory)?;
    }
    let mut session = Session::resume(&root, &args.cli, &args.daemon, args.port)?;
    let source = load(root.join("qualification.json"))?;
    if !args.resume {
        author::prepare(&args, &session, &source)?;
    } else if args.proposal_attempt > 1 {
        let profile = args.model_profile.canonicalize()?;
        let model = load(&profile)?;
        let mut config: serde_json::Value =
            toml::from_str(&std::fs::read_to_string(root.join("host/daemon.toml"))?)?;
        let endpoint = url::Url::parse(crate::client::text(&model["base_url"])?)?;
        let destination = serde_json::json!(format!(
            "{}:{}",
            endpoint.host_str().ok_or("model host absent")?,
            endpoint
                .port_or_known_default()
                .ok_or("model port absent")?
        ));
        let operator = config["actors"]
            .as_array()
            .ok_or("actors absent")?
            .iter()
            .find(|actor| actor["actor"] == author::OPERATOR)
            .ok_or("learning operator absent")?;
        let network = &operator["authority"]["resources"]["network"];
        milkdrift_evidence::application::ensure(
            network["profiles"]
                .as_array()
                .is_some_and(|values| values.contains(&model["identity"]))
                && network["destinations"]
                    .as_array()
                    .is_some_and(|values| values.contains(&destination)),
            "the new proposal profile requires a separately reviewed network grant; use a fresh study for a different endpoint",
        )?;
        config["bind"] = serde_json::json!(format!("127.0.0.1:{}", args.port));
        config["adapters"]["model_profiles"] =
            serde_json::json!([{"capability_id":author::MODEL,"profile":profile}]);
        author::save_config(&root, &config)?;
    }
    session.start()?;
    let result = author::exercise(&args, &mut session, &source);
    let stop = session.stop();
    result?;
    stop
}
