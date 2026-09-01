//! Milkdrift local durable daemon executable.

use std::path::PathBuf;

use clap::Parser;
use milkdrift_daemon::{DaemonConfig, DaemonHost, serve};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "milkdrift-daemon",
    version,
    about = "Milkdrift local durable workflow daemon"
)]
struct Arguments {
    /// Versioned JSON daemon configuration path.
    #[arg(long, env = "MILKDRIFT_DAEMON_CONFIG")]
    config: PathBuf,
    /// Validate and print redacted effective configuration, then exit.
    #[arg(long)]
    print_effective_config: bool,
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
    let config = DaemonConfig::load(&arguments.config)?;
    if arguments.print_effective_config {
        println!(
            "{}",
            serde_json::to_string_pretty(&config.document.redacted_json()?)?
        );
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
    let bind = config.document.bind;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let host = DaemonHost::start(config)?;
    serve(listener, host, async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(phase = "shutdown", outcome = "signal_error", "{error}");
        }
    })
    .await?;
    Ok(())
}
