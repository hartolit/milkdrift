//! Configuration paths are resolved by the production executable in the operator's directory.
use std::{fs, path::Path, process::Command};

#[test]
fn operator_configuration_accepts_bare_relative_and_absolute_paths_before_storage_opens()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/operator/daemon.toml");
    let configuration = directory.path().join("daemon.toml");
    fs::copy(source, &configuration)?;
    let mut effective = None;
    for path in [
        Path::new("daemon.toml"),
        Path::new("./daemon.toml"),
        &configuration,
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_milkdrift-daemon"))
            .current_dir(directory.path())
            .arg("--config")
            .arg(path)
            .arg("--print-effective-config")
            .output()?;
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let document: toml::Value = toml::from_str(std::str::from_utf8(&output.stdout)?)?;
        assert_eq!(
            Path::new(document["data_root"].as_str().ok_or("missing data root")?),
            directory.path().canonicalize()?.join("data")
        );
        if let Some(previous) = effective.as_ref() {
            assert_eq!(previous, &output.stdout);
        }
        effective = Some(output.stdout);
        assert!(!directory.path().join("data").exists());
    }

    let missing = Command::new(env!("CARGO_BIN_EXE_milkdrift-daemon"))
        .current_dir(directory.path())
        .args(["--config", "missing.toml", "--check-config"])
        .output()?;
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8(missing.stderr)?.contains("configuration could not be read"));
    assert!(!directory.path().join("data").exists());
    Ok(())
}
