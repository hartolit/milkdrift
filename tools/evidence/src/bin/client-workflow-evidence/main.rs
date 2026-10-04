//! Retain an ordinary authored workflow journey through actual daemon and CLI binaries.
mod fixture;
mod journey;
mod setup;

use clap::Parser;
use milkdrift_evidence::{
    EvidenceResult,
    application::{
        CliRunner, ensure, hash_file, path_text, run_command, start_daemon, wait_for_readiness,
        write_private,
    },
};
use serde_json::{Value, json};
use std::{fs, io::Write as _, path::PathBuf, process::Command, time::Duration};

#[derive(Parser)]
struct Arguments {
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    daemon: PathBuf,
    #[arg(long)]
    cli: PathBuf,
    /// An explicitly authorized attached loopback endpoint; omission selects controlled responses.
    #[arg(long)]
    model_profile: Option<PathBuf>,
}

struct Journey {
    cli: CliRunner,
    root: PathBuf,
}

impl Journey {
    fn call(&self, args: &[&str]) -> EvidenceResult<Value> {
        self.command(args, true)
    }

    fn refuse(&self, args: &[&str]) -> EvidenceResult<Value> {
        self.command(args, false)
    }

    fn command(&self, args: &[&str], success: bool) -> EvidenceResult<Value> {
        let output = self.cli.run(args, None)?;
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("commands.jsonl"))?;
        ensure(
            log.metadata()?.len() < 16 * 1024 * 1024,
            "command transcript bound exceeded",
        )?;
        serde_json::to_writer(
            &mut log,
            &json!({"arguments":args,"exit":output.status.code(),
            "stdout":output.stdout,"stderr":output.stderr}),
        )?;
        writeln!(log)?;
        ensure(
            output.status.success() == success,
            &format!(
                "unexpected CLI outcome for {args:?}: {} {}",
                output.stdout, output.stderr
            ),
        )?;
        output.final_json()
    }

    fn file(&self, name: &str) -> EvidenceResult<String> {
        Ok(path_text(&self.root.join(name))?.to_owned())
    }

    fn retain(&self, name: &str, value: &Value) -> EvidenceResult {
        write_private(&self.root.join(name), &serde_json::to_vec_pretty(value)?)?;
        Ok(())
    }
}

fn main() -> EvidenceResult {
    run(Arguments::parse())
}

fn run(args: Arguments) -> EvidenceResult {
    ensure(!args.output.exists(), "choose a new evidence directory")?;
    fs::create_dir_all(&args.output)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&args.output, fs::Permissions::from_mode(0o700))?;
    }
    let root = args.output.canonicalize()?;
    let daemon = args.daemon.canonicalize()?;
    let cli = args.cli.canonicalize()?;
    let git = |arguments: &[&str]| -> EvidenceResult<String> {
        let output = run_command(
            Command::new("git").args(arguments),
            None,
            Duration::from_secs(10),
        )?;
        ensure(output.status.success(), "Git provenance unavailable")?;
        Ok(output.stdout.trim().to_owned())
    };
    write_private(
        &root.join("provenance.json"),
        &serde_json::to_vec_pretty(&json!({
            "commit":git(&["rev-parse","HEAD"])?,"edits":git(&["status","--short"])?,
            "daemon":hash_file(&daemon)?,"cli":hash_file(&cli)?,
            "mode":if args.model_profile.is_some(){"authorized_attached_model"}else{"controlled_fixture"},
            "live_budget":{"requests":4,"maximum_output_tokens_per_request":1024,"request_timeout_seconds":180,"automatic_retries":0},
            "cleanup":"Only the evidence daemon and controlled endpoint are stopped; definitions, requests, outputs and store are retained."
        }))?,
    )?;
    let mut fixture = if args.model_profile.is_none() {
        Some(fixture::Model::start(&root)?)
    } else {
        None
    };
    let (runner, config) = setup::configure(
        &root,
        &daemon,
        cli,
        args.model_profile.as_deref(),
        fixture.as_ref(),
    )?;
    let journey = Journey { cli: runner, root };
    let mut child = start_daemon(&daemon, &config)?;
    let result: EvidenceResult = (|| {
        wait_for_readiness(&journey.cli, &mut child)?;
        let revision = journey::author(&journey)?;
        journey::briefs(&journey, &revision)?;
        child.terminate()?;
        child = start_daemon(&daemon, &config)?;
        wait_for_readiness(&journey.cli, &mut child)?;
        journey::replay_and_copy(&journey, &revision)?;
        if fixture.is_some() {
            journey::repair(&journey, &revision)?;
        }
        Ok(())
    })();
    let daemon_cleanup = child.terminate();
    let fixture_cleanup = fixture.as_mut().map(fixture::Model::finish).transpose();
    let count = fixture.as_ref().map(fixture::Model::count);
    let count_result = count.map_or(Ok(()), |count| {
        ensure(
            count == 7,
            "replay or repair made an unexpected provider entry",
        )
    });
    let passed =
        result.is_ok() && daemon_cleanup.is_ok() && fixture_cleanup.is_ok() && count_result.is_ok();
    journey.retain("report.json", &json!({"passed":passed,"scenario":format!("{result:?}"),
        "daemon_cleanup":format!("{daemon_cleanup:?}"),"fixture_cleanup":format!("{fixture_cleanup:?}"),
        "fixture_requests":count,"fixture_count_check":format!("{count_result:?}"),"live_requests_planned":if fixture.is_none(){4}else{0},
        "editorial_quality":"Requires separate review of the retained final text; completeness is checked by the product."}))?;
    result?;
    daemon_cleanup?;
    fixture_cleanup?;
    count_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_evidence::application::application_binary;

    #[test]
    fn maintained_briefs_match_the_operator_acceptance_cases() -> EvidenceResult {
        let examples =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/operator/release-notes");
        assert_eq!(
            fs::read_to_string(examples.join("harbor-brief.txt"))?.trim(),
            fixture::HARBOR_BRIEF
        );
        assert_eq!(
            fs::read_to_string(examples.join("lantern-brief.txt"))?.trim(),
            fixture::LANTERN_BRIEF
        );
        Ok(())
    }

    #[test]
    fn authored_workflow_runs_both_briefs_replays_copies_and_repairs() -> EvidenceResult {
        let directory = tempfile::tempdir()?;
        run(Arguments {
            output: directory.path().join("journey"),
            daemon: application_binary("milkdrift-daemon")?,
            cli: application_binary("milkdrift")?,
            model_profile: None,
        })
    }
}
