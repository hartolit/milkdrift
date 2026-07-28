//! Integration coverage for locked, typed workspace architecture validation.

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use llm_app::validate_workspace;

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
            "llm-app-architecture-{name}-{}-{id}",
            std::process::id()
        ));
        copy_fixture(&source, &root)?;
        Ok(Self { root })
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
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

#[test]
fn actual_workspace_satisfies_architecture_policy() -> Result<(), Box<dyn Error>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
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
        Some(llm_app::DependencyKind::Normal)
    );
    assert!(violation.reason().contains("8-layer production direction"));
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
    assert!(violation.reason().contains("unknown locations"));
    Ok(())
}
