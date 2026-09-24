//! Opt-in checks of verifier resource ownership with real rootless Podman.
use milkdrift_authority::ProtectedEffectPolicy;
use milkdrift_capability::{
    BoundedJson,
    managed::{DataDisposition, ManagedName},
};
use milkdrift_capability_host::managed::ManagedPlatform;
use milkdrift_managed_linux::{
    ContainerLimits, LinuxManagedPlatform, LinuxManagerConfig, ProtectedServiceRecipe,
};
use milkdrift_persistence::managed::{
    InstallationRecord, ManagedChange, ManagedChangePhase, ManagedStep, ManagedTransition,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference, CandidateCheck, CandidateEvaluation, CandidateSubject, CausalId,
    ContentDigest, MediaType,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("b3_{}", blake3::hash(bytes.as_ref()))
}
fn podman(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("/usr/bin/podman").args(args).output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(output.stdout)
}
fn exists(name: &str) -> Result<bool> {
    Ok(Command::new("/usr/bin/podman")
        .args(["container", "exists", name])
        .status()?
        .success())
}
fn private(path: &Path) -> Result {
    fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[test]
#[ignore = "requires MILKDRIFT_PROTECTED_TEST_IMAGE with a preloaded sleep-capable image and real rootless Podman"]
fn verifier_renewal_timeout_recovery_and_preentry_integrity() -> Result {
    let image = std::env::var("MILKDRIFT_PROTECTED_TEST_IMAGE")?;
    let parent = std::env::var_os("MILKDRIFT_LINUX_EVIDENCE_PARENT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = tempfile::Builder::new()
        .prefix("milkdrift-protected-")
        .tempdir_in(parent)?
        .keep();
    eprintln!("protected verification evidence: {}", root.display());
    for name in ["state", "quadlet", "systemd"] {
        private(&root.join(name))?;
    }
    let verifier = root.join("verifier");
    fs::copy(
        env!("CARGO_BIN_EXE_milkdrift-verifier-test-helper"),
        &verifier,
    )?;
    fs::write(root.join("token"), "synthetic-test-token")?;
    fs::write(root.join("clock"), "2027-04-10T09:00:00Z")?;
    let verifier_digest = digest(fs::read(&verifier)?);
    let policy = ProtectedEffectPolicy {
        schema_version: 1,
        required_checks: vec!["lifecycle".to_owned()],
        verifier: verifier_digest.clone(),
        producer: CausalId::new("trusted:physical-verifier")?,
        maximum_candidate_bytes: 1024,
        validity_ms: 60_000,
    };
    let mut recipe = ProtectedServiceRecipe {
        schema_version: 2,
        kind: "protected_service".to_owned(),
        name: ManagedName::new("verifier-test")?,
        image,
        limits: ContainerLimits {
            memory_bytes: 536_870_912,
            cpu_percent: 100,
            pids: 64,
            temporary_bytes: 16_777_216,
        },
        startup_ms: 30_000,
        shutdown_ms: 10_000,
        port: 19849,
        application: BoundedJson::new(serde_json::json!({"timeout":false}))?,
        token_file: root.join("token"),
        token_digest: digest("synthetic-test-token"),
        clock_file: root.join("clock"),
        agreement: digest("agreement"),
        policy,
        verifier_executable: verifier,
        verifier_digest,
        verification_timeout_ms: 5000,
        data_disposition: DataDisposition::Preserve,
    };
    let recipe_path = root.join("recipe.json");
    fs::write(&recipe_path, serde_json::to_vec(&recipe)?)?;
    let config = LinuxManagerConfig {
        state_root: root.join("state"),
        quadlet_directory: root.join("quadlet"),
        systemd_directory: root.join("systemd"),
        recipes: vec![recipe_path.clone()],
    };
    let platform = LinuxManagedPlatform::new(config.clone())?;
    let setup = platform.plan(&recipe.name, &recipe.reference()?, &digest("ownership"), 1)?;
    let bytes = b"immutable candidate fixture: not executed by this lifecycle verifier";
    let mut evaluation = CandidateEvaluation {
        schema_version: 1,
        identity: digest("first-evaluation"),
        started_at: 1,
        expires_at: 60_001,
        complete: false,
        subject: CandidateSubject {
            artifact: ArtifactReference::new(
                ArtifactId::new("candidate")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("text/plain")?,
                bytes.len() as u64,
            ),
            target: recipe.name.clone(),
            generation: 1,
            agreement: recipe.agreement.clone(),
            configuration: setup.recipe.digest.clone(),
            policy: recipe.policy.digest()?,
            verifier: recipe.policy.verifier.clone(),
            producer: recipe.policy.producer.clone(),
        },
        checks: vec![CandidateCheck {
            name: "lifecycle".to_owned(),
            passed: None,
            diagnostic: "incomplete".to_owned(),
        }],
    };
    // This deliberately non-cleaning trusted fixture leaves cleanup to the platform owner.
    for key in ["first-evaluation", "renewed-evaluation"] {
        evaluation.identity = digest(key);
        let checks = platform.evaluate_candidate(&setup, &evaluation, bytes)?;
        assert_eq!(checks[0].passed, Some(true));
        let directory = root
            .join("state")
            .join(format!("verification-{}", evaluation.identity));
        let id = fs::read_to_string(directory.join("launched"))?;
        assert!(
            !exists(id.trim())?,
            "successful verifier leaked its detached container"
        );
    }
    assert!(
        platform
            .evaluate_candidate(&setup, &evaluation, bytes)
            .is_err(),
        "same evaluation scratch must never be rerun"
    );

    // Reconstruct the exact owned leftover a killed host would leave, then recover on reopen.
    let input: serde_json::Value = serde_json::from_slice(&fs::read(
        root.join("state")
            .join(format!("verification-{}", evaluation.identity))
            .join("input.json"),
    )?)?;
    let name = input["container"].as_str().ok_or("container absent")?;
    let owner = input["platform_owner"].as_str().ok_or("owner absent")?;
    podman(&[
        "run",
        "--detach",
        "--pull=never",
        "--network=none",
        &format!("--name={name}"),
        &format!("--label=org.milkdrift.platform={owner}"),
        &format!("--label=org.milkdrift.verification={}", evaluation.identity),
        "--entrypoint=/bin/sleep",
        &recipe.image,
        "120",
    ])?;
    assert!(exists(name)?);
    drop(platform);
    let platform = LinuxManagedPlatform::new(config.clone())?;
    platform.recover_verifications()?;
    assert!(
        !exists(name)?,
        "startup left the interrupted verifier container running"
    );

    evaluation.complete = true;
    evaluation.checks[0].passed = Some(true);
    let prepared = platform.prepare_publication(&setup, &evaluation, bytes, 2)?;
    let path = PathBuf::from(
        prepared.configuration.value()["candidate"]
            .as_str()
            .ok_or("candidate absent")?,
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    fs::write(&path, vec![b'x'; bytes.len()])?;
    let record = InstallationRecord {
        schema_version: 2,
        name: recipe.name.clone(),
        version: 1,
        generation: 1,
        current: Some(setup),
        pending: Some(ManagedTransition {
            change: ManagedChange {
                entry_authorization: None,
                identity: digest("publication"),
                candidate: prepared,
                generation: 2,
                steps: vec![ManagedStep::Configure, ManagedStep::StartService],
                removing: false,
                running: true,
            },
            next_step: 0,
            evidence: vec![],
            phase: ManagedChangePhase::Pending {},
        }),
        desired_running: false,
        observation: None,
        admission_open: false,
        removed: false,
        uses: vec![],
    };
    for step in [ManagedStep::Configure, ManagedStep::StartService] {
        let Err(error) = platform.reconcile(&record, step) else {
            return Err("changed bytes must refuse before activation".into());
        };
        assert!(
            error.to_string().contains("candidate bytes differ"),
            "{error}"
        );
    }
    drop(platform);

    recipe.application = BoundedJson::new(serde_json::json!({"timeout":true}))?;
    fs::write(&recipe_path, serde_json::to_vec(&recipe)?)?;
    let platform = LinuxManagedPlatform::new(config)?;
    let setup = platform.plan(
        &recipe.name,
        &recipe.reference()?,
        &digest("timeout-ownership"),
        1,
    )?;
    evaluation.identity = digest("timed-out-evaluation");
    evaluation.complete = false;
    evaluation.checks[0].passed = None;
    evaluation.subject.configuration = setup.recipe.digest.clone();
    let Err(error) = platform.evaluate_candidate(&setup, &evaluation, bytes) else {
        return Err("verifier must time out".into());
    };
    assert!(error.to_string().contains("deadline exceeded"), "{error}");
    let id = fs::read_to_string(
        root.join("state")
            .join(format!("verification-{}", evaluation.identity))
            .join("launched"),
    )?;
    assert!(
        !exists(id.trim())?,
        "timed-out verifier leaked its detached container"
    );
    Ok(())
}
