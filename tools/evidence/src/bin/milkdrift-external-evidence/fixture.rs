//! Byte-pinned native helper operations used by the generated process profiles.
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, run_command},
};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path, process::Command, time::Duration};

fn checkpoint(git: &str) -> EvidenceResult<String> {
    let result = run_command(
        Command::new(git).args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ]),
        None,
        Duration::from_secs(10),
    )?;
    ensure(result.status.success(), "Git inventory failed")?;
    let names = result
        .stdout
        .split('\0')
        .filter(|v| !v.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    ensure(names.len() <= 64, "fixture repository inventory overflow")?;
    let mut state = Vec::new();
    for name in names {
        let mut bytes = Vec::new();
        fs::File::open(name)?
            .take(1_048_577)
            .read_to_end(&mut bytes)?;
        ensure(bytes.len() <= 1_048_576, "fixture source exceeds bound")?;
        state.push((name, blake3::hash(&bytes).to_hex().to_string()));
    }
    Ok(format!("b3_{}", blake3::hash(&serde_json::to_vec(&state)?)))
}
pub(super) fn run(arguments: Vec<String>) -> EvidenceResult {
    match arguments.first().map(String::as_str) {
        Some("agent") => {
            let path = Path::new("calculator.rs");
            let source = fs::read_to_string(path)?;
            if source.contains("a - b") {
                fs::write(path, source.replace("a - b", "a + b"))?;
            }
            // Exercise the same build-output discipline requested of a real agent. Its binary
            // must not become source-checkpoint input for the independent verifier.
            fs::create_dir_all("target")?;
            let binary = Path::new("target")
                .join(format!("calculator-tests{}", std::env::consts::EXE_SUFFIX));
            let compile = run_command(
                Command::new(arguments.get(1).ok_or("compiler absent")?)
                    .args(["--test", "test_calculator.rs", "-o"])
                    .arg(&binary),
                None,
                Duration::from_secs(30),
            )?;
            ensure(compile.status.success(), "fixture test compilation failed")?;
            let tests = run_command(&mut Command::new(binary), None, Duration::from_secs(10))?;
            ensure(tests.status.success(), "fixture test failed")?;
            println!("fixture coding process inspected and repaired calculator.rs");
            let mut input = Vec::new();
            std::io::stdin().take(65_537).read_to_end(&mut input)?;
            ensure(input.len() <= 65_536, "fixture prompt exceeds bound")
        }
        Some("verify") => verify(&arguments[1..]),
        Some("review") => {
            let root = Path::new(arguments.get(1).ok_or("review root absent")?);
            fs::write(
                root.join("review.json"),
                serde_json::to_vec(
                    &json!({"schema_version":1,"independent_process":true,"context_manifest_observed":root.join("context/manifest.json").exists(),"finding":"controlled verifier gate requires remediation workflow"}),
                )?,
            )?;
            fs::write(
                root.join("remediation-proposal.json"),
                serde_json::to_vec(
                    &json!({"schema_version":1,"action":"rerun coding and independent verification"}),
                )?,
            )?;
            Ok(())
        }
        Some("evidence") => {
            let root = Path::new(arguments.get(1).ok_or("evidence root absent")?);
            let payload: Value =
                serde_json::from_slice(&fs::read(root.join("inputs/payload.json"))?)?;
            fs::write(
                root.join("evidence.txt"),
                payload.as_str().ok_or("evidence payload must be text")?,
            )?;
            Ok(())
        }
        _ => Err("unknown native fixture operation".into()),
    }
}
fn verify(arguments: &[String]) -> EvidenceResult {
    let [mode, git, rustc, root] = arguments else {
        return Err("verifier expects mode, Git, rustc and execution root".into());
    };
    let root = Path::new(root);
    let before = checkpoint(git)?;
    let diff = run_command(
        Command::new(git).args(["diff", "--binary", "HEAD"]),
        None,
        Duration::from_secs(10),
    )?;
    let test_binary = root.join(format!("calculator-tests{}", std::env::consts::EXE_SUFFIX));
    let compile = run_command(
        Command::new(rustc)
            .args(["--edition=2024", "--test", "test_calculator.rs", "-o"])
            .arg(&test_binary),
        None,
        Duration::from_secs(30),
    )?;
    let tests = if compile.status.success() {
        Some(run_command(
            &mut Command::new(&test_binary),
            None,
            Duration::from_secs(10),
        )?)
    } else {
        None
    };
    let successful = tests.as_ref().is_some_and(|r| r.status.success());
    let after = checkpoint(git)?;
    let mut log = format!(
        "ORCHESTRATION_FAULT_INJECTION={}\n{}{}",
        mode == "weak",
        compile.stdout,
        compile.stderr
    );
    if let Some(result) = &tests {
        log.push_str(&result.stdout);
        log.push_str(&result.stderr);
    }
    fs::write(root.join("verification.log"), log)?;
    let coding = if diff.stdout.is_empty() {
        json!({"type":"no_change","justification":"Independent Rust tests verify the requested calculator behavior at the unchanged checkpoint."})
    } else {
        json!({"type":"changed"})
    };
    fs::write(
        root.join("verification-result.json"),
        serde_json::to_vec(
            &json!({"checkpoint":before,"checked_checkpoint":after,"checks":{"rust.tests":mode=="good" && successful,"git.diff":diff.status.success()},"coding":coding}),
        )?,
    )?;
    ensure(
        diff.status.success() && successful,
        "independent Rust verification failed",
    )
}
