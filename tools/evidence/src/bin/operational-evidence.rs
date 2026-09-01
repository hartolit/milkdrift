//! Release-mode operational evidence runner emitting JSON and CSV artifacts.

use std::{fs, path::PathBuf, process::Command};

use milkdrift_evidence::{
    DEFAULT_OPERATION_COUNT, EvidenceResult, application_receipt_paths, artifact_range_read,
    context_discovery_and_selection, context_materialization, daemon_owner_round_trip,
    journal_append_batch, journal_append_one, local_process_stream_drain,
    measure_daemon_saturation, measure_storage_growth, model_stream_parsers,
    peer_observation_paths, projection_rebuild, projection_snapshot_tail,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("operational evidence failed: {error}");
        std::process::exit(1);
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
        journal_append_one()?,
        journal_append_batch()?,
        projection_rebuild()?,
        projection_snapshot_tail()?,
        application_receipt_paths()?,
        peer_observation_paths()?,
        context_discovery_and_selection()?,
        context_materialization()?,
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
    println!("{}", output.join("operational-evidence.json").display());
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
