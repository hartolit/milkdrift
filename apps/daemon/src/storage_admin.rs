//! Explicit OS-owner operations, outside runtime/configuration/credential composition.
mod diagnostics;
mod scan;

use clap::{Args, Subcommand, ValueEnum};
use milkdrift_persistence::{PageSize, RunSummaryCursor, RunSummaryFilter, RunSummaryPageQuery};
use milkdrift_redb_store::offline::{BackupProducer, InspectionFamily, OfflineStore};
use milkdrift_workspace::RunId;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub(super) struct Arguments {
    /// Existing stopped data root; no schema or missing root is initialized.
    #[arg(long)]
    root: PathBuf,
    /// Existing private directory outside the source for the temporary database copy.
    #[arg(long)]
    scratch: PathBuf,
    #[command(subcommand)]
    operation: Operation,
}

#[derive(Subcommand)]
enum Operation {
    /// Schema, durable clock and table counts; no whole-history logical scan.
    Overview,
    /// Page through retained run discovery summaries.
    Runs {
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=128))]
        limit: u32,
        #[arg(long)]
        after: Option<String>,
    },
    /// Inspect one run's retained attempt frontier by replaying up to the explicit event bound.
    Run {
        #[arg(long)]
        run: String,
        #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
        maximum_events: u32,
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=128))]
        limit: u32,
        #[arg(long)]
        after_attempt: Option<String>,
    },
    /// Bounded redacted lease/account/receipt/peer/artifact records.
    Records {
        #[arg(long, value_enum)]
        family: Family,
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=128))]
        limit: u32,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        hash_artifacts: bool,
    },
    /// Explicit resumable integrity scan; choose artifact hashing deliberately.
    Scan {
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u32).range(1..=128))]
        limit: u32,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        hash_artifacts: bool,
    },
    /// Copy a complete stopped generation and verify all files/current readers.
    Backup {
        #[arg(long)]
        destination: PathBuf,
    },
    /// Verify a completed backup's exact inventory, file digests and current readers.
    VerifyBackup,
    /// Verify then restore into a new, inspection-only directory.
    Restore {
        #[arg(long)]
        destination: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Family {
    Leases,
    Accounts,
    HotReceipts,
    ColdReceipts,
    Peers,
    PeerTombstones,
    Artifacts,
    Managed,
}
impl From<Family> for InspectionFamily {
    fn from(value: Family) -> Self {
        match value {
            Family::Leases => Self::Leases,
            Family::Accounts => Self::Accounts,
            Family::HotReceipts => Self::HotReceipts,
            Family::ColdReceipts => Self::ColdReceipts,
            Family::Peers => Self::Peers,
            Family::PeerTombstones => Self::PeerTombstones,
            Family::Artifacts => Self::Artifacts,
            Family::Managed => Self::Managed,
        }
    }
}

pub(super) fn run(arguments: Arguments) -> Result<()> {
    let Arguments {
        root,
        scratch,
        operation,
    } = arguments;
    let report = match operation {
        Operation::VerifyBackup => backup_report(OfflineStore::verify_backup(&root, &scratch)?),
        Operation::Restore { destination } => {
            backup_report(OfflineStore::restore(&root, &destination, &scratch)?)
        }
        operation => {
            let store = OfflineStore::open(&root, &scratch)?;
            let data = match operation {
                Operation::Overview => serde_json::to_value(store.overview()?)?,
                Operation::Runs { limit, after } => {
                    let filter = RunSummaryFilter::default();
                    let cursor = after
                        .map(|id| {
                            RunId::new(id)
                                .map(|run| RunSummaryCursor::for_query(run, filter.clone()))
                        })
                        .transpose()?;
                    let page = store.queries().run_summaries(&RunSummaryPageQuery {
                        filter,
                        cursor,
                        limit: PageSize::new(limit)?,
                    })?;
                    json!({"runs": page.runs, "next": page.next.map(|c| c.after_run().to_string()),
                        "scope": "derived discovery; use explicit integrity scan to verify all indexes"})
                }
                Operation::Run {
                    run,
                    maximum_events,
                    limit,
                    after_attempt,
                } => diagnostics::run(
                    &store,
                    RunId::new(run)?,
                    maximum_events,
                    limit,
                    after_attempt.as_deref(),
                )?,
                Operation::Records {
                    family,
                    limit,
                    after,
                    hash_artifacts,
                } => serde_json::to_value(store.inspect(
                    family.into(),
                    PageSize::new(limit)?,
                    after.as_deref(),
                    hash_artifacts,
                )?)?,
                Operation::Scan {
                    limit,
                    after,
                    hash_artifacts,
                } => scan::page(&store, limit, after.as_deref(), hash_artifacts)?,
                Operation::Backup { destination } => {
                    backup_report(store.backup(&destination, &scratch, producer()?)?)
                }
                Operation::VerifyBackup | Operation::Restore { .. } => {
                    return Err("invalid maintenance dispatch".into());
                }
            };
            json!({"source_database_blake3": store.database_digest(), "data": data})
        }
    };
    let bytes =
        serde_json::to_vec_pretty(&json!({"schema_version": 1, "kind": "inspection_report",
        "execution_authorized": false, "report": report}))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("diagnostic report exceeds 8 MiB; select a smaller page".into());
    }
    std::io::stdout().lock().write_all(&bytes)?;
    println!();
    Ok(())
}

fn backup_report(manifest: milkdrift_redb_store::offline::BackupManifest) -> Value {
    json!({"copy_complete": manifest.complete, "source_database_blake3": manifest.source_database_digest,
        "producer": manifest.producer, "storage_schema": manifest.storage_schema, "document_format": manifest.document_format,
        "clock_high_water_unix_ms": manifest.clock_high_water_unix_ms,
        "documents_checked": manifest.documents_checked, "artifacts_checked": manifest.artifacts_checked,
        "integrity_failures": manifest.integrity_failures,
        "disposition": "preserved inspection-only generation; unresolved effects remain unresolved"})
}

fn producer() -> Result<BackupProducer> {
    let mut file = std::fs::File::open(std::env::current_exe()?)?;
    let mut digest = blake3::Hasher::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(BackupProducer {
        version: env!("CARGO_PKG_VERSION").into(),
        binary_digest: digest.finalize().to_hex().to_string(),
        source_revision: option_env!("MILKDRIFT_SOURCE_REVISION")
            .unwrap_or("unavailable; binary digest is authoritative producer identity")
            .into(),
    })
}
