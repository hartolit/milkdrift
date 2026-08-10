use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::workspace::{benchmark_inventory, load_metadata};

/// One exact Cargo invocation generated for a maintained benchmark target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoCommand {
    arguments: Vec<String>,
}

impl CargoCommand {
    fn benchmark(package: &str, target: &str) -> Self {
        Self {
            arguments: [
                "bench", "--locked", "-p", package, "--bench", target, "--no-run",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    /// Arguments passed to Cargo, beginning with the Cargo subcommand.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

/// An error that prevents generation of the exact maintained-benchmark command plan.
#[derive(Debug)]
pub struct BenchmarkPlanError {
    message: String,
    source: Option<cargo_metadata::Error>,
}

impl fmt::Display for BenchmarkPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for BenchmarkPlanError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

/// Loads locked Cargo metadata and returns sorted exact commands for maintained bench targets.
///
/// Each command has the form `cargo bench --locked -p PACKAGE --bench TARGET --no-run`; command
/// generation never uses `--workspace` and fails if the owning-package registry is inconsistent
/// with Cargo's actual bench targets.
///
/// # Errors
///
/// Returns an error if locked metadata cannot be loaded or the bidirectional benchmark registry is
/// invalid.
pub fn benchmark_command_plan(
    manifest_path: &Path,
) -> Result<Vec<CargoCommand>, BenchmarkPlanError> {
    let metadata = load_metadata(manifest_path, true).map_err(|error| BenchmarkPlanError {
        message: format!("could not load locked Cargo metadata for benchmark planning: {error}"),
        source: Some(error),
    })?;
    let inventory = benchmark_inventory(&metadata).map_err(|issues| BenchmarkPlanError {
        message: issues
            .into_iter()
            .map(|issue| format!("{} / {}: {}", issue.package, issue.target, issue.reason))
            .collect::<Vec<_>>()
            .join("; "),
        source: None,
    })?;

    Ok(inventory
        .into_iter()
        .map(|benchmark| CargoCommand::benchmark(benchmark.package(), benchmark.target()))
        .collect())
}
