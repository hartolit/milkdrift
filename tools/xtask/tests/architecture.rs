//! Integration coverage for metadata-driven workspace architecture validation.

use std::error::Error;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use xtask::{DependencyKind, ValidationReport, benchmark_command_plan, validate_workspace};

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

    fn read(&self, relative: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(self.root.join(relative))?)
    }

    fn write(&self, relative: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn replace(&self, relative: &str, old: &str, new: &str) -> Result<(), Box<dyn Error>> {
        let content = self.read(relative)?;
        if !content.contains(old) {
            return Err(
                format!("fixture {relative} did not contain replacement text `{old}`").into(),
            );
        }
        self.write(relative, &content.replacen(old, new, 1))
    }

    fn append_root(&self, content: &str) -> Result<(), Box<dyn Error>> {
        let mut root = self.read("Cargo.toml")?;
        root.push_str(content);
        self.write("Cargo.toml", &root)
    }

    fn report(&self) -> Result<ValidationReport, Box<dyn Error>> {
        self.refresh_lock()?;
        Ok(validate_workspace(&self.manifest())?)
    }

    fn refresh_lock(&self) -> Result<(), Box<dyn Error>> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let output = Command::new(cargo)
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(self.manifest())
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not generate fixture lockfile: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into())
        }
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
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
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

fn has_violation(report: &ValidationReport, source: &str, rule: &str) -> bool {
    report
        .violations()
        .iter()
        .any(|violation| violation.source() == source && violation.rule() == rule)
}

#[test]
fn actual_workspace_satisfies_architecture_policy() -> Result<(), Box<dyn Error>> {
    let report = validate_workspace(&workspace_manifest())?;
    assert!(
        report.is_valid(),
        "actual workspace violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn scalable_fixture_accepts_all_roles_and_ordinary_legal_edges() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    let report = fixture.report()?;
    assert!(
        report.is_valid(),
        "ordinary role-DAG fixture violations: {:#?}",
        report.violations()
    );
    Ok(())
}

#[test]
fn missing_and_unknown_roles_fail_closed() -> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    missing.replace(
        "crates/domain/f0/Cargo.toml",
        "\n[package.metadata.milkdrift]\nrole = \"domain-foundation\"\n",
        "",
    )?;
    let missing_report = missing.report()?;
    assert!(has_violation(&missing_report, "f0", "ROLE-1"));
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "f0" && violation.reason().contains("missing mandatory")
    }));

    let unknown = FixtureWorkspace::new("scalable-policy")?;
    unknown.replace(
        "crates/domain/f0/Cargo.toml",
        "role = \"domain-foundation\"",
        "role = \"mystery-layer\"",
    )?;
    let unknown_report = unknown.report()?;
    assert!(unknown_report.violations().iter().any(|violation| {
        violation.source() == "f0"
            && violation.rule() == "ROLE-1"
            && violation.reason().contains("unknown role")
    }));
    Ok(())
}

#[test]
fn root_policy_schema_version_is_mandatory_and_exact() -> Result<(), Box<dyn Error>> {
    let missing_namespace = FixtureWorkspace::new("scalable-policy")?;
    missing_namespace.replace(
        "Cargo.toml",
        "[workspace.metadata.milkdrift]",
        "[workspace.metadata.other]",
    )?;
    missing_namespace.replace(
        "Cargo.toml",
        "[[workspace.metadata.milkdrift.exceptions]]",
        "[[workspace.metadata.other.exceptions]]",
    )?;
    let missing_namespace_report = missing_namespace.report()?;
    assert!(
        missing_namespace_report
            .violations()
            .iter()
            .any(|violation| {
                violation.source() == "workspace metadata"
                    && violation.target() == "milkdrift"
                    && violation.rule() == "POLICY-EXCEPTION-1"
                    && violation.reason().contains("missing mandatory")
            })
    );

    let missing_version = FixtureWorkspace::new("scalable-policy")?;
    missing_version.replace("Cargo.toml", "policy-version = 1\n", "")?;
    let missing_version_report = missing_version.report()?;
    assert!(missing_version_report.violations().iter().any(|violation| {
        violation.source() == "workspace metadata"
            && violation.target() == "policy-version"
            && violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("missing mandatory")
    }));

    let wrong_version = FixtureWorkspace::new("scalable-policy")?;
    wrong_version.replace("Cargo.toml", "policy-version = 1", "policy-version = 2")?;
    let wrong_version_report = wrong_version.report()?;
    assert!(wrong_version_report.violations().iter().any(|violation| {
        violation.source() == "workspace metadata"
            && violation.target() == "policy-version"
            && violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("integer 1")
    }));
    Ok(())
}

