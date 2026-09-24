//! The maintained factory uses production CLI readers without a daemon or model.
#[cfg(target_os = "linux")]
#[test]
fn maintained_slotbook_factory_authors_strict_governed_documents()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("slotbook");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_slotbook-evidence"))
        .arg("prepare")
        .args([
            "--root",
            root.to_str().ok_or("non-UTF8 path")?,
            "--cli",
            milkdrift_evidence::application::application_binary("milkdrift")?
                .to_str()
                .ok_or("CLI path is not UTF-8")?,
            "--verifier",
            env!("CARGO_BIN_EXE_slotbook-verifier"),
            "--image",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(root.join("governed.json"))?;
    let (document, revision) = milkdrift_blueprint::BlueprintRevisionDocument::from_json(&bytes)?;
    assert_eq!(document.to_canonical_json()?, bytes);
    assert_eq!(
        revision
            .semantic()
            .agreement()
            .ok_or("agreement missing")?
            .scope()
            .maximum_revisions(),
        4
    );
    assert!(
        revision
            .semantic()
            .nodes()
            .contains_key(&milkdrift_blueprint::NodeId::new("publish-candidate")?)
    );
    let policy = milkdrift_authority::ProtectedEffectPolicy::from_json(&std::fs::read(
        root.join("policy.json"),
    )?)?;
    assert_eq!(
        policy.digest()?,
        revision
            .semantic()
            .agreement()
            .ok_or("agreement missing")?
            .effect_policy()
    );
    let bytes = std::fs::read(root.join("protected-recipe.json"))?;
    let recipe = milkdrift_managed_linux::ProtectedServiceRecipe::from_json(&bytes)?;
    assert_eq!(recipe.policy, policy);
    assert_eq!(recipe.verifier_executable, root.join("trusted-verifier"));
    assert_eq!(
        recipe.verifier_digest,
        format!(
            "b3_{}",
            blake3::hash(&std::fs::read(&recipe.verifier_executable)?)
        )
    );
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    for (key, invalid) in [
        ("schema_version", serde_json::json!(1)),
        ("executable", serde_json::json!("/usr/bin/interpreter")),
        (
            "verifier_digest",
            serde_json::json!(format!("b3_{}", "a".repeat(64))),
        ),
    ] {
        let mut changed = value.clone();
        changed[key] = invalid;
        assert!(
            milkdrift_managed_linux::ProtectedServiceRecipe::from_json(&serde_json::to_vec(
                &changed
            )?)
            .is_err(),
            "accepted incompatible {key}"
        );
    }
    Ok(())
}
