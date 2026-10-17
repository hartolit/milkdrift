//! Shared production-daemon fixture for finite task evidence and browser journeys.
mod animation;
mod animation_check;
mod check;
mod model;
mod report;
mod setup;

use clap::{Parser, Subcommand, ValueEnum};
use milkdrift_evidence::{
    EvidenceResult,
    application::{OwnedChild, ensure, write_private},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Create a fresh private root and serve two independently authorized production daemons.
    Serve {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        daemon: PathBuf,
        #[arg(long, default_value_t = 3600)]
        lifetime_secs: u64,
    },
    /// Exercise the actual CLI and public client against isolated production task daemons.
    Check {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        daemon: PathBuf,
        #[arg(long)]
        cli: PathBuf,
    },
    /// Analyze the exact task-supplied Cargo manifest. Used by the configured process adapter.
    CargoReport {
        #[arg(long, default_value = "Cargo.toml")]
        file: PathBuf,
        #[arg(long, default_value_t = 5000)]
        delay_ms: u64,
    },
    /// Ask the running Rust fixture owner to perform a bounded lifecycle operation.
    Control {
        #[arg(long)]
        root: PathBuf,
        #[arg(value_enum)]
        operation: Control,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Control {
    RestartA,
    RestartB,
    CrashA,
    CrashB,
    Stop,
}

#[tokio::main]
async fn main() -> EvidenceResult {
    match Arguments::parse().action {
        Action::Serve {
            root,
            daemon,
            lifetime_secs,
        } => run_fixture(&root, &daemon, lifetime_secs, None).await,
        Action::Check { root, daemon, cli } => run_fixture(&root, &daemon, 300, Some(&cli)).await,
        Action::CargoReport { file, delay_ms } => report::run(&file, delay_ms),
        Action::Control { root, operation } => {
            let operation = match operation {
                Control::RestartA => "restart-a",
                Control::RestartB => "restart-b",
                Control::CrashA => "crash-a",
                Control::CrashB => "crash-b",
                Control::Stop => "stop",
            };
            write_private(&root.join("control-request"), operation.as_bytes())?;
            Ok(())
        }
    }
}

struct Daemon {
    child: OwnedChild,
    config: PathBuf,
    endpoint: String,
    token: String,
    log: PathBuf,
}

fn start(executable: &Path, config: &Path, log: &Path) -> EvidenceResult<OwnedChild> {
    let stderr = fs::OpenOptions::new().create(true).append(true).open(log)?;
    OwnedChild::spawn(
        Command::new(executable)
            .arg("--config")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr)),
    )
}

