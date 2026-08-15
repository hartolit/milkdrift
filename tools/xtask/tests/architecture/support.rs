//! Integration coverage for metadata-driven workspace architecture validation.

pub(crate) use std::error::Error;
pub(crate) use std::ffi::OsString;
pub(crate) use std::fmt::Write as _;
pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::Command;
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) use xtask::{
    DependencyKind, ValidationReport, benchmark_command_plan, validate_workspace,
};

pub(crate) static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) struct FixtureWorkspace {
    pub(crate) root: PathBuf,
}

impl FixtureWorkspace {
    pub(crate) fn new(name: &str) -> Result<Self, Box<dyn Error>> {
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

    pub(crate) fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    pub(crate) fn read(&self, relative: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(self.root.join(relative))?)
    }

    pub(crate) fn write(&self, relative: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    pub(crate) fn replace(
        &self,
        relative: &str,
        old: &str,
        new: &str,
    ) -> Result<(), Box<dyn Error>> {
        let content = self.read(relative)?;
        if !content.contains(old) {
            return Err(
                format!("fixture {relative} did not contain replacement text `{old}`").into(),
            );
        }
        self.write(relative, &content.replacen(old, new, 1))
    }

    pub(crate) fn append_root(&self, content: &str) -> Result<(), Box<dyn Error>> {
        let mut root = self.read("Cargo.toml")?;
        root.push_str(content);
        self.write("Cargo.toml", &root)
    }

    pub(crate) fn report(&self) -> Result<ValidationReport, Box<dyn Error>> {
        self.refresh_lock()?;
        Ok(validate_workspace(&self.manifest())?)
    }

    pub(crate) fn refresh_lock(&self) -> Result<(), Box<dyn Error>> {
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

pub(crate) fn copy_fixture(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
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

pub(crate) fn workspace_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Cargo.toml")
}

pub(crate) fn has_violation(report: &ValidationReport, source: &str, rule: &str) -> bool {
    report
        .violations()
        .iter()
        .any(|violation| violation.source() == source && violation.rule() == rule)
}
