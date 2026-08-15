use super::support::*;

#[test]
fn cuda_hardware_alias_is_exact_local_and_harness_free() -> Result<(), Box<dyn Error>> {
    let wrong_alias = FixtureWorkspace::new("cuda-policy")?;
    wrong_alias.replace(
        "crates/runtime/application-runtime/Cargo.toml",
        "cuda-hardware-tests = [\"cuda\"]",
        "cuda-hardware-tests = []",
    )?;
    let wrong_alias_report = wrong_alias.report()?;
    assert!(has_violation(
        &wrong_alias_report,
        "application-runtime",
        "CUDA-HARDWARE-TEST-1"
    ));

    let harnessed = FixtureWorkspace::new("cuda-policy")?;
    harnessed.replace(
        "crates/runtime/inference-runtime/Cargo.toml",
        "harness = false",
        "harness = true",
    )?;
    let harnessed_report = harnessed.report()?;
    assert!(has_violation(
        &harnessed_report,
        "inference-runtime",
        "CUDA-HARDWARE-TEST-1"
    ));

    let forwarded = FixtureWorkspace::new("cuda-policy")?;
    forwarded.replace(
        "crates/apps/desktop-slint/Cargo.toml",
        "cuda = [\"application-runtime/cuda\"]",
        "cuda = [\"application-runtime/cuda\"]\nhardware = [\"application-runtime/cuda-hardware-tests\"]",
    )?;
    let forwarded_report = forwarded.report()?;
    assert!(has_violation(
        &forwarded_report,
        "desktop-slint",
        "CUDA-HARDWARE-TEST-1"
    ));
    Ok(())
}

#[test]
fn harness_free_cuda_fixture_targets_link_run_all_cases_and_require_opt_in()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.refresh_lock()?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let target = fixture.root.join("target");

    for package in ["candle-backend", "inference-runtime", "application-runtime"] {
        let status = Command::new(&cargo)
            .args(["test", "--offline", "--locked", "--manifest-path"])
            .arg(fixture.manifest())
            .args([
                "-p",
                package,
                "--features",
                "cuda-hardware-tests",
                "--test",
                "cuda_hardware",
                "--quiet",
            ])
            .env("CARGO_TARGET_DIR", &target)
            .env("MILKDRIFT_CUDA_TEST", "1")
            .status()?;
        assert!(
            status.success(),
            "{package} custom CUDA target did not pass"
        );
    }

    let output = Command::new(cargo)
        .args(["test", "--offline", "--locked", "--manifest-path"])
        .arg(fixture.manifest())
        .args([
            "-p",
            "candle-backend",
            "--features",
            "cuda-hardware-tests",
            "--test",
            "cuda_hardware",
            "--quiet",
        ])
        .env("CARGO_TARGET_DIR", target)
        .env_remove("MILKDRIFT_CUDA_TEST")
        .output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("set MILKDRIFT_CUDA_TEST=1"));
    Ok(())
}
