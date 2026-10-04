//! Byte-pinned process fixtures shared by local and peer daemon scenarios.

use milkdrift_local_process::PlatformSupport;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

pub(crate) fn executable() -> TestResult<PathBuf> {
    let current = std::env::current_exe()?;
    let profile = current
        .parent()
        .and_then(Path::parent)
        .ok_or("Cargo test profile absent")?;
    let path = profile.join(format!(
        "milkdrift-process-test-helper{}",
        std::env::consts::EXE_SUFFIX
    ));
    if !path.is_file() {
        return Err("build milkdrift-process-test-helper before isolated daemon tests".into());
    }
    Ok(path.canonicalize()?)
}

pub(crate) fn configured_process_profile(directory: &tempfile::TempDir) -> TestResult<PathBuf> {
    let executable = executable()?;
    let bytes = fs::read(&executable)?;
    let mut profile: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../adapters/local-process/tests/fixtures/process-profile-v2.json"
    ))?;
    *profile
        .pointer_mut("/profile/executable")
        .ok_or("fixture field /profile/executable absent")? = json!(executable);
    *profile
        .pointer_mut("/profile/arguments")
        .ok_or("fixture field /profile/arguments absent")? = json!(["echo", "golden"]);
    // These tests exercise daemon composition, not the golden fixture's one-second
    // deadline. Unoptimized durable reporting can exceed that deadline on a busy runner.
    *profile
        .pointer_mut("/profile/limits/wall_timeout_ms")
        .ok_or("fixture field /profile/limits/wall_timeout_ms absent")? = json!(10000);
    *profile
        .pointer_mut("/profile/filesystem_roots")
        .ok_or("fixture field /profile/filesystem_roots absent")? = json!([
        {"path": executable.parent().ok_or("fixture executable parent absent")?, "access":"execute"},
        {"path": std::env::temp_dir().canonicalize()?, "access":"read_write"}
    ]);
    *profile
        .pointer_mut("/profile/platform")
        .ok_or("fixture field /profile/platform absent")? = json!(PlatformSupport::current());
    *profile
        .pointer_mut("/profile/implementation/content_digest")
        .ok_or("fixture field /profile/implementation/content_digest absent")? =
        json!(format!("b3_{}", blake3::hash(&bytes)));
    *profile
        .pointer_mut("/profile/implementation/size_bytes")
        .ok_or("fixture field /profile/implementation/size_bytes absent")? = json!(bytes.len());
    let path = directory.path().join("process-profile-v2.json");
    fs::write(&path, serde_json::to_vec(&profile)?)?;
    Ok(path)
}