#[test]
fn explicit_role_at_an_incompatible_location_fails_without_path_inference()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "crates/domain/f0/Cargo.toml",
        "role = \"domain-foundation\"",
        "role = \"adapter\"",
    )?;
    let report = fixture.report()?;
    assert!(report.violations().iter().any(|violation| {
        violation.source() == "f0"
            && violation.rule() == "LAYOUT-1"
            && violation.reason().contains("never inferred")
    }));
    Ok(())
}

#[test]
fn portable_infrastructure_and_runtime_upward_edges_are_denied() -> Result<(), Box<dyn Error>> {
    let cases = [
        (
            "crates/domain/f0/Cargo.toml",
            "adapter = { path = \"../../adapters/adapter\" }",
            "f0",
            "adapter",
        ),
        (
            "crates/platform/platform/Cargo.toml",
            "e0 = { path = \"../../runtime/e0\" }",
            "platform",
            "e0",
        ),
        (
            "crates/adapters/adapter/Cargo.toml",
            "e0 = { path = \"../../runtime/e0\" }",
            "adapter",
            "e0",
        ),
        (
            "crates/runtime/e0/Cargo.toml",
            "e1 = { path = \"../e1\" }",
            "e0",
            "e1",
        ),
    ];

    for (manifest, dependency, source, target) in cases {
        let fixture = FixtureWorkspace::new("scalable-policy")?;
        let mut content = fixture.read(manifest)?;
        write!(content, "\n[dev-dependencies]\n{dependency}\n")?;
        fixture.write(manifest, &content)?;
        let report = fixture.report()?;
        assert!(report.violations().iter().any(|violation| {
            violation.source() == source
                && violation.target() == target
                && violation.rule() == "LAYER-DAG-1"
                && violation.dependency_kind() == Some(DependencyKind::Development)
        }));
    }
    Ok(())
}

#[test]
fn product_dependencies_on_benchmarks_and_tools_are_absolute_denials_for_all_kinds()
-> Result<(), Box<dyn Error>> {
    let kinds = [
        ("dependencies", DependencyKind::Normal),
        ("build-dependencies", DependencyKind::Build),
        ("dev-dependencies", DependencyKind::Development),
    ];
    let targets = [
        (
            "observer",
            "../../../benchmarks/observer",
            "BENCHMARK-OBSERVER-1",
        ),
        (
            "policy-tool",
            "../../../tools/policy-tool",
            "TOOLING-ISOLATION-1",
        ),
    ];

    for (section, expected_kind) in kinds {
        for (target, path, rule) in targets {
            let fixture = FixtureWorkspace::new("scalable-policy")?;
            fixture.write(
                "crates/domain/f1-b/Cargo.toml",
                &format!(
                    "[package]\nname = \"f1-b\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[package.metadata.milkdrift]\nrole = \"domain-feature\"\n\n[{section}]\n{target} = {{ path = \"{path}\" }}\n"
                ),
            )?;
            let report = fixture.report()?;
            assert!(report.violations().iter().any(|violation| {
                violation.source() == "f1-b"
                    && violation.target() == target
                    && violation.rule() == rule
                    && violation.dependency_kind() == Some(expected_kind)
            }));
        }
    }
    Ok(())
}

#[test]
fn actual_acyclic_domain_peer_edges_need_no_duplicate_registry() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    let report = fixture.report()?;
    assert!(
        report.is_valid(),
        "ordinary F1 -> F1 -> F0 Cargo graph was rejected: {:#?}",
        report.violations()
    );

    Ok(())
}

#[test]
fn same_layer_runtime_peers_are_not_universally_permitted() -> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.replace(
        "crates/runtime/capability/Cargo.toml",
        "role = \"runtime-capability\"",
        "role = \"runtime-foundation\"",
    )?;
    let report = fixture.report()?;
    assert!(report.violations().iter().any(|violation| {
        violation.source() == "capability"
            && violation.target() == "e0"
            && violation.rule() == "LAYER-DAG-1"
    }));
    Ok(())
}

