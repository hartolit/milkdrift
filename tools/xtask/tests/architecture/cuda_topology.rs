use super::support::*;

#[test]
fn exact_cuda_chain_and_hardware_suite_are_accepted() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    let report = fixture.report()?;
    assert!(
        report.is_valid(),
        "exact CUDA fixture violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn cuda_defaults_aliases_direct_features_and_unreviewed_forwards_fail() -> Result<(), Box<dyn Error>>
{
    let default = FixtureWorkspace::new("cuda-policy")?;
    default.replace(
        "crates/runtime/application-runtime/Cargo.toml",
        "default = []",
        "default = [\"cuda\"]",
    )?;
    let default_report = default.report()?;
    assert!(has_violation(
        &default_report,
        "application-runtime",
        "CUDA-DEFAULT-1"
    ));

    let alias = FixtureWorkspace::new("cuda-policy")?;
    alias.replace(
        "crates/apps/desktop-slint/Cargo.toml",
        "cuda = [\"application-runtime/cuda\"]",
        "cuda = [\"application-runtime/cuda\"]\ngpu = [\"cuda\"]",
    )?;
    let alias_report = alias.report()?;
    assert!(alias_report.violations().iter().any(|violation| {
        violation.source() == "desktop-slint"
            && violation.target() == "gpu"
            && violation.rule() == "CUDA-BOUNDARY-1"
    }));

    let direct = FixtureWorkspace::new("cuda-policy")?;
    direct.replace(
        "crates/runtime/application-runtime/Cargo.toml",
        "candle-backend = { path = \"../../adapters/candle-backend\" }",
        "candle-backend = { path = \"../../adapters/candle-backend\", features = [\"cuda\"] }",
    )?;
    let direct_report = direct.report()?;
    assert!(has_violation(
        &direct_report,
        "application-runtime",
        "CUDA-BOUNDARY-1"
    ));

    let unreviewed = FixtureWorkspace::new("cuda-policy")?;
    unreviewed.replace(
        "crates/apps/desktop-slint/Cargo.toml",
        "cuda = [\"application-runtime/cuda\"]",
        "cuda = [\"application-runtime/cuda\", \"application-runtime/cuda\"]",
    )?;
    let unreviewed_report = unreviewed.report()?;
    assert!(has_violation(
        &unreviewed_report,
        "desktop-slint",
        "CUDA-BOUNDARY-1"
    ));
    Ok(())
}

#[test]
fn provider_cudarc_activation_cannot_bypass_the_exact_cuda_feature() -> Result<(), Box<dyn Error>> {
    let mutations = [
        (
            "default = []",
            "default = [\"dep:cudarc\"]",
            "default",
            "CUDA-DEFAULT-1",
        ),
        (
            "cuda-hardware-tests = [\"cuda\"]",
            "cuda-hardware-tests = [\"cuda\"]\ngpu = [\"dep:cudarc\"]",
            "gpu",
            "CUDA-BOUNDARY-1",
        ),
        (
            "cuda-hardware-tests = [\"cuda\"]",
            "cuda-hardware-tests = [\"cuda\", \"dep:cudarc\"]",
            "cuda-hardware-tests",
            "CUDA-HARDWARE-TEST-1",
        ),
        (
            "default = []",
            "default = [\"cudarc/driver\"]",
            "default",
            "CUDA-DEFAULT-1",
        ),
        (
            "cuda-hardware-tests = [\"cuda\"]",
            "cuda-hardware-tests = [\"cuda\"]\ngpu = [\"cudarc/driver\"]",
            "gpu",
            "CUDA-BOUNDARY-1",
        ),
        (
            "cuda-hardware-tests = [\"cuda\"]",
            "cuda-hardware-tests = [\"cuda\", \"cudarc/driver\"]",
            "cuda-hardware-tests",
            "CUDA-HARDWARE-TEST-1",
        ),
    ];

    for (old, new, target, rule) in mutations {
        let fixture = FixtureWorkspace::new("cuda-policy")?;
        fixture.replace("crates/adapters/candle-backend/Cargo.toml", old, new)?;
        let report = fixture.report()?;
        assert!(report.violations().iter().any(|violation| {
            violation.source() == "candle-backend"
                && violation.target() == target
                && violation.rule() == rule
        }));
        assert!(report.violations().iter().any(|violation| {
            violation.source() == "candle-backend"
                && violation.target() == "cudarc activation"
                && violation.rule() == "CUDA-CONTRACT-1"
        }));
    }
    Ok(())
}

#[test]
fn cuda_topology_requires_exactly_one_provider() -> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("cuda-policy")?;
    missing.replace(
        "crates/adapters/candle-backend/Cargo.toml",
        "cuda-provider = true\n",
        "",
    )?;
    let missing_report = missing.report()?;
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "workspace CUDA topology"
            && violation.rule() == "CUDA-CONTRACT-1"
            && violation.reason().contains("found 0 (none)")
    }));

    let duplicate = FixtureWorkspace::new("cuda-policy")?;
    duplicate.replace(
        "crates/runtime/application-runtime/Cargo.toml",
        "role = \"runtime-application\"",
        "role = \"runtime-application\"\ncuda-provider = true",
    )?;
    let duplicate_report = duplicate.report()?;
    assert!(duplicate_report.violations().iter().any(|violation| {
        violation.source() == "workspace CUDA topology"
            && violation.rule() == "CUDA-CONTRACT-1"
            && violation.reason().contains("found 2 (")
            && violation.reason().contains("application-runtime")
            && violation.reason().contains("candle-backend")
    }));
    Ok(())
}

