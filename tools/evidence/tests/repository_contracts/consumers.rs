//! Real outside-workspace clients exercise privacy and feature gates after macro expansion.

use std::{fs, path::Path, process::Command};

use super::{TestResult, root};

fn check(directory: &Path, source: &str, helpers: bool) -> TestResult<std::process::Output> {
    fs::write(directory.join("src/lib.rs"), source)?;
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(directory)
        .args(["check", "--offline", "--quiet"])
        .env(
            "CARGO_TARGET_DIR",
            root()?.join("target/client-ready-workflows/strict-checks/consumer-build"),
        );
    if helpers {
        command.args(["--features", "helpers"]);
    }
    Ok(command.output()?)
}

#[test]
fn outside_consumers_require_validated_construction_and_explicit_helper_features() -> TestResult {
    assert!(
        milkdrift_authority::Selection::<milkdrift_capability::OperationId>::only(
            std::collections::BTreeSet::new()
        )
        .is_err()
    );
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("src"))?;
    let repository = root()?;
    let mut manifest = String::from(
        "[package]\nname='boundary-consumer'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\n",
    );
    for (name, path) in [
        ("authority", "crates/authority"),
        ("capability", "crates/capability"),
        ("blueprint", "crates/blueprint"),
        ("runtime", "crates/runtime"),
        ("capability-host", "crates/capability-host"),
        ("redb-store", "adapters/redb-store"),
    ] {
        manifest.push_str(&format!(
            "milkdrift-{name}={{path={}}}\n",
            serde_json::to_string(&repository.join(path))?
        ));
    }
    manifest.push_str("[features]\nhelpers=['milkdrift-runtime/test-support','milkdrift-capability-host/test-support','milkdrift-redb-store/test-admin']\n");
    fs::write(directory.path().join("Cargo.toml"), manifest)?;
    let valid = r#"
        pub use milkdrift_capability::{SchemaId, ExtensionKey, TrustZone, PeerId, BoundedJson};
        pub fn selector() -> milkdrift_authority::Selection<milkdrift_capability::OperationId> {
            milkdrift_authority::Selection::any()
        }
        pub fn inspect(revision: &milkdrift_blueprint::BlueprintRevision) { let _ = revision.id(); }
    "#;
    let output = check(directory.path(), valid, false)?;
    assert!(
        output.status.success(),
        "valid consumer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = check(
        directory.path(),
        "pub fn corrupt(value: &mut milkdrift_blueprint::BlueprintRevision) { value.sequence = 99; }",
        false,
    )?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        !output.status.success() && stderr.contains("E0616") && stderr.contains("sequence"),
        "expected private revision field, got {stderr}"
    );
    let output = check(
        directory.path(),
        "pub fn bypass() { let _ = milkdrift_authority::Selection::<milkdrift_capability::OperationId> { kind: todo!() }; }",
        false,
    )?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        !output.status.success() && stderr.contains("E0451") && stderr.contains("kind"),
        "expected private selector construction, got {stderr}"
    );
    let helpers = r#"
        pub use milkdrift_runtime::{ManualClock, DeterministicExecutor};
        pub use milkdrift_capability_host::{InMemorySecretResolver, SystemPeerClock};
        pub use milkdrift_redb_store::{FaultInjector, FaultPoint};
    "#;
    let output = check(directory.path(), helpers, false)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        !output.status.success() && stderr.contains("E0432"),
        "default helper surface exposed: {stderr}"
    );
    for name in ["ManualClock", "InMemorySecretResolver", "FaultInjector"] {
        assert!(stderr.contains(name));
    }
    let output = check(directory.path(), helpers, true)?;
    assert!(
        output.status.success(),
        "explicit helper consumer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