#[test]
fn build_and_development_edges_follow_explicit_policy_distinctions() -> Result<(), Box<dyn Error>> {
    let legal_build = FixtureWorkspace::new("scalable-policy")?;
    legal_build.replace(
        "crates/runtime/e1/Cargo.toml",
        "[dependencies]",
        "[build-dependencies]",
    )?;
    let legal_report = legal_build.report()?;
    assert!(
        legal_report.is_valid(),
        "legal downward build edge failed: {:#?}",
        legal_report.violations()
    );

    let observer_build = FixtureWorkspace::new("scalable-policy")?;
    observer_build.replace(
        "benchmarks/observer/Cargo.toml",
        "[dependencies]",
        "[build-dependencies]",
    )?;
    let observer_report = observer_build.report()?;
    assert!(has_violation(
        &observer_report,
        "observer",
        "BENCHMARK-BUILD-1"
    ));

    let reviewed_development = FixtureWorkspace::new("scalable-policy")?;
    reviewed_development.replace(
        "crates/runtime/e1/Cargo.toml",
        "[dependencies]",
        "[dev-dependencies]",
    )?;
    let unreviewed_report = reviewed_development.report()?;
    assert!(has_violation(
        &unreviewed_report,
        "e1",
        "POLICY-EXCEPTION-1"
    ));
    reviewed_development.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"local-e1-capability-dev\"\nsource = \"e1\"\ntarget = \"capability\"\nscope = \"local\"\nkind = \"development\"\nrationale = \"the fixture proves exact local development-edge review\"\n",
    )?;
    let reviewed_report = reviewed_development.report()?;
    assert!(
        reviewed_report.is_valid(),
        "reviewed downward development edge failed: {:#?}",
        reviewed_report.violations()
    );
    Ok(())
}

#[test]
fn exception_registry_requires_exact_live_edges() -> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    let root = missing.read("Cargo.toml")?;
    let exception = "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-policy-tool-reviewed-ext\"\nsource = \"policy-tool\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"the fixture tooling dependency exercises exact external exception matching\"\n";
    missing.write("Cargo.toml", &root.replace(exception, ""))?;
    let missing_report = missing.report()?;
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "policy-tool"
            && violation.target() == "reviewed-ext"
            && violation.rule() == "EXTERNAL-DEPENDENCY-1"
    }));

    let wrong_kind = FixtureWorkspace::new("scalable-policy")?;
    wrong_kind.replace("Cargo.toml", "kind = \"normal\"", "kind = \"development\"")?;
    let wrong_kind_report = wrong_kind.report()?;
    assert!(wrong_kind_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1" && violation.reason().contains("wrong-kind")
    }));

    let stale = FixtureWorkspace::new("scalable-policy")?;
    stale.replace(
        "Cargo.toml",
        "target = \"reviewed-ext\"",
        "target = \"stale-ext\"",
    )?;
    let stale_report = stale.report()?;
    assert!(stale_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1" && violation.reason().contains("stale exception")
    }));
    Ok(())
}

#[test]
fn exception_registry_rejects_duplicates_missing_packages_and_empty_rationales()
-> Result<(), Box<dyn Error>> {
    let duplicate = FixtureWorkspace::new("scalable-policy")?;
    duplicate.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-policy-tool-reviewed-ext\"\nsource = \"policy-tool\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"duplicate\"\n",
    )?;
    let duplicate_report = duplicate.report()?;
    assert!(duplicate_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && (violation.reason().contains("globally unique")
                || violation.reason().contains("duplicate exception"))
    }));

    let missing_package = FixtureWorkspace::new("scalable-policy")?;
    missing_package.replace(
        "Cargo.toml",
        "source = \"policy-tool\"",
        "source = \"missing-tool\"",
    )?;
    let missing_package_report = missing_package.report()?;
    assert!(missing_package_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("not a workspace member")
    }));

    let empty = FixtureWorkspace::new("scalable-policy")?;
    empty.replace(
        "Cargo.toml",
        "rationale = \"the fixture tooling dependency exercises exact external exception matching\"",
        "rationale = \"   \"",
    )?;
    let empty_report = empty.report()?;
    assert!(empty_report.violations().iter().any(|violation| {
        violation.rule() == "POLICY-EXCEPTION-1"
            && violation.reason().contains("nonempty rationale")
    }));
    Ok(())
}

