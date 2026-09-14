//! Milkdrift local durable daemon executable.

use std::path::PathBuf;

use clap::Parser;
use milkdrift_daemon::{DaemonConfig, DaemonHost, serve};
use tracing_subscriber::EnvFilter;
mod storage_admin;

#[derive(Parser)]
#[command(
    name = "milkdrift-daemon",
    version,
    about = "Milkdrift local durable workflow daemon",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Arguments {
    /// Versioned TOML daemon configuration path.
    #[arg(long, env = "MILKDRIFT_DAEMON_CONFIG", required = true)]
    config: Option<PathBuf>,
    /// Validate and print redacted effective configuration, then exit.
    #[arg(long)]
    print_effective_config: bool,
    /// Validate configuration without starting the daemon.
    #[arg(long, conflicts_with = "print_effective_config")]
    check_config: bool,
    /// Serve authenticated recovery controls with execution and peer workers disabled.
    #[arg(long, conflicts_with_all = ["check_config", "print_effective_config"])]
    recovery: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Offline inspection/backup under OS file-owner authority; never starts a host.
    StorageAdmin(storage_admin::Arguments),
}

#[tokio::main]
async fn main() {
    let exit = match run().await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("milkdrift-daemon: {error}");
            1
        }
    };
    if exit != 0 {
        std::process::exit(exit);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    if let Some(Command::StorageAdmin(arguments)) = arguments.command {
        return storage_admin::run(arguments);
    }
    let config = DaemonConfig::load(arguments.config.as_ref().ok_or("--config is required")?)?;
    if arguments.print_effective_config {
        print!("{}", config.redacted_toml());
        return Ok(());
    }
    if arguments.check_config {
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .with_current_span(true)
        .with_span_list(false)
        .init();
    let bind = config.bind();
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let host = if arguments.recovery {
        DaemonHost::start_recovery(config)?
    } else {
        DaemonHost::start(config)?
    };
    serve(listener, host, async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(phase = "shutdown", outcome = "signal_error", "{error}");
        }
    })
    .await?;
    Ok(())
}
