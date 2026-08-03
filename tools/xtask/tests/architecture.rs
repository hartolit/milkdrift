//! Integration coverage for locked, typed workspace architecture validation.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use xtask::validate_workspace;

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct FixtureWorkspace {
    root: PathBuf,
}

impl FixtureWorkspace {
    fn new(name: &str) -> Result<Self, Box<dyn Error>> {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "xtask-architecture-{name}-{}-{id}",
            std::process::id()
        ));
        copy_fixture(&source, &root)?;
        Ok(Self { root })
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    fn write(&self, relative: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }
}

impl Drop for FixtureWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_fixture(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let file_name = entry.file_name();
        let destination_name = if file_name == "fixture.lock" {
            OsString::from("Cargo.lock")
        } else {
            file_name
        };
        let destination_path = destination.join(destination_name);
        if file_type.is_dir() {
            copy_fixture(&entry.path(), &destination_path)?;
        } else {
            fs::copy(entry.path(), destination_path)?;
        }
    }
    Ok(())
}

fn workspace_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Cargo.toml")
}

#[test]
fn actual_workspace_satisfies_architecture_policy() -> Result<(), Box<dyn Error>> {
    let manifest = workspace_manifest();
    let report = validate_workspace(&manifest)?;

    assert!(
        report.is_valid(),
        "actual workspace violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn forbidden_actual_manifest_edge_reports_rule_and_reason() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("forbidden-edge")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report.violations().iter().find(|violation| {
        violation.source() == "domain-contracts" && violation.target() == "candle-backend"
    }) else {
        return Err("fixture did not report its F0 -> adapter manifest edge".into());
    };

    assert_eq!(violation.rule(), "LAYER-PROD-1");
    assert_eq!(
        violation.dependency_kind(),
        Some(xtask::DependencyKind::Normal)
    );
    assert!(violation.reason().contains("9-role workspace dependency"));
    let rendered = violation.to_string();
    assert!(rendered.contains("policy rule LAYER-PROD-1"));
    assert!(rendered.contains("normal and build dependencies"));
    Ok(())
}

#[test]
fn unknown_workspace_location_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unknown-location")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "mystery")
    else {
        return Err("fixture's unknown package location was accepted".into());
    };

    assert_eq!(violation.rule(), "LAYOUT-1");
    assert!(violation.target().contains("crates/experimental/mystery"));
    assert!(
        violation
            .reason()
            .contains("never receive a fallback layer")
    );
    Ok(())
}

#[test]
fn unknown_tooling_package_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unknown-tool")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "release-tool")
    else {
        return Err("fixture's unknown tooling package was accepted".into());
    };

    assert_eq!(violation.rule(), "LAYOUT-1");
    assert!(violation.target().contains("tools/release"));
    assert!(violation.reason().contains("unknown tools fail closed"));
    Ok(())
}

#[test]
fn unknown_benchmark_package_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unknown-benchmark")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "experimental-benchmarks")
    else {
        return Err("fixture's unknown benchmark package was accepted".into());
    };

    assert_eq!(violation.rule(), "BENCHMARK-ROLE-1");
    assert!(violation.target().contains("benchmarks/experimental"));
    assert!(
        violation
            .reason()
            .contains("unknown benchmark package paths fail closed")
    );
    Ok(())
}

#[test]
fn benchmark_package_properties_and_reverse_edges_fail_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("benchmark-policy")?;
    fixture.write("benchmarks/runtime/build.rs", "fn main() {}\n")?;
    let report = validate_workspace(&fixture.manifest())?;

    for rule in ["BENCHMARK-PUBLISH-1", "BENCHMARK-BUILD-1"] {
        assert!(report.violations().iter().any(|violation| {
            violation.source() == "runtime-benchmarks" && violation.rule() == rule
        }));
    }
    let Some(reverse) = report.violations().iter().find(|violation| {
        violation.source() == "domain-contracts"
            && violation.target() == "runtime-benchmarks"
            && violation.rule() == "BENCHMARK-REVERSE-1"
    }) else {
        return Err("fixture's production-to-benchmark edge was accepted".into());
    };
    assert_eq!(
        reverse.dependency_kind(),
        Some(xtask::DependencyKind::Development)
    );
    assert!(reverse.reason().contains("outer consumer only"));
    Ok(())
}

#[test]
fn unregistered_runtime_role_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unregistered-runtime")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "memory-runtime")
    else {
        return Err("fixture's unregistered runtime was accepted".into());
    };

    assert_eq!(violation.rule(), "RUNTIME-ROLE-1");
    assert!(
        violation
            .reason()
            .contains("directory placement does not grant a capability role")
    );
    Ok(())
}

#[test]
fn unregistered_platform_role_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unregistered-platform")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "native")
    else {
        return Err("fixture's unregistered platform crate was accepted".into());
    };

    assert_eq!(violation.rule(), "PLATFORM-ROLE-1");
    assert!(
        violation
            .reason()
            .contains("directory placement does not grant infrastructure authority")
    );
    Ok(())
}

#[test]
fn unreviewed_runtime_edge_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unreviewed-runtime-edge")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report.violations().iter().find(|violation| {
        violation.source() == "application-runtime" && violation.target() == "corrective-workflow"
    }) else {
        return Err("fixture's unreviewed E1 -> capability edge was accepted".into());
    };

    assert_eq!(violation.rule(), "ENGINE-LOCAL-PROD-1");
    assert!(
        violation
            .reason()
            .contains("exact reviewed composition edge")
    );
    Ok(())
}