#[test]
fn unnecessary_exceptions_and_attempted_absolute_overrides_fail() -> Result<(), Box<dyn Error>> {
    let unnecessary = FixtureWorkspace::new("scalable-policy")?;
    unnecessary.replace(
        "crates/adapters/adapter/Cargo.toml",
        "platform = { path = \"../../platform/platform\" }",
        "platform = { path = \"../../platform/platform\" }\nreviewed-ext = \"=0.1.0\"",
    )?;
    unnecessary.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"external-adapter-reviewed-ext\"\nsource = \"adapter\"\ntarget = \"reviewed-ext\"\nscope = \"external\"\nkind = \"normal\"\nrationale = \"must be rejected as redundant\"\n",
    )?;
    let unnecessary_report = unnecessary.report()?;
    assert!(unnecessary_report.violations().iter().any(|violation| {
        violation.source() == "external-adapter-reviewed-ext"
            && violation.reason().contains("unnecessary exception")
    }));

    let absolute = FixtureWorkspace::new("scalable-policy")?;
    absolute.replace(
        "crates/domain/f1-b/Cargo.toml",
        "f1-a = { path = \"../f1-a\" }",
        "f1-a = { path = \"../f1-a\" }\nadapter = { path = \"../../adapters/adapter\" }",
    )?;
    absolute.append_root(
        "\n[[workspace.metadata.milkdrift.exceptions]]\nid = \"local-f1-b-adapter\"\nsource = \"f1-b\"\ntarget = \"adapter\"\nscope = \"local\"\nkind = \"normal\"\nrationale = \"attempted upward override\"\n",
    )?;
    let absolute_report = absolute.report()?;
    assert!(absolute_report.violations().iter().any(|violation| {
        violation.source() == "local-f1-b-adapter"
            && violation.reason().contains("cannot override absolute")
    }));
    assert!(has_violation(&absolute_report, "f1-b", "LAYER-DAG-1"));
    Ok(())
}

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

#[test]
fn benchmark_registry_is_bidirectional_and_rejects_duplicates_and_non_benches()
-> Result<(), Box<dyn Error>> {
    let missing = FixtureWorkspace::new("scalable-policy")?;
    missing.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"missing\"]",
    )?;
    let missing_report = missing.report()?;
    assert!(missing_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("does not exist")
    }));

    let unregistered = FixtureWorkspace::new("scalable-policy")?;
    unregistered.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]\n",
        "",
    )?;
    let unregistered_report = unregistered.report()?;
    assert!(unregistered_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("unregistered")
    }));

    let non_bench = FixtureWorkspace::new("scalable-policy")?;
    non_bench.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"sampling_pipeline\", \"ordinary_target\"]",
    )?;
    let non_bench_report = non_bench.report()?;
    assert!(non_bench_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.target() == "ordinary_target"
            && violation.reason().contains("not a Cargo bench target")
    }));

    let duplicate = FixtureWorkspace::new("scalable-policy")?;
    duplicate.replace(
        "crates/domain/f1-a/Cargo.toml",
        "benchmark-targets = [\"sampling_pipeline\"]",
        "benchmark-targets = [\"sampling_pipeline\", \"sampling_pipeline\"]",
    )?;
    let duplicate_report = duplicate.report()?;
    assert!(duplicate_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("unique")
    }));

    let harnessed = FixtureWorkspace::new("scalable-policy")?;
    harnessed.replace(
        "crates/domain/f1-a/Cargo.toml",
        "harness = false",
        "harness = true",
    )?;
    let harnessed_report = harnessed.report()?;
    assert!(harnessed_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("harness = false")
    }));

    let implicit = FixtureWorkspace::new("scalable-policy")?;
    implicit.replace(
        "crates/domain/f1-a/Cargo.toml",
        "\n[[bench]]\nname = \"sampling_pipeline\"\nharness = false\n",
        "",
    )?;
    let implicit_report = implicit.report()?;
    assert!(implicit_report.violations().iter().any(|violation| {
        violation.source() == "f1-a"
            && violation.rule() == "BENCHMARK-REGISTRY-1"
            && violation.reason().contains("explicit [[bench]]")
    }));
    Ok(())
}

#[test]
fn generated_benchmark_commands_are_sorted_exact_and_never_workspace_wide()
-> Result<(), Box<dyn Error>> {
    let fixture = FixtureWorkspace::new("scalable-policy")?;
    fixture.refresh_lock()?;
    let commands = benchmark_command_plan(&fixture.manifest())?;
    let arguments = commands
        .iter()
        .map(|command| command.arguments().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        vec![
            vec![
                "bench",
                "--locked",
                "-p",
                "f1-a",
                "--bench",
                "sampling_pipeline",
                "--no-run",
            ],
            vec![
                "bench", "--locked", "-p", "observer", "--bench", "runtime", "--no-run",
            ],
        ]
    );
    assert!(
        arguments
            .iter()
            .flatten()
            .all(|argument| argument != "--workspace")
    );

    let actual = benchmark_command_plan(&workspace_manifest())?;
    let actual_pairs = actual
        .iter()
        .map(|command| {
            let arguments = command.arguments();
            (arguments.get(3).cloned(), arguments.get(5).cloned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual_pairs,
        vec![
            (
                Some("runtime-benchmarks".to_owned()),
                Some("runtime".to_owned())
            ),
            (
                Some("sampling".to_owned()),
                Some("sampling_pipeline".to_owned())
            ),
        ]
    );
    Ok(())
}
