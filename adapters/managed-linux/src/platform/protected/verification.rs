use super::{LinuxManagedPlatform, ManagedError, decode, digest, immutable, rejected};
use crate::command;
use milkdrift_persistence::managed::ApprovedSetup;
use milkdrift_workspace::{CandidateCheck, CandidateEvaluation};
use std::{fs, path::Path, time::Duration};

pub(super) fn evaluate(
    platform: &LinuxManagedPlatform,
    setup: &ApprovedSetup,
    evaluation: &CandidateEvaluation,
    bytes: &[u8],
) -> Result<Vec<CandidateCheck>, ManagedError> {
    super::diagnose(platform, setup)?;
    let d = decode(platform, setup)?;
    evaluation.validate().map_err(rejected)?;
    let subject = &evaluation.subject;
    if !subject.artifact.verifies(bytes) {
        return Err(rejected("verifier candidate bytes differ"));
    }
    // A new authorized request can renew expired evidence for unchanged bytes. Exact request
    // replay is fenced by the journal before this method is called.
    let directory = platform
        .config
        .state_root
        .join(format!("verification-{}", evaluation.identity));
    let container = container_name(&platform.owner, &evaluation.identity);
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory).map_err(|_| {
        rejected("verification scratch already exists; prior observation requires inspection")
    })?;
    immutable(&directory.as_path().join("candidate.py"), bytes)?;
    let harness = fs::read(&d.recipe.verifier_file).map_err(rejected)?;
    if digest(&harness) != d.recipe.verifier_source_digest {
        return Err(rejected("verifier generation changed during preparation"));
    }
    immutable(&directory.as_path().join("verifier.py"), &harness)?;
    let input = serde_json::json!({"schema_version":1, "image":d.recipe.image, "executable":d.recipe.executable,
        "candidate":directory.as_path().join("candidate.py"), "token_file":d.recipe.token_file, "directory":directory.as_path(), "application": d.recipe.application,
        "image_identity":LinuxManagedPlatform::image_identity(&d.recipe.image)?,
        "limits":d.recipe.limits, "subject":subject, "required_checks": d.recipe.policy.required_checks,
        "container":container, "platform_owner":platform.owner, "evaluation":evaluation.identity});
    immutable(
        &directory.as_path().join("input.json"),
        &serde_json::to_vec(&input).map_err(rejected)?,
    )?;
    let output = command::run(
        Path::new("/usr/bin/python3"),
        &[
            "-I".to_owned(),
            directory
                .as_path()
                .join("verifier.py")
                .display()
                .to_string(),
            directory.as_path().join("input.json").display().to_string(),
        ],
        &[],
        Duration::from_millis(d.recipe.verification_timeout_ms),
        32_768,
        None,
    );
    // A detached container outlives the verifier process group. Fence it even when that group
    // timed out, overflowed its output, or exited without running its own cleanup.
    cleanup(platform, &evaluation.identity)?;
    let output = output?;
    if !output.success {
        return Err(rejected("trusted verifier execution did not complete"));
    }
    let value =
        milkdrift_contracts::parse_json_without_duplicates(&output.stdout).map_err(rejected)?;
    let checks: Vec<CandidateCheck> = serde_json::from_value(value).map_err(rejected)?;
    if checks.len() > 32 || checks.iter().any(|c| c.diagnostic.len() > 256) {
        return Err(rejected("verifier observations exceeded bounds"));
    }
    Ok(checks)
}

fn container_name(owner: &str, evaluation: &str) -> String {
    format!("mdverify-{}", digest(format!("{owner}:{evaluation}")))
}