#[test]
fn unreviewed_domain_peer_edge_fails_closed() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("unreviewed-domain-edge")?;
    let report = validate_workspace(&fixture.manifest())?;
    let Some(violation) = report
        .violations()
        .iter()
        .find(|violation| violation.source() == "sampling" && violation.target() == "tokenization")
    else {
        return Err("fixture's unreviewed F1 -> F1 edge was accepted".into());
    };

    assert_eq!(violation.rule(), "DOMAIN-LOCAL-PROD-1");
    assert_eq!(
        violation.dependency_kind(),
        Some(xtask::DependencyKind::Normal)
    );
    assert!(violation.reason().contains("exact reviewed edge"));
    assert!(violation.reason().contains("acyclic domain graph"));
    Ok(())
}

#[test]
fn exact_reviewed_cuda_feature_chain_is_accepted() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    let report = validate_workspace(&fixture.manifest())?;
    let cuda_violations = report
        .violations()
        .iter()
        .filter(|violation| {
            violation.rule().starts_with("CUDA-") || violation.rule() == "POLICY-REVIEW-1"
        })
        .collect::<Vec<_>>();

    assert!(
        cuda_violations.is_empty(),
        "exact reviewed CUDA chain was rejected: {cuda_violations:#?}"
    );
    Ok(())
}

#[test]
fn e1_and_application_defaults_cannot_reach_cuda() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.write(
        "crates/runtime/application-runtime/Cargo.toml",
        "[package]\nname = \"application-runtime\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = [\"cuda\"]\ncuda = [\"candle-backend/cuda\"]\n\n[dependencies]\ncandle-backend = { path = \"../../adapters/candle-backend\" }\n",
    )?;
    fixture.write(
        "crates/apps/desktop-slint/Cargo.toml",
        "[package]\nname = \"desktop-slint\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = [\"cuda\"]\ncuda = [\"application-runtime/cuda\"]\n\n[dependencies]\napplication-runtime = { path = \"../../runtime/application-runtime\" }\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    for package in ["application-runtime", "desktop-slint"] {
        assert!(report.violations().iter().any(|violation| {
            violation.source() == package
                && violation.target() == "default"
                && violation.rule() == "CUDA-DEFAULT-1"
        }));
    }
    Ok(())
}

#[test]
fn generic_gpu_alias_is_not_a_reviewed_cuda_feature() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.write(
        "crates/apps/desktop-slint/Cargo.toml",
        "[package]\nname = \"desktop-slint\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = []\ncuda = [\"application-runtime/cuda\"]\ngpu = [\"cuda\"]\n\n[dependencies]\napplication-runtime = { path = \"../../runtime/application-runtime\" }\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    assert!(report.violations().iter().any(|violation| {
        violation.source() == "desktop-slint"
            && violation.target() == "gpu"
            && violation.rule() == "CUDA-BOUNDARY-1"
            && violation.reason().contains("aliases are not allowed")
    }));
    Ok(())
}

#[test]
fn cargo_metadata_invalid_target_feature_references_remain_rejected() -> Result<(), Box<dyn Error>>
{
    let fixture = FixtureWorkspace::new("cuda-policy")?;
    fixture.write(
        "crates/adapters/candle-backend/Cargo.toml",
        "[package]\nname = \"candle-backend\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = []\n\n[dependencies]\ncandle-core = { path = \"../../../vendor/candle-core\", default-features = false }\ncandle-nn = { path = \"../../../vendor/candle-nn\", default-features = false }\ncandle-transformers = { path = \"../../../vendor/candle-transformers\", default-features = false }\ncudarc = { path = \"../../../vendor/cudarc\", default-features = false, optional = true }\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    for package in ["application-runtime", "inference-runtime"] {
        assert!(report.violations().iter().any(|violation| {
            violation.source() == package
                && violation.target() == "candle-backend/cuda"
                && violation.rule() == "CUDA-BOUNDARY-1"
                && violation
                    .reason()
                    .contains("retains invalid target-feature references")
        }));
    }
    Ok(())
}

#[test]
fn candle_cuda_cannot_become_a_default_feature() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("forbidden-edge")?;
    fixture.write(
        "crates/adapters/candle-backend/Cargo.toml",
        "[package]\nname = \"candle-backend\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = [\"cuda\"]\ncuda = []\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    assert!(report.violations().iter().any(|violation| {
        violation.source() == "candle-backend" && violation.rule() == "CUDA-DEFAULT-1"
    }));
    Ok(())
}

#[test]
fn production_crates_cannot_enable_candle_cuda_outside_reviewed_composition()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("forbidden-edge")?;
    fixture.write(
        "crates/adapters/candle-backend/Cargo.toml",
        "[package]\nname = \"candle-backend\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = []\ncuda = []\n",
    )?;
    fixture.write(
        "crates/domain/domain-contracts/Cargo.toml",
        "[package]\nname = \"domain-contracts\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ngpu = [\"candle-backend/cuda\"]\n\n[dependencies]\ncandle-backend = { path = \"../../adapters/candle-backend\" }\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    assert!(report.violations().iter().any(|violation| {
        violation.source() == "domain-contracts" && violation.rule() == "CUDA-BOUNDARY-1"
    }));
    Ok(())
}

#[test]
fn cudnn_flash_attention_and_nccl_require_a_separate_decision() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("forbidden-edge")?;
    fixture.write(
        "crates/adapters/candle-backend/Cargo.toml",
        "[package]\nname = \"candle-backend\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[features]\ndefault = []\ncuda = []\ncudnn = []\nflash-attn = []\nnccl = []\n",
    )?;
    let report = validate_workspace(&fixture.manifest())?;

    for prohibited in ["cudnn", "flash-attn", "nccl"] {
        assert!(report.violations().iter().any(|violation| {
            violation.source() == "candle-backend"
                && violation.target() == prohibited
                && violation.rule() == "CUDA-PROHIBITED-1"
        }));
    }
    Ok(())
}
