#![forbid(unsafe_code)]

//! Cargo-native owner for Milkdrift's focused mutation-evidence campaigns.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

const CLASSIFICATION_PATH: &str = ".cargo/mutation-classifications.json";
const DEFAULT_JOBS: u32 = 2;

type ToolResult<T> = Result<T, ToolFailure>;

#[derive(Debug)]
struct ToolFailure {
    exit_code: u8,
    message: String,
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ToolFailure {}

impl ToolFailure {
    fn new(exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
        }
    }

    fn operational(message: impl Into<String>) -> Self {
        Self::new(1, message)
    }

    fn mutation_outcome(message: impl Into<String>) -> Self {
        Self::new(2, message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum MutationShard {
    Authority,
    Retention,
    Runtime,
    Uncertainty,
    Controller,
    Context,
    Peer,
}

#[cfg(test)]
const ALL_SHARDS: [MutationShard; 7] = [
    MutationShard::Authority,
    MutationShard::Retention,
    MutationShard::Runtime,
    MutationShard::Uncertainty,
    MutationShard::Controller,
    MutationShard::Context,
    MutationShard::Peer,
];

impl MutationShard {
    const fn name(self) -> &'static str {
        match self {
            Self::Authority => "authority",
            Self::Retention => "retention",
            Self::Runtime => "runtime",
            Self::Uncertainty => "uncertainty",
            Self::Controller => "controller",
            Self::Context => "context",
            Self::Peer => "peer",
        }
    }

    const fn specification(self) -> ShardSpecification {
        match self {
            Self::Authority => ShardSpecification {
                files: &[
                    "crates/authority/src/selection.rs",
                    "crates/authority/src/evaluator.rs",
                    "crates/authority/src/model/capability.rs",
                    "crates/authority/src/model/resource.rs",
                    "adapters/redb-store/src/peer.rs",
                    "adapters/redb-store/src/peer/validation.rs",
                ],
                pattern: "(Selection.*(matches|is_subset_of)|validate_count|GrantSetEvaluator.*evaluate|CapabilityAuthorityScope::is_subset_of|AuthorityBudget::fits_within|within|validate_admission)",
                test_packages: &[
                    "milkdrift-authority",
                    "milkdrift-peer-http",
                    "milkdrift-evidence",
                ],
                cargo_test_arguments: &[],
            },
            Self::Retention => ShardSpecification {
                files: &["adapters/redb-store/src/application.rs"],
                pattern: "(commit_application_command|archive_application_command_receipts|archive_oldest_hot_receipts|receipt_accounting_values|ReceiptLocation::status)",
                test_packages: &["milkdrift-redb-store", "milkdrift-evidence"],
                cargo_test_arguments: &[],
            },
            Self::Runtime => ShardSpecification {
                files: &[
                    "crates/runtime/src/engine/reconciliation.rs",
                    "crates/runtime/src/engine.rs",
                ],
                pattern: "(handle_new_command|replay_if_present|projection_checkpoint_due|plan_revision_adoption|plan_reconciliation_decision|plan_reconciliation_application)",
                test_packages: &["milkdrift-runtime"],
                cargo_test_arguments: &[
                    "--lib",
                    "--test",
                    "durable_runtime",
                    "--test",
                    "structured_runtime",
                ],
            },
            Self::Uncertainty => ShardSpecification {
                files: &[
                    "crates/runtime/src/engine/effects.rs",
                    "crates/runtime/src/engine/support.rs",
                ],
                pattern: "(recovery_classification|record_effect_uncertainty)",
                test_packages: &["milkdrift-runtime"],
                cargo_test_arguments: &["--test", "structured_runtime"],
            },
            Self::Controller => ShardSpecification {
                files: &[
                    "crates/control/src/controller/lifecycle.rs",
                    "crates/control/src/controller/policy.rs",
                ],
                pattern: "(ControllerPolicy::assess|ControllerLifecycleOwner.*(progress|assess)|bound_outcome)",
                test_packages: &["milkdrift-control"],
                cargo_test_arguments: &["--lib", "--test", "control_service"],
            },
            Self::Context => ShardSpecification {
                files: &["crates/runtime/src/context.rs"],
                pattern: "(CausalContextBuilder::build|budget_overflow)",
                test_packages: &["milkdrift-runtime"],
                cargo_test_arguments: &["--test", "causal_context"],
            },
            Self::Peer => ShardSpecification {
                files: &[
                    "adapters/redb-store/src/peer.rs",
                    "adapters/redb-store/src/peer/accounting.rs",
                    "adapters/redb-store/src/peer/retention.rs",
                    "adapters/redb-store/src/peer/validation.rs",
                ],
                pattern: "(admit_peer_execution|claim_peer_dispatch|mark_peer_entered|release_peer_claim|mark_peer_uncertain|append_peer_observation|request_peer_cancellation|acknowledge_peer_cancellation|recover_peer_claims|archive_peer_executions|release_active_accounting|validate_record|validate_tombstone)",
                test_packages: &["milkdrift-peer-http", "milkdrift-evidence"],
                cargo_test_arguments: &[],
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ShardSpecification {
    files: &'static [&'static str],
    pattern: &'static str,
    test_packages: &'static [&'static str],
    cargo_test_arguments: &'static [&'static str],
}

#[derive(Debug, Parser)]
#[command(
    name = "mutation-evidence",
    about = "Run one bounded Milkdrift mutation-evidence shard"
)]
struct Arguments {
    /// Semantic mutation area to list or execute.
    #[arg(value_enum)]
    shard: MutationShard,
    /// List the selected mutants without running the campaign.
    #[arg(long)]
    list: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Classification {
    Equivalent,
    UnreachableByValidContract,
    ToolLimitation,
}

impl Classification {
    const fn name(self) -> &'static str {
        match self {
            Self::Equivalent => "equivalent",
            Self::UnreachableByValidContract => "unreachable_by_valid_contract",
            Self::ToolLimitation => "tool_limitation",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClassificationEntry {
    mutant: String,
    classification: Classification,
    explanation: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationPolicy {
    survivors_require_exact_mutant_identity: bool,
    allowed_classifications: Vec<Classification>,
    unclassified_survivors_fail_the_lane: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationDocument {
    schema_version: u32,
    cargo_mutants_version: String,
    policy: ClassificationPolicy,
    classifications: Vec<ClassificationEntry>,
}

#[derive(Debug, Serialize)]
struct ClassificationReport {
    schema_version: u32,
    cargo_mutants_version: String,
    missed: Vec<ClassificationEntry>,
    timeouts: Vec<String>,
}

fn main() -> ExitCode {
    match execute(Arguments::parse()) {
        Ok(exit_code) => ExitCode::from(exit_code),
        Err(failure) => {
            eprintln!("mutation-evidence: {}", failure.message);
            ExitCode::from(failure.exit_code)
        }
    }
}

fn execute(arguments: Arguments) -> ToolResult<u8> {
    let repository = repository_root()?;
    let specification = arguments.shard.specification();
    validate_specification(&repository, specification)?;
    let output = mutation_output(arguments.shard)?;
    let output_parent = output
        .parent()
        .ok_or_else(|| ToolFailure::operational("mutation output directory must have a parent"))?;
    fs::create_dir_all(repository.join(output_parent)).map_err(|error| {
        ToolFailure::operational(format!("cannot create mutation output parent: {error}"))
    })?;

    let mut command = mutation_command(&repository, &output, specification)?;
    if arguments.list {
        command.arg("--list");
    }
    let status = command.status().map_err(|error| {
        ToolFailure::operational(format!("failed to start cargo-mutants: {error}"))
    })?;
    let status_code = status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1);
    if arguments.list || status_code != 2 {
        return Ok(status_code);
    }

    let mutants_output = resolve_path(&repository, &output).join("mutants.out");
    let classified =
        classify_mutation_outcomes(&mutants_output, &repository.join(CLASSIFICATION_PATH))?;
    for entry in classified {
        println!(
            "CLASSIFIED {}: {}",
            entry.classification.name(),
            entry.mutant
        );
    }
    Ok(0)
}

fn repository_root() -> ToolResult<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            ToolFailure::operational("evidence package must remain under tools/evidence")
        })
}

fn mutation_output(shard: MutationShard) -> ToolResult<PathBuf> {
    match env::var_os("CARGO_MUTANTS_OUTPUT") {
        Some(value) if value.is_empty() => Err(ToolFailure::operational(
            "CARGO_MUTANTS_OUTPUT must not be empty",
        )),
        Some(value) => Ok(PathBuf::from(value)),
        None => Ok(Path::new("target/mutation").join(shard.name())),
    }
}

fn mutation_jobs() -> ToolResult<u32> {
    let Some(value) = env::var_os("CARGO_MUTANTS_JOBS") else {
        return Ok(DEFAULT_JOBS);
    };
    let value = value
        .to_str()
        .ok_or_else(|| ToolFailure::operational("CARGO_MUTANTS_JOBS must be valid Unicode"))?;
    let jobs = value.parse::<u32>().map_err(|error| {
        ToolFailure::operational(format!(
            "CARGO_MUTANTS_JOBS must be a positive integer: {error}"
        ))
    })?;
    if jobs == 0 {
        return Err(ToolFailure::operational(
            "CARGO_MUTANTS_JOBS must be greater than zero",
        ));
    }
    Ok(jobs)
}

fn mutation_command(
    repository: &Path,
    output: &Path,
    specification: ShardSpecification,
) -> ToolResult<Command> {
    let cargo = env::var_os("CARGO")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command
        .current_dir(repository)
        .arg("mutants")
        .arg("--workspace")
        .arg("--output")
        .arg(output)
        .arg("--jobs")
        .arg(mutation_jobs()?.to_string())
        .arg("--re")
        .arg(specification.pattern)
        .arg("--baseline")
        .arg("run")
        .arg("--build-timeout")
        .arg("180")
        .arg("--no-shuffle");
    for file in specification.files {
        command.arg("--file").arg(file);
    }
    for package in specification.test_packages {
        command.arg("--test-package").arg(package);
    }
    for argument in specification.cargo_test_arguments {
        command.arg("--cargo-test-arg").arg(argument);
    }
    Ok(command)
}

fn validate_specification(repository: &Path, specification: ShardSpecification) -> ToolResult<()> {
    if specification.files.is_empty()
        || specification.pattern.is_empty()
        || specification.test_packages.is_empty()
    {
        return Err(ToolFailure::operational(
            "mutation shard specification must be complete",
        ));
    }
    for file in specification.files {
        if !repository.join(file).is_file() {
            return Err(ToolFailure::operational(format!(
                "mutation shard source does not exist: {file}"
            )));
        }
    }
    Ok(())
}

fn classify_mutation_outcomes(
    output_directory: &Path,
    classification_path: &Path,
) -> ToolResult<Vec<ClassificationEntry>> {
    let timeouts = read_nonempty_lines(&output_directory.join("timeout.txt"))?;
    if !timeouts.is_empty() {
        return Err(ToolFailure::mutation_outcome(format!(
            "mutation lane has {} timeout(s); classifications cannot hide timeouts",
            timeouts.len()
        )));
    }
    let missed = read_nonempty_lines(&output_directory.join("missed.txt"))?;
    if missed.is_empty() {
        return Err(ToolFailure::mutation_outcome(
            "cargo-mutants failed without a classifiable missed mutant",
        ));
    }

    let bytes = fs::read(classification_path).map_err(|error| {
        ToolFailure::mutation_outcome(format!(
            "cannot read {}: {error}",
            classification_path.display()
        ))
    })?;
    let document: ClassificationDocument = serde_json::from_slice(&bytes).map_err(|error| {
        ToolFailure::mutation_outcome(format!("invalid mutation classification document: {error}"))
    })?;
    let accepted = validate_classification_document(&document)?;

    let mut classified = Vec::with_capacity(missed.len());
    let mut unexplained = Vec::new();
    for mutant in &missed {
        if let Some(entry) = accepted.get(mutant.as_str()) {
            classified.push((*entry).clone());
        } else {
            unexplained.push(mutant);
        }
    }
    if !unexplained.is_empty() {
        let mut message = String::from("unclassified surviving mutants:");
        for mutant in unexplained {
            message.push_str("\n- ");
            message.push_str(mutant);
        }
        return Err(ToolFailure::mutation_outcome(message));
    }

    let report = ClassificationReport {
        schema_version: 1,
        cargo_mutants_version: document.cargo_mutants_version,
        missed: classified.clone(),
        timeouts,
    };
    let mut report_bytes = serde_json::to_vec_pretty(&report).map_err(|error| {
        ToolFailure::mutation_outcome(format!("cannot encode classification report: {error}"))
    })?;
    report_bytes.push(b'\n');
    fs::write(
        output_directory.join("classification-report.json"),
        report_bytes,
    )
    .map_err(|error| {
        ToolFailure::mutation_outcome(format!("cannot write classification report: {error}"))
    })?;
    Ok(classified)
}

fn validate_classification_document(
    document: &ClassificationDocument,
) -> ToolResult<BTreeMap<&str, &ClassificationEntry>> {
    if document.schema_version != 1 {
        return Err(ToolFailure::mutation_outcome(format!(
            "unsupported mutation classification schema {}",
            document.schema_version
        )));
    }
    if document.cargo_mutants_version.trim().is_empty() {
        return Err(ToolFailure::mutation_outcome(
            "mutation classification document has no cargo-mutants version",
        ));
    }
    if !document.policy.survivors_require_exact_mutant_identity
        || !document.policy.unclassified_survivors_fail_the_lane
    {
        return Err(ToolFailure::mutation_outcome(
            "mutation classification policy must require exact identities and reject unclassified survivors",
        ));
    }
    let allowed: BTreeSet<_> = document
        .policy
        .allowed_classifications
        .iter()
        .copied()
        .collect();
    if allowed.len() != document.policy.allowed_classifications.len() || allowed.is_empty() {
        return Err(ToolFailure::mutation_outcome(
            "allowed mutation classifications must be unique and nonempty",
        ));
    }

    let mut accepted = BTreeMap::new();
    for entry in &document.classifications {
        if entry.mutant.trim().is_empty() {
            return Err(ToolFailure::mutation_outcome(
                "mutation classification has an empty mutant identity",
            ));
        }
        if !allowed.contains(&entry.classification) {
            return Err(ToolFailure::mutation_outcome(format!(
                "invalid classification '{}' for {}",
                entry.classification.name(),
                entry.mutant
            )));
        }
        if entry.explanation.trim().len() < 20 {
            return Err(ToolFailure::mutation_outcome(format!(
                "classification explanation is missing or too short for {}",
                entry.mutant
            )));
        }
        if accepted.insert(entry.mutant.as_str(), entry).is_some() {
            return Err(ToolFailure::mutation_outcome(format!(
                "duplicate mutation classification for {}",
                entry.mutant
            )));
        }
    }
    Ok(accepted)
}

fn read_nonempty_lines(path: &Path) -> ToolResult<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path).map_err(|error| {
        ToolFailure::mutation_outcome(format!("cannot read {}: {error}", path.display()))
    })?;
    Ok(contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn resolve_path(repository: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn arguments_accept_exact_shards_and_optional_list_mode() -> TestResult {
        let authority = Arguments::try_parse_from(["mutation-evidence", "authority"])?;
        assert_eq!(authority.shard, MutationShard::Authority);
        assert!(!authority.list);
        let peer = Arguments::try_parse_from(["mutation-evidence", "peer", "--list"])?;
        assert_eq!(peer.shard, MutationShard::Peer);
        assert!(peer.list);
        assert!(Arguments::try_parse_from(["mutation-evidence"]).is_err());
        assert!(Arguments::try_parse_from(["mutation-evidence", "unknown"]).is_err());
        assert!(Arguments::try_parse_from(["mutation-evidence", "peer", "--run"]).is_err());
        Ok(())
    }

    #[test]
    fn every_shard_names_existing_sources_after_module_splits() -> TestResult {
        let repository = repository_root()?;
        for shard in ALL_SHARDS {
            validate_specification(&repository, shard.specification())?;
        }
        let controller = MutationShard::Controller.specification();
        assert!(
            controller
                .files
                .contains(&"crates/control/src/controller/lifecycle.rs")
        );
        assert!(
            controller
                .files
                .contains(&"crates/control/src/controller/policy.rs")
        );
        let authority = MutationShard::Authority.specification();
        assert!(
            authority
                .files
                .contains(&"crates/authority/src/model/resource.rs")
        );
        let peer = MutationShard::Peer.specification();
        assert!(
            peer.files
                .contains(&"adapters/redb-store/src/peer/validation.rs")
        );
        Ok(())
    }

    #[test]
    fn accepted_survivors_write_the_canonical_report() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let output = temporary.path().join("mutants.out");
        fs::create_dir_all(&output)?;
        fs::write(output.join("missed.txt"), "mutant-a\n")?;
        let classifications = temporary.path().join("classifications.json");
        fs::write(
            &classifications,
            classification_document("mutant-a", "equivalent", valid_explanation()),
        )?;

        let matched = classify_mutation_outcomes(&output, &classifications)?;
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].mutant, "mutant-a");
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("classification-report.json"))?)?;
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["cargo_mutants_version"], "27.1.0");
        assert_eq!(report["missed"][0]["mutant"], "mutant-a");
        assert_eq!(report["timeouts"], serde_json::json!([]));
        Ok(())
    }

    #[test]
    fn timeouts_and_unclassified_survivors_fail_closed() -> TestResult {
        let temporary = tempfile::tempdir()?;
        let output = temporary.path().join("mutants.out");
        fs::create_dir_all(&output)?;
        fs::write(output.join("timeout.txt"), "timed-out-mutant\n")?;
        let classifications = temporary.path().join("classifications.json");
        fs::write(
            &classifications,
            classification_document("known-mutant", "equivalent", valid_explanation()),
        )?;
        let timeout = classify_mutation_outcomes(&output, &classifications);
        assert!(timeout.is_err());

        fs::write(output.join("timeout.txt"), "")?;
        fs::write(output.join("missed.txt"), "unknown-mutant\n")?;
        let unclassified = classify_mutation_outcomes(&output, &classifications);
        assert!(unclassified.is_err());
        assert!(!output.join("classification-report.json").exists());
        Ok(())
    }

    #[test]
    fn malformed_policy_duplicate_identity_and_short_explanation_are_rejected() -> TestResult {
        let short: ClassificationDocument = serde_json::from_str(&classification_document(
            "mutant-a",
            "equivalent",
            "too short",
        ))?;
        assert!(validate_classification_document(&short).is_err());

        let duplicate = format!(
            "{{\"schema_version\":1,\"cargo_mutants_version\":\"27.1.0\",\"policy\":{},\"classifications\":[{},{}]}}",
            valid_policy(),
            classification_entry("mutant-a", "equivalent", valid_explanation()),
            classification_entry("mutant-a", "tool_limitation", valid_explanation())
        );
        let duplicate: ClassificationDocument = serde_json::from_str(&duplicate)?;
        assert!(validate_classification_document(&duplicate).is_err());

        let disabled = "{\"schema_version\":1,\"cargo_mutants_version\":\"27.1.0\",\"policy\":{\"survivors_require_exact_mutant_identity\":false,\"allowed_classifications\":[\"equivalent\"],\"unclassified_survivors_fail_the_lane\":true},\"classifications\":[]}";
        let disabled: ClassificationDocument = serde_json::from_str(disabled)?;
        assert!(validate_classification_document(&disabled).is_err());
        Ok(())
    }

    fn classification_document(mutant: &str, classification: &str, explanation: &str) -> String {
        format!(
            "{{\"schema_version\":1,\"cargo_mutants_version\":\"27.1.0\",\"policy\":{},\"classifications\":[{}]}}",
            valid_policy(),
            classification_entry(mutant, classification, explanation)
        )
    }

    fn valid_policy() -> &'static str {
        "{\"survivors_require_exact_mutant_identity\":true,\"allowed_classifications\":[\"equivalent\",\"unreachable_by_valid_contract\",\"tool_limitation\"],\"unclassified_survivors_fail_the_lane\":true}"
    }

    fn classification_entry(mutant: &str, classification: &str, explanation: &str) -> String {
        serde_json::json!({
            "mutant": mutant,
            "classification": classification,
            "explanation": explanation,
        })
        .to_string()
    }

    fn valid_explanation() -> &'static str {
        "This exact mutant is behaviorally equivalent by construction."
    }
}