#[test]
fn cudnn_flash_attention_and_nccl_remain_absolute_denials() -> Result<(), Box<dyn Error>> {
    for prohibited in ["cudnn", "flash-attn", "nccl"] {
        let fixture = FixtureWorkspace::new("cuda-policy")?;
        fixture.replace(
            "crates/adapters/candle-backend/Cargo.toml",
            "cuda-hardware-tests = [\"cuda\"]",
            &format!("cuda-hardware-tests = [\"cuda\"]\n{prohibited} = []"),
        )?;
        let report = fixture.report()?;
        assert!(report.violations().iter().any(|violation| {
            violation.source() == "candle-backend"
                && violation.target() == prohibited
                && violation.rule() == "CUDA-PROHIBITED-1"
        }));
    }

    let dependency = FixtureWorkspace::new("cuda-policy")?;
    dependency.write(
        "vendor/cudnn/Cargo.toml",
        "[package]\nname = \"cudnn\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
    )?;
    dependency.write("vendor/cudnn/src/lib.rs", "pub fn fixture() {}\n")?;
    dependency.replace(
        "Cargo.toml",
        "cudarc = { path = \"vendor/cudarc\" }",
        "cudarc = { path = \"vendor/cudarc\" }\ncudnn = { path = \"vendor/cudnn\" }",
    )?;
    dependency.replace(
        "crates/adapters/candle-backend/Cargo.toml",
        "[dependencies]",
        "[dependencies]\ncudnn = \"=1.0.0\"",
    )?;
    let dependency_report = dependency.report()?;
    assert!(dependency_report.violations().iter().any(|violation| {
        violation.source() == "candle-backend"
            && violation.target() == "cudnn"
            && violation.rule() == "CUDA-PROHIBITED-1"
    }));
    Ok(())
}

#[test]
fn exact_cudarc_dependency_contract_is_enforced() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.replace(
        "crates/adapters/candle-backend/Cargo.toml",
        "default-features = false, features = [\"std\", \"driver\", \"cuda-version-from-build-system\", \"dynamic-linking\"], optional = true",
        "default-features = true, features = [\"std\", \"driver\", \"cuda-version-from-build-system\", \"dynamic-linking\"], optional = true",
    )?;
    let report = fixture.report()?;
    assert!(has_violation(&report, "candle-backend", "CUDA-CONTRACT-1"));
    Ok(())
}