fn cleanup(platform: &LinuxManagedPlatform, evaluation: &str) -> Result<(), ManagedError> {
    let name = container_name(&platform.owner, evaluation);
    if let Some(value) = LinuxManagedPlatform::inspect("container", &name)? {
        let id = owned_container(&value, &platform.owner, evaluation)?;
        // Delete the inspected identity, not a name that could have been replaced meanwhile.
        LinuxManagedPlatform::podman(&[
            "rm".to_owned(),
            "--force".to_owned(),
            "--time=0".to_owned(),
            id.to_owned(),
        ])?;
        if LinuxManagedPlatform::inspect("container", id)?.is_some() {
            return Err(rejected("verification container cleanup remains uncertain"));
        }
    }
    Ok(())
}

fn owned_container<'a>(
    value: &'a serde_json::Value,
    owner: &str,
    evaluation: &str,
) -> Result<&'a str, ManagedError> {
    let id = value.get("Id").and_then(|v| v.as_str()).unwrap_or("");
    if !milkdrift_contracts::is_canonical_blake3_digest(evaluation)
        || value
            .pointer("/Config/Labels/org.milkdrift.platform")
            .and_then(|v| v.as_str())
            != Some(owner)
        || value
            .pointer("/Config/Labels/org.milkdrift.verification")
            .and_then(|v| v.as_str())
            != Some(evaluation)
        || value.get("Name").and_then(|v| v.as_str())
            != Some(container_name(owner, evaluation).as_str())
        || id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(rejected("verification container ownership differs"));
    }
    Ok(id)
}

pub(super) fn recover(platform: &LinuxManagedPlatform) -> Result<(), ManagedError> {
    let mut has_verification = false;
    for entry in fs::read_dir(&platform.config.state_root).map_err(rejected)? {
        let entry = entry.map_err(rejected)?;
        if entry.file_name().to_str().is_some_and(|name| {
            name.strip_prefix("verification-")
                .is_some_and(milkdrift_contracts::is_canonical_blake3_digest)
        }) {
            has_verification = true;
            break;
        }
    }
    if !has_verification {
        return Ok(());
    }
    // Called only during startup, before admission. The bounded engine response discovers only
    // this platform's verification resources; failure keeps startup closed for inspection.
    let bytes = LinuxManagedPlatform::podman(&[
        "ps".to_owned(),
        "--all".to_owned(),
        "--quiet".to_owned(),
        "--no-trunc".to_owned(),
        "--filter".to_owned(),
        format!("label=org.milkdrift.platform={}", platform.owner),
        "--filter".to_owned(),
        "label=org.milkdrift.verification".to_owned(),
    ])?;
    let ids = std::str::from_utf8(&bytes).map_err(rejected)?;
    for id in ids.lines() {
        let Some(value) = LinuxManagedPlatform::inspect("container", id)? else {
            continue;
        };
        let evaluation = value
            .pointer("/Config/Labels/org.milkdrift.verification")
            .and_then(|v| v.as_str())
            .ok_or_else(|| rejected("verification identity absent"))?;
        owned_container(&value, &platform.owner, evaluation)?;
        cleanup(platform, evaluation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{container_name, owned_container};

    #[test]
    fn verifier_cleanup_requires_the_exact_platform_evaluation_and_container() {
        let owner = "test-platform";
        let evaluation = format!("b3_{}", "a".repeat(64));
        let id = "b".repeat(64);
        let value = serde_json::json!({"Id":id, "Name":container_name(owner, &evaluation),
            "Config":{"Labels":{"org.milkdrift.platform":owner,"org.milkdrift.verification":evaluation}}});
        assert_eq!(
            owned_container(&value, owner, &evaluation).ok(),
            Some(id.as_str())
        );
        assert!(owned_container(&value, "foreign-platform", &evaluation).is_err());
        assert!(owned_container(&value, owner, &format!("b3_{}", "c".repeat(64))).is_err());
        for field in ["Id", "Name", "Config"] {
            let mut changed = value.clone();
            changed[field] = serde_json::json!("foreign");
            assert!(owned_container(&changed, owner, &evaluation).is_err());
        }
        assert_ne!(
            container_name(owner, &evaluation),
            container_name("foreign-platform", &evaluation)
        );
    }
}
