//! The maintained factory uses production CLI readers without a daemon or model.
#[cfg(target_os = "linux")]
#[test]
fn maintained_slotbook_factory_authors_strict_governed_documents()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("slotbook");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("/usr/bin/python3")
        .arg(repository.join("examples/adaptive-slotbook/prepare.py"))
        .args([
            "--root",
            root.to_str().ok_or("non-UTF8 path")?,
            "--cli",
            env!("CARGO_BIN_EXE_milkdrift"),
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
    Ok(())
}
