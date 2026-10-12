//! Release-mode operational evidence runner emitting JSON and CSV artifacts.

use std::io::Write as _;
use std::{fs, path::PathBuf, process::Command};

use milkdrift_evidence::{
    DEFAULT_OPERATION_COUNT, EvidenceResult, artifact_range_read, daemon_owner_round_trip,
    explicit_input_materialization, explicit_input_selection, local_process_stream_drain,
    measure_daemon_saturation, measure_storage_growth, model_stream_parsers,
    peer_observation_paths, workflow_commit_batch, workflow_commit_one, workflow_read_page,
    workflow_receipt_paths, workflow_snapshot_reopen,
};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "Final process-boundary diagnostic after the evidence operation has failed; reporting cannot turn it into success."
            )]
            {
                eprintln!("operational evidence failed: {error}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> EvidenceResult {
    let mut output = PathBuf::from("target/evidence");
    let mut operations = DEFAULT_OPERATION_COUNT;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                output = PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| std::io::Error::other("--output requires a directory"))?,
                );
            }
            "--operations" => {
                operations = arguments
                    .next()
                    .ok_or_else(|| std::io::Error::other("--operations requires a count"))?
                    .parse()?;
            }
            _ => return Err(std::io::Error::other(format!("unknown argument {argument}")).into()),
        }
    }

    fs::create_dir_all(&output)?;
    let git_commit = command_stdout("git", &["rev-parse", "HEAD"])?;
    let git_tree = command_stdout("git", &["rev-parse", "HEAD^{tree}"])?;
    let source_dirty = !command_stdout("git", &["status", "--porcelain=v1"])?.is_empty();
    let rustc_verbose = command_stdout("rustc", &["-vV"])?;
    let scenarios = vec![
        workflow_commit_one()?,
        workflow_commit_batch()?,
        workflow_read_page()?,
        workflow_snapshot_reopen()?,
        workflow_receipt_paths()?,
        peer_observation_paths()?,
        explicit_input_selection()?,
        explicit_input_materialization()?,
        artifact_range_read()?,
        local_process_stream_drain()?,
        model_stream_parsers()?,
        daemon_owner_round_trip()?,
    ];
    let storage = measure_storage_growth(operations)?;
    let daemon = measure_daemon_saturation(operations)?;
    let document = serde_json::json!({
        "schema_version": 1,
        "build": {
            "target_os": std::env::consts::OS,
            "target_arch": std::env::consts::ARCH,
            "git_commit": git_commit,
            "git_tree": git_tree,
            "source_dirty": source_dirty,
            "rustc_verbose": rustc_verbose,
        },
        "parameters": { "operations": operations },
        "scenarios": scenarios,
        "storage": storage,
        "daemon": daemon,
    });
    fs::write(
        output.join("operational-evidence.json"),
        serde_json::to_vec_pretty(&document)?,
    )?;
    let mut csv = String::from("scenario,operations,bytes,checksum\n");
    for scenario in &scenarios {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            scenario.scenario, scenario.operations, scenario.bytes, scenario.checksum
        ));
    }
    fs::write(output.join("scenario-summary.csv"), csv)?;
    writeln!(
        std::io::stdout().lock(),
        "{}",
        output.join("operational-evidence.json").display()
    )?;
    Ok(())
}

fn command_stdout(program: &str, arguments: &[&str]) -> EvidenceResult<String> {
    let output = Command::new(program).args(arguments).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "{program} {} failed with {}",
            arguments.join(" "),
            output.status
        ))
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
