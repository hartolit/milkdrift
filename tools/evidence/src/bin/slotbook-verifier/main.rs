//! Operator-owned finite HTTP verifier, pinned by executable digest outside editable workers.
mod checks;
mod service;

use milkdrift_evidence::EvidenceResult;
use serde::Deserialize;
use serde_json::Value;
use std::{fs, io::Read, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Specification {
    schema_version: u32,
    image: String,
    image_identity: String,
    candidate: PathBuf,
    token_file: PathBuf,
    directory: PathBuf,
    application: Value,
    limits: Value,
    subject: Value,
    required_checks: Vec<String>,
    container: String,
    platform_owner: String,
    evaluation: String,
}

#[derive(Debug)]
struct Violation(&'static str);
impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Violation {}
fn require(condition: bool, message: &'static str) -> EvidenceResult {
    if condition {
        Ok(())
    } else {
        Err(Box::new(Violation(message)))
    }
}

fn main() -> EvidenceResult {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("expected verifier input document")?;
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(65_537).read_to_end(&mut bytes)?;
    require(bytes.len() <= 65_536, "verifier input exceeds bound")?;
    let spec: Specification = serde_json::from_slice(&bytes)?;
    require(
        spec.schema_version == 2 && spec.required_checks.len() <= 32,
        "unsupported verifier contract",
    )?;
    let artifact = spec
        .subject
        .get("artifact")
        .ok_or("candidate subject absent")?;
    let expected = artifact
        .get("digest")
        .and_then(Value::as_str)
        .ok_or("candidate digest absent")?;
    // The platform already validates these bytes; this independent read also makes a copied
    // verifier reject a changed candidate before it starts its isolated application.
    let expected_size = artifact
        .get("size_bytes")
        .and_then(Value::as_u64)
        .ok_or("candidate size absent")?;
    let read_bound = expected_size
        .checked_add(1)
        .ok_or("candidate size overflows")?;
    let mut candidate = fs::File::open(&spec.candidate)?.take(read_bound);
    let mut hasher = blake3::Hasher::new();
    let size = std::io::copy(&mut candidate, &mut hasher)?;
    require(expected_size == size, "candidate size differs from subject")?;
    let actual = format!("b3_{}", hasher.finalize());
    require(
        actual == expected || actual.trim_start_matches("b3_") == expected,
        "candidate bytes differ from subject",
    )?;
    let mut service = service::Service::prepare(spec)?;
    let result = service.observe();
    let cleanup = service.cleanup();
    cleanup?;
    println!("{}", serde_json::to_string(&result?)?);
    Ok(())
}