async fn ready(daemon: &mut Daemon) -> EvidenceResult {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        ensure(
            daemon.child.try_wait()?.is_none(),
            "production daemon exited before readiness; inspect private daemon log",
        )?;
        if let Ok(reply) = client
            .get(format!("{}/v1/session", daemon.endpoint))
            .bearer_auth(&daemon.token)
            .send()
            .await
            && reply.status().is_success()
        {
            return Ok(());
        }
        ensure(
            Instant::now() < deadline,
            "production daemon did not become ready within 30 seconds",
        )?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn run_fixture(
    root: &Path,
    daemon: &Path,
    lifetime_secs: u64,
    cli: Option<&Path>,
) -> EvidenceResult {
    ensure(
        (30..=14_400).contains(&lifetime_secs),
        "fixture lifetime must be 30..=14400 seconds",
    )?;
    let daemon_source = daemon.canonicalize()?;
    let fixture_source = std::env::current_exe()?.canonicalize()?;
    let root = setup::fresh_root(root)?;
    let tool_root = root.join("tools");
    fs::create_dir(&tool_root)?;
    let daemon = tool_root.join(if cfg!(windows) {
        "milkdrift-daemon.exe"
    } else {
        "milkdrift-daemon"
    });
    fs::copy(&daemon_source, &daemon)?;
    ensure(
        milkdrift_evidence::application::hash_file(&daemon_source)?
            == milkdrift_evidence::application::hash_file(&daemon)?,
        "immutable daemon executable copy differs from the built daemon",
    )?;
    let fixture = tool_root.join(if cfg!(windows) {
        "cargo-report.exe"
    } else {
        "cargo-report"
    });
    fs::copy(&fixture_source, &fixture)?;
    ensure(
        milkdrift_evidence::application::hash_file(&fixture_source)?
            == milkdrift_evidence::application::hash_file(&fixture)?,
        "immutable report executable copy differs from the built tool",
    )?;
    let mut instances = Vec::new();
    let mut model = model::Model::start(&root)?;
    let mut connections = serde_json::Map::new();
    for name in ["a", "b"] {
        let configured = setup::configure(&root, &fixture, name, &model)?;
        let log = root.join(format!("daemon-{name}.log"));
        write_private(&log, b"")?;
        let child = start(&daemon, &configured.config, &log)?;
        let mut instance = Daemon {
            child,
            config: configured.config,
            endpoint: configured.endpoint,
            token: configured.token,
            log,
        };
        ready(&mut instance).await?;
        connections.insert(name.to_owned(), json!({"url":instance.endpoint,"identity":format!("host:browser-{name}"),"token":instance.token}));
        instances.push(instance);
    }
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    connections.insert("capability".into(), json!("cargo-report"));
    connections.insert("model_capability".into(), json!("controlled-model"));
    connections.insert("planning_capability".into(), json!("controlled-planner"));
    connections.insert(
        "planning_expect".into(),
        json!([
            "Animation composition",
            "Corrected render research",
            "Adopt corrected research",
            "Final animation report"
        ]),
    );
    connections.insert(
        "planning_research_titles".into(),
        json!([
            "Research motion representation",
            "Research training data",
            "Research render validation"
        ]),
    );
    connections.insert(
        "planning_project_root".into(),
        json!(root.join("daemon-a/animation-project")),
    );
    connections.insert(
        "model_expect".into(),
        json!(["Controlled analysis", "detected_sections", "input_digest"]),
    );
    connections.insert("input_a".into(), json!(repository.join("Cargo.toml")));
    connections.insert(
        "input_b".into(),
        json!(repository.join("crates/runtime/Cargo.toml")),
    );
    connections.insert(
        "expect_a".into(),
        json!([
            "Workspace members:",
            "Workspace dependency: milkdrift-runtime"
        ]),
    );
    connections.insert(
        "expect_b".into(),
        json!([
            "Package: milkdrift-runtime",
            "Dependency: milkdrift-blueprint"
        ]),
    );
    connections.insert("report_delay_ms".into(), json!(5000));
    write_private(
        &root.join("browser.json"),
        &serde_json::to_vec_pretty(&connections)?,
    )?;
    write_private(
        &root.join("ready.json"),
        &serde_json::to_vec_pretty(&json!({
            "daemon_binary":daemon,"daemon_source_binary":daemon_source,"fixture_binary":fixture,"fixture_source_binary":fixture_source,"root":root,
            "daemons":instances.iter().map(|instance|json!({"endpoint":instance.endpoint,"pid":instance.child.id()})).collect::<Vec<_>>(),
            "lifetime_secs":lifetime_secs,"browser_config":root.join("browser.json")
        }))?,
    )?;
    #[expect(
        clippy::print_stdout,
        reason = "The evidence executable reports the non-secret path used by the browser test driver."
    )]
    {
        println!(
            "Production browser fixture ready: {}",
            root.join("browser.json").display()
        );
    }
    let outcome = if let Some(cli) = cli {
        match tokio::time::timeout(
            Duration::from_secs(90),
            check::run(&root, cli, &daemon, &mut instances),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err("connected task scenario exceeded its 90-second deadline".into()),
        }
    } else {
        serve_loop(&root, &daemon, lifetime_secs, &mut instances)
            .await
            .map(|()| json!({"browser_fixture":"closed"}))
    };
    let mut cleanup = Vec::new();
    for instance in &mut instances {
        if let Err(error) = instance.child.shutdown() {
            cleanup.push(error.to_string());
        }
    }
    if let Err(error) = model.finish() {
        cleanup.push(error.to_string());
    }
    let report = match (outcome, cleanup.is_empty()) {
        (Ok(report), true) => report,
        (Err(error), true) => return Err(error),
        (Ok(_), false) => {
            return Err(format!("fixture cleanup unconfirmed: {}", cleanup.join("; ")).into());
        }
        (Err(error), false) => {
            return Err(format!(
                "{error}; fixture cleanup unconfirmed: {}",
                cleanup.join("; ")
            )
            .into());
        }
    };
    if cli.is_some() {
        write_private(
            &root.join("check.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}

async fn serve_loop(
    root: &Path,
    daemon: &Path,
    lifetime_secs: u64,
    instances: &mut [Daemon],
) -> EvidenceResult {
    let deadline = Instant::now() + Duration::from_secs(lifetime_secs);
    let control = root.join("control-request");
    while Instant::now() < deadline {
        if control.try_exists()? {
            let request = fs::read_to_string(&control)?;
            fs::remove_file(&control)?;
            if request == "stop" {
                break;
            }
            let (index, abrupt) = match request.as_str() {
                "restart-a" => (0, false),
                "restart-b" => (1, false),
                "crash-a" => (0, true),
                "crash-b" => (1, true),
                _ => return Err("unknown fixture lifecycle request".into()),
            };
            let instance = instances
                .get_mut(index)
                .ok_or("fixture daemon index absent")?;
            if abrupt {
                instance.child.terminate()?;
            } else {
                instance.child.shutdown()?;
            }
            instance.child = start(daemon, &instance.config, &instance.log)?;
            ready(instance).await?;
        }
        for instance in instances.iter_mut() {
            ensure(
                instance.child.try_wait()?.is_none(),
                "owned production daemon exited during browser fixture",
            )?;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
